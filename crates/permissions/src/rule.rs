//! A single permission rule: a tool selector, a subject pattern, and the action to take.

use crate::action::Action;
use crate::glob::glob_match;

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
        self.tool == ANY_TOOL || self.tool == tool
    }

    /// Whether this rule matches a gated action against `tool` with subject `subject`.
    #[must_use]
    pub fn matches(&self, tool: &str, subject: &str) -> bool {
        self.matches_tool(tool) && glob_match(&self.pattern, subject)
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
}
