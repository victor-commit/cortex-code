//! Per-agent rulesets and subagent narrowing.
//!
//! Each agent carries its own independent permission ruleset that governs the active turn
//! (architecture §3.4). [`AgentRulesets`] holds a shared **base** ruleset that applies to every
//! agent plus per-agent rule overrides; [`AgentRulesets::for_agent`] composes the effective ruleset
//! for a named agent by appending its overrides after the base, so under last-matching-rule-wins an
//! agent's rule overrides a conflicting base rule. Two agents whose rules differ therefore decide
//! the same action differently (VAL-PERM-009).
//!
//! When an agent spawns a subagent, the subagent's effective permission is **narrowed** to at most
//! the parent's: [`narrow_action`] (and the engine's parent cap, [`crate::PermissionEngine`]) take
//! the most restrictive of the child's decision and the parent's, so the child can never perform an
//! action the parent is denied (VAL-PERM-010).

use std::collections::BTreeMap;

use crate::Ruleset;
use crate::action::Action;

/// The per-agent permission rulesets: a shared base plus per-agent overrides.
///
/// The base rules apply to every agent; an agent's own rules are layered on top (appended after the
/// base) so they win under last-matching-rule-wins. An agent with no overrides is governed by the
/// base alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentRulesets {
    base: Ruleset,
    per_agent: BTreeMap<String, Ruleset>,
}

impl AgentRulesets {
    /// Build from a base ruleset (applied to all agents) and per-agent override rulesets.
    #[must_use]
    pub fn new(base: Ruleset, per_agent: BTreeMap<String, Ruleset>) -> Self {
        Self { base, per_agent }
    }

    /// The empty configuration: no base rules and no per-agent overrides (every action defaults to
    /// ask for every agent).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The shared base ruleset applied to every agent.
    #[must_use]
    pub fn base(&self) -> &Ruleset {
        &self.base
    }

    /// Whether `name` has its own override ruleset.
    #[must_use]
    pub fn has_agent(&self, name: &str) -> bool {
        self.per_agent.contains_key(name)
    }

    /// The names of every agent that carries an override ruleset, in sorted order.
    #[must_use]
    pub fn agent_names(&self) -> Vec<&str> {
        self.per_agent.keys().map(String::as_str).collect()
    }

    /// The effective ruleset governing `name`'s turn: the base rules followed by the agent's own
    /// overrides (so the agent's rules win under last-matching-rule-wins). An unknown agent name is
    /// governed by the base ruleset alone.
    #[must_use]
    pub fn for_agent(&self, name: &str) -> Ruleset {
        let mut rules = self.base.rules().to_vec();
        if let Some(agent) = self.per_agent.get(name) {
            rules.extend(agent.rules().iter().cloned());
        }
        Ruleset::new(rules)
    }
}

/// The effective action for a subagent, narrowed to at most its parent's.
///
/// Returns the most restrictive of the child's decision and the parent's, so the result is never
/// more permissive than either: a subagent can restrict itself further than its parent but can
/// never exceed it (VAL-PERM-010). If the parent denies an action, the subagent's attempt is denied
/// regardless of the child ruleset.
#[must_use]
pub fn narrow_action(parent: &Ruleset, child: &Ruleset, tool: &str, subject: &str) -> Action {
    parent
        .classify(tool, subject)
        .most_restrictive(child.classify(tool, subject))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Rule;

    fn ruleset(rules: Vec<Rule>) -> Ruleset {
        Ruleset::new(rules)
    }

    #[test]
    fn for_agent_layers_overrides_after_the_base() {
        let base = ruleset(vec![Rule::new("write", "**", Action::Ask)]);
        let mut per_agent = BTreeMap::new();
        per_agent.insert(
            "plan".to_owned(),
            ruleset(vec![Rule::new("write", "**", Action::Deny)]),
        );
        per_agent.insert(
            "build".to_owned(),
            ruleset(vec![Rule::new("write", "**", Action::Allow)]),
        );
        let rulesets = AgentRulesets::new(base, per_agent);

        // The same write is decided differently per agent (VAL-PERM-009).
        assert_eq!(
            rulesets.for_agent("plan").classify("write", "src/a.txt"),
            Action::Deny
        );
        assert_eq!(
            rulesets.for_agent("build").classify("write", "src/a.txt"),
            Action::Allow
        );
        // An agent with no overrides falls back to the base.
        assert_eq!(
            rulesets.for_agent("general").classify("write", "src/a.txt"),
            Action::Ask
        );
    }

    #[test]
    fn agent_names_are_sorted_and_membership_is_observable() {
        let mut per_agent = BTreeMap::new();
        per_agent.insert("plan".to_owned(), Ruleset::empty());
        per_agent.insert("build".to_owned(), Ruleset::empty());
        let rulesets = AgentRulesets::new(Ruleset::empty(), per_agent);
        assert_eq!(rulesets.agent_names(), vec!["build", "plan"]);
        assert!(rulesets.has_agent("plan"));
        assert!(!rulesets.has_agent("explore"));
    }

    #[test]
    fn narrow_action_caps_the_child_to_the_parent() {
        // Parent allows generally but denies `rm *`; the child allows everything. The subagent is
        // still denied rm (it cannot exceed the parent), but may run what the parent allows.
        let parent = ruleset(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]);
        let child = ruleset(vec![Rule::new("bash", "*", Action::Allow)]);
        assert_eq!(
            narrow_action(&parent, &child, "bash", "rm victim.txt"),
            Action::Deny
        );
        // A command the parent allows is governed by the child's allow.
        assert_eq!(
            narrow_action(&parent, &child, "bash", "echo hi"),
            Action::Allow
        );
    }

    #[test]
    fn narrow_action_defaults_to_the_more_restrictive_of_an_undecided_parent() {
        // A parent that only denies `rm *` has no opinion on `echo` (default ask), so a child that
        // would allow `echo` is capped to ask: the subagent can never exceed the parent.
        let parent = ruleset(vec![Rule::new("bash", "rm *", Action::Deny)]);
        let child = ruleset(vec![Rule::new("bash", "*", Action::Allow)]);
        assert_eq!(
            narrow_action(&parent, &child, "bash", "rm victim.txt"),
            Action::Deny
        );
        assert_eq!(
            narrow_action(&parent, &child, "bash", "echo hi"),
            Action::Ask
        );
    }

    #[test]
    fn narrow_action_lets_the_child_restrict_itself_further() {
        // Parent allows everything; the child denies secrets. The subagent is more restrictive.
        let parent = ruleset(vec![Rule::new("write", "**", Action::Allow)]);
        let child = ruleset(vec![
            Rule::new("write", "**", Action::Allow),
            Rule::new("write", "secrets/**", Action::Deny),
        ]);
        assert_eq!(
            narrow_action(&parent, &child, "write", "secrets/k.txt"),
            Action::Deny
        );
        assert_eq!(
            narrow_action(&parent, &child, "write", "src/k.txt"),
            Action::Allow
        );
    }
}
