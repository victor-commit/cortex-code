use core::fmt;

use crate::Exit;

/// A canonical, typed failure class shared across Cortex Code surfaces.
///
/// Each variant is a distinct, stable class so machine consumers and tests can tell failures
/// apart three ways: by [`kind`](Diagnostic::kind) (a stable identifier), by the process
/// [`exit`](Diagnostic::exit) code, and by the rendered message. The classes mirror the
/// foundational failure modes named in the architecture — the network kill-switch refusing an
/// outbound operation, an absent credential, an unreachable transport, and a missing test
/// cassette — so a `net-disabled` refusal is never confused with a missing key or a connection
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    /// The `CORTEX_NO_NET` kill-switch refused a network-requiring operation before any socket
    /// was opened (fail-closed). The payload names the operation that was refused.
    NetDisabled(String),
    /// A credential required to reach a provider was absent. The payload names the provider.
    NoCredential(String),
    /// A transport/connection error occurred while reaching a remote endpoint. The payload
    /// carries the underlying detail.
    TransportUnavailable(String),
    /// A test cassette/fixture was required but could not be found (tests only). The payload
    /// names the missing cassette.
    CassetteMissing(String),
}

impl Diagnostic {
    /// Construct a [`Diagnostic::NetDisabled`] naming the refused outbound operation.
    #[must_use]
    pub fn net_disabled(operation: impl Into<String>) -> Self {
        Diagnostic::NetDisabled(operation.into())
    }

    /// Construct a [`Diagnostic::NoCredential`] naming the provider whose credential is absent.
    #[must_use]
    pub fn no_credential(provider: impl Into<String>) -> Self {
        Diagnostic::NoCredential(provider.into())
    }

    /// Construct a [`Diagnostic::TransportUnavailable`] carrying the underlying detail.
    #[must_use]
    pub fn transport_unavailable(detail: impl Into<String>) -> Self {
        Diagnostic::TransportUnavailable(detail.into())
    }

    /// Construct a [`Diagnostic::CassetteMissing`] naming the missing cassette.
    #[must_use]
    pub fn cassette_missing(name: impl Into<String>) -> Self {
        Diagnostic::CassetteMissing(name.into())
    }

    /// The stable, machine-comparable identifier for this diagnostic class.
    ///
    /// These strings are part of the contract; do not rename existing values.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Diagnostic::NetDisabled(_) => "net-disabled",
            Diagnostic::NoCredential(_) => "no-credential",
            Diagnostic::TransportUnavailable(_) => "transport-unavailable",
            Diagnostic::CassetteMissing(_) => "cassette-missing",
        }
    }

    /// The process exit code this diagnostic maps to.
    ///
    /// `NetDisabled` carries its own dedicated code so a kill-switch refusal is distinguishable
    /// from other failures by `$?` alone; the remaining classes fall back to the generic failure
    /// code until later milestones give them dedicated handling.
    #[must_use]
    pub const fn exit(&self) -> Exit {
        match self {
            Diagnostic::NetDisabled(_) => Exit::NetDisabled,
            Diagnostic::NoCredential(_)
            | Diagnostic::TransportUnavailable(_)
            | Diagnostic::CassetteMissing(_) => Exit::Failure,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Diagnostic::NetDisabled(operation) => write!(
                f,
                "network disabled: CORTEX_NO_NET=1 refused {operation} before opening a connection"
            ),
            Diagnostic::NoCredential(provider) => {
                write!(f, "missing credential: no API key available for {provider}")
            }
            Diagnostic::TransportUnavailable(detail) => {
                write!(f, "transport unavailable: {detail}")
            }
            Diagnostic::CassetteMissing(name) => {
                write!(f, "cassette missing: no recorded fixture named {name}")
            }
        }
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_stable_and_distinct() {
        assert_eq!(Diagnostic::net_disabled("x").kind(), "net-disabled");
        assert_eq!(Diagnostic::no_credential("x").kind(), "no-credential");
        assert_eq!(
            Diagnostic::transport_unavailable("x").kind(),
            "transport-unavailable"
        );
        assert_eq!(Diagnostic::cassette_missing("x").kind(), "cassette-missing");

        let kinds = [
            Diagnostic::net_disabled("x").kind(),
            Diagnostic::no_credential("x").kind(),
            Diagnostic::transport_unavailable("x").kind(),
            Diagnostic::cassette_missing("x").kind(),
        ];
        let mut unique = kinds.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len(), "every kind must be distinct");
    }

    #[test]
    fn net_disabled_uses_the_dedicated_exit_code() {
        assert_eq!(Diagnostic::net_disabled("op").exit(), Exit::NetDisabled);
        assert_eq!(Diagnostic::net_disabled("op").exit().code(), 4);
    }

    #[test]
    fn net_disabled_exit_differs_from_credential_and_transport() {
        let net = Diagnostic::net_disabled("op").exit().code();
        let cred = Diagnostic::no_credential("openrouter").exit().code();
        let transport = Diagnostic::transport_unavailable("boom").exit().code();
        assert_ne!(net, cred, "net-disabled must not share credential's code");
        assert_ne!(
            net, transport,
            "net-disabled must not share transport's code"
        );
        assert_ne!(net, 0, "net-disabled is always a failure");
    }

    #[test]
    fn net_disabled_message_names_the_kill_switch_and_operation() {
        let message = Diagnostic::net_disabled("outbound connectivity check").to_string();
        assert!(
            message.contains("CORTEX_NO_NET"),
            "message must name the kill-switch: {message}"
        );
        assert!(
            message.contains("network disabled"),
            "message must read as a network-disabled error: {message}"
        );
        assert!(
            message.contains("outbound connectivity check"),
            "message must name the refused operation: {message}"
        );
    }

    #[test]
    fn messages_are_distinct_across_classes() {
        let net = Diagnostic::net_disabled("op").to_string();
        let cred = Diagnostic::no_credential("openrouter").to_string();
        let transport = Diagnostic::transport_unavailable("connection refused").to_string();

        assert!(net != cred && net != transport && cred != transport);
        // The net-disabled message must not read like a credential or transport error.
        assert!(!net.contains("missing credential"));
        assert!(!net.contains("transport unavailable"));
        assert!(!transport.contains("network disabled"));
        assert!(!cred.contains("network disabled"));
    }
}
