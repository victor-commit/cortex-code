//! An ordered set of permission rules evaluated last-matching-rule-wins.

use crate::action::Action;
use crate::rule::Rule;

/// An ordered list of [`Rule`]s evaluated with **last-matching-rule-wins** precedence.
///
/// Evaluation walks the rules in order and the LAST rule that matches a gated action decides the
/// outcome (architecture §3.4; VAL-PERM-008). When no rule matches, the action defaults to
/// [`Action::Ask`] (never a silent allow/deny; VAL-PERM-001).
///
/// Rules from multiple config layers are combined into one ruleset with [`Ruleset::from_layers`],
/// which concatenates the per-layer rule lists in ascending precedence order (global, project,
/// nested). Because the last matching rule wins, a later layer's rule overrides a conflicting
/// earlier layer's rule deterministically (VAL-PERM-017).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ruleset {
    rules: Vec<Rule>,
}

impl Ruleset {
    /// Build a ruleset from an ordered list of rules (first declared first, last declared last).
    #[must_use]
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// The empty ruleset: every action defaults to [`Action::Ask`].
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Combine layered rulesets into one, in ascending precedence order (lowest layer first).
    ///
    /// The rule lists are concatenated in iteration order, so rules from a later (higher
    /// precedence) layer appear after those from an earlier layer. Under last-matching-rule-wins
    /// this gives the later layer's rules precedence over conflicting earlier ones, matching the
    /// settings layering order (defaults < global < project < nested).
    #[must_use]
    pub fn from_layers(layers: impl IntoIterator<Item = Ruleset>) -> Ruleset {
        let mut rules = Vec::new();
        for layer in layers {
            rules.extend(layer.rules);
        }
        Ruleset::new(rules)
    }

    /// The rules in evaluation order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Whether the ruleset has no rules (so every action defaults to [`Action::Ask`]).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Append a rule to the end (so it takes precedence over earlier rules it conflicts with).
    pub fn push(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// Resolve a gated action to its action plus the rule that decided it.
    ///
    /// Returns the LAST rule that matches `tool`/`subject` and its action; when no rule matches,
    /// returns ([`Action::Ask`], `None`) — the default-ask case.
    #[must_use]
    pub fn resolve(&self, tool: &str, subject: &str) -> (Action, Option<&Rule>) {
        let mut decided: (Action, Option<&Rule>) = (Action::Ask, None);
        for rule in &self.rules {
            if rule.matches(tool, subject) {
                decided = (rule.action, Some(rule));
            }
        }
        decided
    }

    /// Resolve a gated action to its action, defaulting to [`Action::Ask`] when no rule matches.
    #[must_use]
    pub fn classify(&self, tool: &str, subject: &str) -> Action {
        self.resolve(tool, subject).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::ANY_TOOL;

    #[test]
    fn empty_ruleset_defaults_to_ask() {
        let ruleset = Ruleset::empty();
        assert_eq!(ruleset.classify("bash", "echo hi"), Action::Ask);
        assert!(ruleset.resolve("bash", "echo hi").1.is_none());
    }

    #[test]
    fn a_single_matching_rule_decides() {
        let ruleset = Ruleset::new(vec![Rule::new("bash", "echo *", Action::Allow)]);
        assert_eq!(ruleset.classify("bash", "echo hi"), Action::Allow);
        // A non-matching subject still defaults to ask.
        assert_eq!(ruleset.classify("bash", "rm x"), Action::Ask);
    }

    #[test]
    fn last_matching_rule_wins() {
        // allow-then-deny: the deny matches last and governs `rm x`.
        let allow_then_deny = Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]);
        assert_eq!(allow_then_deny.classify("bash", "rm x"), Action::Deny);
        // A bash command not matching the deny is governed by the broad allow.
        assert_eq!(allow_then_deny.classify("bash", "echo hi"), Action::Allow);

        // deny-then-allow: reordering flips `rm x` to allow (the broad allow now matches last).
        let deny_then_allow = Ruleset::new(vec![
            Rule::new("bash", "rm *", Action::Deny),
            Rule::new("bash", "*", Action::Allow),
        ]);
        assert_eq!(deny_then_allow.classify("bash", "rm x"), Action::Allow);
    }

    #[test]
    fn resolve_returns_the_deciding_rule() {
        let ruleset = Ruleset::new(vec![
            Rule::new(ANY_TOOL, "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]);
        let (action, rule) = ruleset.resolve("bash", "rm target.txt");
        assert_eq!(action, Action::Deny);
        assert_eq!(rule.map(|r| r.pattern.as_str()), Some("rm *"));
    }

    #[test]
    fn from_layers_concatenates_in_precedence_order() {
        let global = Ruleset::new(vec![Rule::new("bash", "rm *", Action::Allow)]);
        let project = Ruleset::new(vec![Rule::new("bash", "rm *", Action::Deny)]);
        // global first, then project: project's rule matches last and wins.
        let combined = Ruleset::from_layers([global.clone(), project.clone()]);
        assert_eq!(combined.classify("bash", "rm x"), Action::Deny);

        // Removing the project layer restores the global outcome.
        let global_only = Ruleset::from_layers([global]);
        assert_eq!(global_only.classify("bash", "rm x"), Action::Allow);
    }
}
