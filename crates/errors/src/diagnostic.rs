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
    /// A remote provider accepted the connection but returned an error response (e.g. an
    /// authentication rejection, an unknown model, or a rate-limit). The payload carries the
    /// normalized upstream detail. This is distinct from a [`Diagnostic::TransportUnavailable`]
    /// (the connection itself succeeded) and from a [`Diagnostic::NoCredential`] (no local key).
    Upstream(String),
    /// A test cassette/fixture was required but could not be found (tests only). The payload
    /// names the missing cassette.
    CassetteMissing(String),
    /// A turn exhausted its step budget (`MAX_STEPS`) without completing, so the durable runner
    /// stopped with an explicit, non-crashing signal rather than looping. The payload carries the
    /// human-readable detail (e.g. the budget that was reached).
    StepLimitExceeded(String),
    /// A turn's context exceeded the model's window and compaction/overflow recovery could not
    /// shrink it enough to proceed, so the durable runner stopped with an explicit, non-crashing
    /// signal rather than hanging or panicking. The payload carries the human-readable detail.
    ContextOverflow(String),
    /// The final assistant output failed the requested structured-output schema validation. The
    /// payload carries the validation detail.
    SchemaValidation(String),
    /// The runner detected repeated identical tool calls (a doom-loop) and stopped before
    /// executing further side effects. The payload carries the loop detail.
    DoomLoop(String),
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

    /// Construct a [`Diagnostic::Upstream`] carrying the normalized provider-side detail.
    #[must_use]
    pub fn upstream(detail: impl Into<String>) -> Self {
        Diagnostic::Upstream(detail.into())
    }

    /// Construct a [`Diagnostic::CassetteMissing`] naming the missing cassette.
    #[must_use]
    pub fn cassette_missing(name: impl Into<String>) -> Self {
        Diagnostic::CassetteMissing(name.into())
    }

    /// Construct a [`Diagnostic::StepLimitExceeded`] carrying the step-budget detail.
    #[must_use]
    pub fn step_limit_exceeded(detail: impl Into<String>) -> Self {
        Diagnostic::StepLimitExceeded(detail.into())
    }

    /// Construct a [`Diagnostic::ContextOverflow`] carrying the context-overflow detail.
    #[must_use]
    pub fn context_overflow(detail: impl Into<String>) -> Self {
        Diagnostic::ContextOverflow(detail.into())
    }

    /// Construct a [`Diagnostic::SchemaValidation`] carrying the schema-validation detail.
    #[must_use]
    pub fn schema_validation(detail: impl Into<String>) -> Self {
        Diagnostic::SchemaValidation(detail.into())
    }

    /// Construct a [`Diagnostic::DoomLoop`] carrying the loop-detection detail.
    #[must_use]
    pub fn doom_loop(detail: impl Into<String>) -> Self {
        Diagnostic::DoomLoop(detail.into())
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
            Diagnostic::Upstream(_) => "upstream",
            Diagnostic::CassetteMissing(_) => "cassette-missing",
            Diagnostic::StepLimitExceeded(_) => "step-limit-exceeded",
            Diagnostic::ContextOverflow(_) => "context-overflow",
            Diagnostic::SchemaValidation(_) => "schema-validation",
            Diagnostic::DoomLoop(_) => "doom-loop",
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
            | Diagnostic::Upstream(_)
            | Diagnostic::CassetteMissing(_)
            | Diagnostic::StepLimitExceeded(_)
            | Diagnostic::ContextOverflow(_)
            | Diagnostic::SchemaValidation(_)
            | Diagnostic::DoomLoop(_) => Exit::Failure,
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
            Diagnostic::Upstream(detail) => {
                write!(f, "upstream error: {detail}")
            }
            Diagnostic::CassetteMissing(name) => {
                write!(f, "cassette missing: no recorded fixture named {name}")
            }
            Diagnostic::StepLimitExceeded(detail) => {
                write!(f, "step limit exceeded: {detail}")
            }
            Diagnostic::ContextOverflow(detail) => {
                write!(f, "context overflow: {detail}")
            }
            Diagnostic::SchemaValidation(detail) => {
                write!(f, "schema validation failed: {detail}")
            }
            Diagnostic::DoomLoop(detail) => {
                write!(f, "doom-loop detected: {detail}")
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
        assert_eq!(Diagnostic::upstream("x").kind(), "upstream");
        assert_eq!(Diagnostic::cassette_missing("x").kind(), "cassette-missing");
        assert_eq!(
            Diagnostic::step_limit_exceeded("x").kind(),
            "step-limit-exceeded"
        );
        assert_eq!(Diagnostic::context_overflow("x").kind(), "context-overflow");
        assert_eq!(
            Diagnostic::schema_validation("x").kind(),
            "schema-validation"
        );
        assert_eq!(Diagnostic::doom_loop("x").kind(), "doom-loop");

        let kinds = [
            Diagnostic::net_disabled("x").kind(),
            Diagnostic::no_credential("x").kind(),
            Diagnostic::transport_unavailable("x").kind(),
            Diagnostic::upstream("x").kind(),
            Diagnostic::cassette_missing("x").kind(),
            Diagnostic::step_limit_exceeded("x").kind(),
            Diagnostic::context_overflow("x").kind(),
            Diagnostic::schema_validation("x").kind(),
            Diagnostic::doom_loop("x").kind(),
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
    fn step_limit_is_a_failure_with_an_explicit_message() {
        let diag = Diagnostic::step_limit_exceeded("reached the maximum of 25 steps");
        assert_eq!(diag.kind(), "step-limit-exceeded");
        assert_eq!(diag.exit(), Exit::Failure);
        let message = diag.to_string();
        assert!(
            message.contains("step limit exceeded"),
            "message must read as a step-limit error: {message}"
        );
        assert!(
            message.contains("25 steps"),
            "message must carry the step-budget detail: {message}"
        );
    }

    #[test]
    fn context_overflow_is_a_failure_with_an_explicit_message() {
        let diag = Diagnostic::context_overflow(
            "the request exceeds the model's context window even after compaction",
        );
        assert_eq!(diag.kind(), "context-overflow");
        assert_eq!(diag.exit(), Exit::Failure);
        let message = diag.to_string();
        assert!(
            message.contains("context overflow"),
            "message must read as a context-overflow error: {message}"
        );
        assert!(
            message.contains("context window"),
            "message must carry the overflow detail: {message}"
        );
    }

    #[test]
    fn messages_are_distinct_across_classes() {
        let net = Diagnostic::net_disabled("op").to_string();
        let cred = Diagnostic::no_credential("openrouter").to_string();
        let transport = Diagnostic::transport_unavailable("connection refused").to_string();
        let schema = Diagnostic::schema_validation("expected object").to_string();
        let doom_loop = Diagnostic::doom_loop("tool bash repeated").to_string();

        assert!(
            net != cred
                && net != transport
                && net != schema
                && net != doom_loop
                && cred != transport
                && cred != schema
                && cred != doom_loop
                && transport != schema
                && transport != doom_loop
                && schema != doom_loop
        );
        // The net-disabled message must not read like a credential or transport error.
        assert!(!net.contains("missing credential"));
        assert!(!net.contains("transport unavailable"));
        assert!(!transport.contains("network disabled"));
        assert!(!cred.contains("network disabled"));
        assert!(schema.contains("schema validation failed"));
        assert!(doom_loop.contains("doom-loop detected"));
    }
}
