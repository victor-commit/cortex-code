//! The three permission actions and how they parse from config.

/// One of exactly three permission actions a rule can declare for a gated tool action.
///
/// The engine resolves every gated action to one of these. [`Action::Ask`] is also the built-in
/// default when no rule matches: an ungoverned action is never silently allowed or denied
/// (architecture §3.4; VAL-PERM-001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The action runs immediately with no prompt.
    Allow,
    /// The action is suspended pending a decision; an observable ask is surfaced.
    Ask,
    /// The action is blocked entirely; the effect never occurs.
    Deny,
}

impl Action {
    /// The stable lowercase name of this action, as written in config.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }

    /// Parse an action from its config string, case-insensitively.
    ///
    /// Returns `None` for any value other than `allow`/`ask`/`deny` so the loader can surface a
    /// clear diagnostic rather than silently defaulting.
    #[must_use]
    pub fn parse(value: &str) -> Option<Action> {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" => Some(Action::Allow),
            "ask" => Some(Action::Ask),
            "deny" => Some(Action::Deny),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_the_three_actions_case_insensitively() {
        assert_eq!(Action::parse("allow"), Some(Action::Allow));
        assert_eq!(Action::parse("ASK"), Some(Action::Ask));
        assert_eq!(Action::parse("  Deny "), Some(Action::Deny));
    }

    #[test]
    fn parse_rejects_unknown_actions() {
        assert_eq!(Action::parse("permit"), None);
        assert_eq!(Action::parse(""), None);
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for action in [Action::Allow, Action::Ask, Action::Deny] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
    }
}
