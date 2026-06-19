//! The fail-closed outbound network guard for the `CORTEX_NO_NET` kill-switch.
//!
//! Every network-touching path must call [`guard_outbound`] immediately before it would open an
//! outbound socket. When the kill-switch is engaged the guard returns the canonical
//! [`Diagnostic::NetDisabled`](errors::Diagnostic::NetDisabled) and the caller must NOT create a
//! socket. This single chokepoint is what makes the switch fail-closed: it is consulted before any
//! address resolution, credential read, or `connect()`, so a refusal never touches the network.

use std::ffi::OsStr;

use errors::Diagnostic;

/// The environment variable that hard-disables outbound network access when set to the
/// documented truthy value.
pub const NO_NET_ENV: &str = "CORTEX_NO_NET";

/// The single documented value that engages the kill-switch.
const ENABLED_VALUE: &str = "1";

/// Whether the `CORTEX_NO_NET` kill-switch is currently engaged.
///
/// Only the exact documented value (`1`) engages it; unset, empty, `0`, `true`, or any other
/// value leaves outbound network enabled. The decision is read from the process environment on
/// each call, so toggling the variable between runs is deterministic and consistent.
#[must_use]
pub fn network_disabled() -> bool {
    is_engaged(std::env::var_os(NO_NET_ENV).as_deref())
}

/// The pure kill-switch decision for a raw environment value (the testable core of
/// [`network_disabled`]). Kept separate so the on/off rule can be exhaustively unit-tested without
/// mutating the process environment.
#[must_use]
fn is_engaged(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new(ENABLED_VALUE))
}

/// Fail-closed outbound guard: call immediately before opening any outbound socket.
///
/// Returns [`Diagnostic::net_disabled`](errors::Diagnostic::net_disabled) (naming `operation`)
/// when the kill-switch is engaged, in which case the caller MUST abort without creating a socket.
/// Returns `Ok(())` when outbound network is permitted.
pub fn guard_outbound(operation: &str) -> Result<(), Diagnostic> {
    decide(network_disabled(), operation)
}

/// The pure guard decision, separated from the environment read so both branches are unit-testable.
fn decide(disabled: bool, operation: &str) -> Result<(), Diagnostic> {
    if disabled {
        Err(Diagnostic::net_disabled(operation))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_documented_value_engages_the_switch() {
        assert!(is_engaged(Some(OsStr::new("1"))), "`1` engages the switch");
    }

    #[test]
    fn unset_leaves_network_enabled() {
        assert!(!is_engaged(None), "an unset variable never engages");
    }

    #[test]
    fn other_values_do_not_engage_the_switch() {
        for value in ["", "0", "2", "11", " 1", "1 ", "true", "yes", "on", "TRUE"] {
            assert!(
                !is_engaged(Some(OsStr::new(value))),
                "value {value:?} must not engage the kill-switch"
            );
        }
    }

    #[test]
    fn guard_refuses_when_disabled_with_a_net_disabled_diagnostic() {
        let err = decide(true, "outbound connectivity check").expect_err("must refuse");
        assert_eq!(err.kind(), "net-disabled");
        assert_eq!(err, Diagnostic::net_disabled("outbound connectivity check"));
    }

    #[test]
    fn guard_permits_when_enabled() {
        assert!(decide(false, "outbound connectivity check").is_ok());
    }
}
