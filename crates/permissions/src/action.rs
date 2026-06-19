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

    /// How restrictive this action is, on a total order `Allow < Ask < Deny`.
    ///
    /// Used to combine two actions into the more restrictive of the two, which is the foundation of
    /// subagent narrowing: a spawned subagent's effective permission is the most restrictive of its
    /// own decision and its parent's, so it can never exceed the parent (VAL-PERM-010).
    const fn restrictiveness(self) -> u8 {
        match self {
            Action::Allow => 0,
            Action::Ask => 1,
            Action::Deny => 2,
        }
    }

    /// The more restrictive of `self` and `other` (`Allow < Ask < Deny`).
    ///
    /// Because the result is never more permissive than either input, capping a child action with a
    /// parent's via this combinator guarantees the child can never exceed the parent
    /// (VAL-PERM-010). Ties keep `self`.
    #[must_use]
    pub const fn most_restrictive(self, other: Action) -> Action {
        if other.restrictiveness() > self.restrictiveness() {
            other
        } else {
            self
        }
    }

    /// Whether `self` is strictly more restrictive than `other` (`Allow < Ask < Deny`).
    #[must_use]
    pub const fn is_more_restrictive_than(self, other: Action) -> bool {
        self.restrictiveness() > other.restrictiveness()
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

    #[test]
    fn most_restrictive_orders_allow_ask_deny() {
        // Deny dominates everything; ask dominates allow; allow only wins against itself.
        assert_eq!(Action::Allow.most_restrictive(Action::Deny), Action::Deny);
        assert_eq!(Action::Deny.most_restrictive(Action::Allow), Action::Deny);
        assert_eq!(Action::Allow.most_restrictive(Action::Ask), Action::Ask);
        assert_eq!(Action::Ask.most_restrictive(Action::Allow), Action::Ask);
        assert_eq!(Action::Allow.most_restrictive(Action::Allow), Action::Allow);
        assert_eq!(Action::Deny.most_restrictive(Action::Deny), Action::Deny);
    }

    #[test]
    fn most_restrictive_never_exceeds_either_input() {
        // The result is never more permissive than either operand — the subagent-cap invariant:
        // the capped action is at least as restrictive as both the parent's and the child's.
        for parent in [Action::Allow, Action::Ask, Action::Deny] {
            for child in [Action::Allow, Action::Ask, Action::Deny] {
                let capped = parent.most_restrictive(child);
                assert!(
                    capped.restrictiveness() >= parent.restrictiveness()
                        && capped.restrictiveness() >= child.restrictiveness(),
                    "{capped:?} must be at least as restrictive as both {parent:?} and {child:?}"
                );
            }
        }
    }

    #[test]
    fn is_more_restrictive_than_is_strict() {
        assert!(Action::Deny.is_more_restrictive_than(Action::Ask));
        assert!(Action::Ask.is_more_restrictive_than(Action::Allow));
        assert!(!Action::Allow.is_more_restrictive_than(Action::Allow));
        assert!(!Action::Ask.is_more_restrictive_than(Action::Deny));
    }
}
