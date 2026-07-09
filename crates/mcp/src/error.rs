//! Errors raised while connecting to or talking with an MCP server.

use std::time::Duration;

use redaction::PLACEHOLDER;

/// Scrub known opaque credentials as well as generally recognizable secret-shaped values from
/// untrusted MCP server messages before they reach a status, tool result, or log sink.
///
/// OAuth bearer tokens are provider-defined and need not resemble API keys, so the shared generic
/// redactor alone cannot recognize them. The active credentials are supplied by the connection
/// layer and replaced exactly here, before errors can leave the MCP crate.
#[must_use]
pub(crate) fn redact_message(message: impl AsRef<str>, secrets: &[String]) -> String {
    let mut redacted = redaction::redact(message.as_ref());
    let mut known = secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .collect::<Vec<_>>();
    known.sort_unstable_by_key(|secret| std::cmp::Reverse(secret.len()));
    for secret in known {
        redacted = redacted.replace(secret.as_str(), PLACEHOLDER);
    }
    redaction::redact(&redacted)
}

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
    /// A resource id/URI did not resolve to any connected MCP resource.
    #[error("unknown MCP resource `{id}`")]
    UnknownResource {
        /// The qualified resource id or URI that did not resolve.
        id: String,
    },
    /// A prompt id/name did not resolve to any connected MCP prompt.
    #[error("unknown MCP prompt `{id}`")]
    UnknownPrompt {
        /// The qualified prompt id or prompt name that did not resolve.
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

    pub(crate) fn unknown_resource(id: &str) -> Self {
        Self::UnknownResource { id: id.to_owned() }
    }

    pub(crate) fn unknown_prompt(id: &str) -> Self {
        Self::UnknownPrompt { id: id.to_owned() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_an_opaque_bearer_token_reflected_by_a_remote_server() {
        let token = "opaque-oauth-access-token-value".to_owned();
        let reflected = format!("remote error included Bearer {token}");
        let redacted = redact_message(reflected, &[token]);

        assert_eq!(
            redacted,
            format!("remote error included Bearer {PLACEHOLDER}")
        );
    }
}
