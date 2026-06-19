//! Provider/transport error normalization into the canonical [`errors::Diagnostic`] classes.
//!
//! Every failure the LLM layer can raise is one of a small, stable set of classes so the runner
//! and CLI can branch on it without parsing prose. The three network-facing classes the transport
//! distinguishes are deliberately separate: a [`LlmError::NetDisabled`] refusal (the `CORTEX_NO_NET`
//! kill-switch, before any socket opens), a [`LlmError::Transport`] failure (the connection itself
//! could not be established or stalled), and a [`LlmError::Upstream`] error (the provider accepted
//! the request but returned an error response). Each maps onto a canonical
//! [`errors::Diagnostic`] for exit-code mapping at the surface.

use std::fmt;

use errors::Diagnostic;

/// A normalized error from the LLM provider/transport layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// The `CORTEX_NO_NET` kill-switch refused the operation before any socket was opened. The
    /// payload names the refused operation.
    NetDisabled(String),
    /// No credential was available for the provider. The payload names the provider.
    NoCredential(String),
    /// The connection could not be established, was reset, or stalled past the idle timeout. The
    /// payload carries the (secret-scrubbed) underlying detail.
    Transport(String),
    /// The provider accepted the connection but returned an error (auth rejected, unknown model,
    /// rate-limited, server error). The payload carries the normalized, scrubbed detail.
    Upstream(String),
    /// A response body could not be decoded into the expected protocol shape.
    Decode(String),
    /// A required test cassette/fixture was missing (tests only). The payload names the cassette.
    CassetteMissing(String),
}

impl LlmError {
    /// Construct a [`LlmError::NetDisabled`] naming the refused operation.
    #[must_use]
    pub fn net_disabled(operation: impl Into<String>) -> Self {
        LlmError::NetDisabled(operation.into())
    }

    /// Construct a [`LlmError::NoCredential`] naming the provider.
    #[must_use]
    pub fn no_credential(provider: impl Into<String>) -> Self {
        LlmError::NoCredential(provider.into())
    }

    /// Construct a [`LlmError::Transport`], scrubbing any secret-shaped substring from the detail.
    #[must_use]
    pub fn transport(detail: impl Into<String>) -> Self {
        LlmError::Transport(redaction::redact(&detail.into()))
    }

    /// Construct a [`LlmError::Upstream`], scrubbing any secret-shaped substring from the detail.
    #[must_use]
    pub fn upstream(detail: impl Into<String>) -> Self {
        LlmError::Upstream(redaction::redact(&detail.into()))
    }

    /// Construct a [`LlmError::Decode`], scrubbing any secret-shaped substring from the detail.
    #[must_use]
    pub fn decode(detail: impl Into<String>) -> Self {
        LlmError::Decode(redaction::redact(&detail.into()))
    }

    /// Construct a [`LlmError::CassetteMissing`] naming the cassette.
    #[must_use]
    pub fn cassette_missing(name: impl Into<String>) -> Self {
        LlmError::CassetteMissing(name.into())
    }

    /// Lift a canonical [`errors::Diagnostic`] (e.g. from the outbound guard) into an [`LlmError`].
    #[must_use]
    pub fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        match diagnostic {
            Diagnostic::NetDisabled(operation) => LlmError::NetDisabled(operation),
            Diagnostic::NoCredential(provider) => LlmError::NoCredential(provider),
            Diagnostic::TransportUnavailable(detail) => LlmError::Transport(detail),
            Diagnostic::Upstream(detail) => LlmError::Upstream(detail),
            Diagnostic::CassetteMissing(name) => LlmError::CassetteMissing(name),
        }
    }

    /// The stable, machine-comparable class identifier, aligned with [`errors::Diagnostic::kind`].
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.to_diagnostic().kind()
    }

    /// Normalize this error into a canonical [`errors::Diagnostic`] for exit-code mapping.
    ///
    /// A [`LlmError::Decode`] folds into the transport class (a malformed frame is a transport-level
    /// protocol failure); the remaining classes map one-to-one.
    #[must_use]
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            LlmError::NetDisabled(operation) => Diagnostic::net_disabled(operation),
            LlmError::NoCredential(provider) => Diagnostic::no_credential(provider),
            LlmError::Transport(detail) => Diagnostic::transport_unavailable(detail),
            LlmError::Decode(detail) => {
                Diagnostic::transport_unavailable(format!("malformed response: {detail}"))
            }
            LlmError::Upstream(detail) => Diagnostic::upstream(detail),
            LlmError::CassetteMissing(name) => Diagnostic::cassette_missing(name),
        }
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_diagnostic())
    }
}

impl std::error::Error for LlmError {}

impl From<Diagnostic> for LlmError {
    fn from(diagnostic: Diagnostic) -> Self {
        LlmError::from_diagnostic(diagnostic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_map_to_distinct_canonical_kinds() {
        assert_eq!(LlmError::net_disabled("op").kind(), "net-disabled");
        assert_eq!(
            LlmError::no_credential("openrouter").kind(),
            "no-credential"
        );
        assert_eq!(LlmError::transport("reset").kind(), "transport-unavailable");
        assert_eq!(LlmError::decode("bad json").kind(), "transport-unavailable");
        assert_eq!(LlmError::upstream("401").kind(), "upstream");
        assert_eq!(LlmError::cassette_missing("x").kind(), "cassette-missing");
    }

    #[test]
    fn net_disabled_transport_and_upstream_are_distinct_classes() {
        let net = LlmError::net_disabled("op").kind();
        let transport = LlmError::transport("reset").kind();
        let upstream = LlmError::upstream("401").kind();
        assert_ne!(net, transport);
        assert_ne!(net, upstream);
        assert_ne!(transport, upstream);
    }

    #[test]
    fn net_disabled_keeps_its_dedicated_exit_code() {
        assert_eq!(
            LlmError::net_disabled("op").to_diagnostic().exit().code(),
            4
        );
    }

    #[test]
    fn transport_constructor_scrubs_secret_shaped_detail() {
        let key = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8));
        let err = LlmError::transport(format!("connect failed using {key}"));
        assert!(
            !err.to_string().contains(&key),
            "secret must be scrubbed from transport detail: {err}"
        );
        assert!(err.to_string().contains(redaction::PLACEHOLDER));
    }

    #[test]
    fn round_trips_through_diagnostic() {
        for err in [
            LlmError::net_disabled("op"),
            LlmError::no_credential("openrouter"),
            LlmError::transport("reset"),
            LlmError::upstream("401"),
            LlmError::cassette_missing("c"),
        ] {
            let lifted = LlmError::from_diagnostic(err.to_diagnostic());
            assert_eq!(lifted.kind(), err.kind());
        }
    }
}
