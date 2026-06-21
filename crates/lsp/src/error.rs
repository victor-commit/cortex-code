//! Error surface for local Language Server Protocol clients.

use thiserror::Error;

/// Errors raised while spawning a language server or driving its stdio JSON-RPC lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LspError {
    /// The configured language-server command is empty or otherwise unusable.
    #[error("the language server command is invalid")]
    InvalidCommand,
    /// The child process could not be spawned.
    #[error("the language server process could not be started")]
    Spawn,
    /// The stdio connection failed during read or write.
    #[error("the language server stdio connection failed")]
    Io,
    /// No response arrived within the configured timeout.
    #[error("the language server did not respond in time")]
    Timeout,
    /// A framed message could not be parsed as a valid JSON-RPC payload.
    #[error("the language server sent a malformed message")]
    Protocol,
    /// A framed message advertised a body larger than the configured limit.
    #[error("the language server message exceeded the size limit")]
    ResponseTooLarge,
    /// The child closed its output stream before the expected response arrived.
    #[error("the language server stream closed unexpectedly")]
    Closed,
    /// The server replied with a JSON-RPC error object for the request.
    #[error("the language server returned a protocol error")]
    ServerError {
        /// The JSON-RPC error code reported by the server.
        code: i64,
    },
}

impl LspError {
    /// A stable machine-readable code for this error.
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::InvalidCommand => "lsp.invalid_command",
            Self::Spawn => "lsp.spawn_failed",
            Self::Io => "lsp.io_failed",
            Self::Timeout => "lsp.timeout",
            Self::Protocol => "lsp.protocol_violation",
            Self::ResponseTooLarge => "lsp.response_too_large",
            Self::Closed => "lsp.stream_closed",
            Self::ServerError { .. } => "lsp.server_error",
        }
    }
}
