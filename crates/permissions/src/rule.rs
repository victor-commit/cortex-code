//! A single permission rule: a tool selector, a subject pattern, and the action to take.

use crate::action::Action;
use crate::glob::glob_match;
use crate::scope::ToolScope;

/// The tool selector that matches any gated tool.
pub const ANY_TOOL: &str = "*";

/// One permission rule.
///
/// A rule matches a gated action when its [`tool`](Rule::tool) selector matches the action's tool
/// and its [`pattern`](Rule::pattern) matches the action's subject. Rules are evaluated in order
/// inside a [`Ruleset`](crate::Ruleset) with last-matching-rule-wins precedence (architecture
/// §3.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The tool this rule applies to (e.g. `bash`, `write`), or [`ANY_TOOL`] (`*`) for any tool.
    pub tool: String,
    /// The glob pattern matched against the action's subject (see [`glob_match`]).
    pub pattern: String,
    /// The action taken when this rule matches.
    pub action: Action,
}

impl Rule {
    /// Build a rule from a tool selector, a subject pattern, and an action.
    #[must_use]
    pub fn new(tool: impl Into<String>, pattern: impl Into<String>, action: Action) -> Self {
        Self {
            tool: tool.into(),
            pattern: pattern.into(),
            action,
        }
    }

    /// Whether this rule's tool selector matches `tool` (an exact match, or the `*` wildcard).
    #[must_use]
    pub fn matches_tool(&self, tool: &str) -> bool {
        if self.tool == ANY_TOOL {
            return true;
        }
        let rule_tool = crate::scope::canonical_tool_name(&self.tool);
        let request_tool = crate::scope::canonical_tool_name(tool);
        if rule_tool == request_tool {
            return true;
        }
        crate::scope::capability_for_tool(request_tool).is_some_and(|cap| cap == rule_tool)
    }

    /// Whether this rule matches a gated action against `tool` with subject `subject`.
    #[must_use]
    pub fn matches(&self, tool: &str, subject: &str) -> bool {
        if !self.matches_tool(tool) {
            return false;
        }
        if matches!(crate::scope::scope_of(tool), Some(ToolScope::Path)) {
            let pattern = crate::scope::normalize_path(&self.pattern);
            let subject = crate::scope::normalize_path(subject);
            return glob_match(&pattern, &subject);
        }
        if matches!(crate::scope::scope_of(tool), Some(ToolScope::Command)) {
            let patterns = crate::scope::command_match_candidates(&self.pattern);
            let subjects = crate::scope::command_match_candidates(subject);
            return patterns
                .iter()
                .any(|pattern| subjects.iter().any(|subject| glob_match(pattern, subject)));
        }
        glob_match(&self.pattern, subject)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_tool_matches_any_tool() {
        let rule = Rule::new(ANY_TOOL, "*", Action::Allow);
        assert!(rule.matches("bash", "rm x"));
        assert!(rule.matches("write", "src/a.txt"));
    }

    #[test]
    fn named_tool_only_matches_that_tool() {
        let rule = Rule::new("bash", "rm *", Action::Deny);
        assert!(rule.matches("bash", "rm x"));
        assert!(!rule.matches("write", "rm x"));
    }

    #[test]
    fn pattern_must_match_the_subject() {
        let rule = Rule::new("bash", "git push*", Action::Deny);
        assert!(rule.matches("bash", "git push origin"));
        assert!(!rule.matches("bash", "git status"));
    }

    #[test]
    fn path_rules_expand_home_aliases_before_matching() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = std::path::PathBuf::from(home)
            .to_string_lossy()
            .replace('\\', "/");
        let rule = Rule::new("read", "$HOME/private/**", Action::Deny);
        assert!(rule.matches("read", &format!("{home}/private/secret.txt")));
        assert!(rule.matches("read", "~/private/secret.txt"));
    }

    #[test]
    fn capability_tool_selector_matches_equivalent_mutators() {
        let rule = Rule::new("edit", "src/**", Action::Deny);
        assert!(rule.matches("write", "src/main.rs"));
        assert!(rule.matches("edit", "src/main.rs"));
        assert!(rule.matches("apply_patch", "src/main.rs"));
        assert!(rule.matches("format", "src/main.rs"));
        assert!(!rule.matches("read", "src/main.rs"));
    }

    #[test]
    fn command_rules_match_arity_normalized_families() {
        let rule = Rule::new("bash", "git checkout", Action::Deny);
        assert!(rule.matches("bash", "git checkout feature/branch"));
        assert!(rule.matches("bash", "sudo git checkout main"));
        assert!(!rule.matches("bash", "git status"));
    }

    #[test]
    fn command_rules_expand_home_aliases_before_matching() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = std::path::PathBuf::from(home)
            .to_string_lossy()
            .replace('\\', "/");
        let rule = Rule::new("bash", "cat $HOME/private/**", Action::Deny);
        assert!(rule.matches("bash", &format!("cat {home}/private/secret.txt")));
        assert!(rule.matches("bash", "cat ~/private/secret.txt"));
    }
}
