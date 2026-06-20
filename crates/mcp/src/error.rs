//! Errors raised while connecting to or talking with an MCP server.

use std::time::Duration;

/// A failure connecting to or operating an MCP server.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The server's command/config was invalid (e.g. an empty command array).
    #[error("MCP server `{server}`: {message}")]
    Config {
        /// The server name.
        server: String,
        /// The human-readable reason.
        message: String,
    },
    /// The child process could not be spawned.
    #[error("MCP server `{server}`: failed to spawn `{program}`: {source}")]
    Spawn {
        /// The server name.
        server: String,
        /// The program that failed to launch.
        program: String,
        /// The underlying spawn error.
        source: std::io::Error,
    },
    /// The initialize handshake failed.
    #[error("MCP server `{server}`: handshake failed: {message}")]
    Handshake {
        /// The server name.
        server: String,
        /// The normalized handshake error.
        message: String,
    },
    /// A remote server requires OAuth authorization before it will serve.
    #[error("MCP server `{server}`: authorization required (run `cortex mcp auth {server}`)")]
    NeedsAuth {
        /// The server name.
        server: String,
        /// The server's `WWW-Authenticate` challenge, when provided.
        www_authenticate: Option<String>,
    },
    /// An operation timed out.
    #[error("MCP server `{server}`: {operation} timed out after {timeout:?}")]
    Timeout {
        /// The server name.
        server: String,
        /// The operation that timed out (e.g. `tools/call`).
        operation: String,
        /// The elapsed budget.
        timeout: Duration,
    },
    /// A protocol/service operation failed.
    #[error("MCP server `{server}`: {operation} failed: {message}")]
    Service {
        /// The server name.
        server: String,
        /// The operation that failed.
        operation: String,
        /// The normalized service error.
        message: String,
    },
    /// The targeted server connection is closed.
    #[error("MCP server `{server}` is not connected")]
    Closed {
        /// The server name.
        server: String,
    },
    /// A tool id did not resolve to any connected MCP tool.
    #[error("unknown MCP tool `{id}`")]
    UnknownTool {
        /// The qualified tool id that did not resolve.
        id: String,
    },
}

impl McpError {
    pub(crate) fn config(server: &str, message: impl Into<String>) -> Self {
        Self::Config {
            server: server.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn handshake(server: &str, message: impl Into<String>) -> Self {
        Self::Handshake {
            server: server.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn needs_auth(server: &str, www_authenticate: Option<String>) -> Self {
        Self::NeedsAuth {
            server: server.to_owned(),
            www_authenticate,
        }
    }

    pub(crate) fn timeout(server: &str, operation: &str, timeout: Duration) -> Self {
        Self::Timeout {
            server: server.to_owned(),
            operation: operation.to_owned(),
            timeout,
        }
    }

    pub(crate) fn service(server: &str, operation: &str, message: impl Into<String>) -> Self {
        Self::Service {
            server: server.to_owned(),
            operation: operation.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn closed(server: &str) -> Self {
        Self::Closed {
            server: server.to_owned(),
        }
    }

    pub(crate) fn unknown_tool(id: &str) -> Self {
        Self::UnknownTool { id: id.to_owned() }
    }
}
