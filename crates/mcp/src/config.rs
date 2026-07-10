//! Declarative configuration for an MCP server entry (`mcp.<name>` in the layered TOML config).
//!
//! A server is declared by transport `type` (`local` stdio, or `remote` for the HTTP/SSE
//! transport), a `command` array (the program plus its arguments for a local server) or a `url`
//! (plus optional `headers`) for a remote server, and the optional `cwd`/`environment`/`timeout`
//! knobs. An entry can be turned off with `enabled = false` so a configured server is skipped
//! without removing its definition.
//!
//! Parsing is decoupled from the config crate's TOML representation: the CLI converts the merged
//! `mcp` table to a [`serde_json::Value`] (which `toml::Value` serializes into losslessly) and hands
//! it to [`servers_from_value`], so this crate depends only on `serde`/`serde_json`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The transport a configured MCP server is reached over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// A local server spawned as a child process and spoken to over stdio (the default).
    #[default]
    Local,
    /// A remote server reached over HTTP (Streamable HTTP, falling back to the legacy HTTP+SSE
    /// transport).
    Remote,
}

/// One configured MCP server entry.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    /// The transport used to reach the server. Defaults to [`McpTransport::Local`] (stdio).
    #[serde(rename = "type", default)]
    pub transport: McpTransport,
    /// The command to launch for a local server: the program followed by its arguments.
    #[serde(default)]
    pub command: Vec<String>,
    /// The base URL of a remote server (HTTP/SSE transport). Required for a `remote` entry.
    #[serde(default)]
    pub url: Option<String>,
    /// Extra HTTP headers sent with every request to a remote server (e.g. an API token). The
    /// `Authorization` header obtained via the OAuth flow is layered on top of these.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// The working directory the local server child is spawned in (defaults to the cortex cwd).
    #[serde(default)]
    pub cwd: Option<String>,
    /// Extra environment variables to set on the spawned child, layered on top of the documented
    /// safe execution baseline (`PATH` on Unix, plus required Windows loader variables).
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// The per-operation timeout (handshake and each request) in seconds. Falls back to the
    /// built-in default when absent.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Whether this server is connected. A disabled entry is never spawned and its tools are absent.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional OAuth overrides for a remote server.
    #[serde(default)]
    pub oauth: Option<McpOAuthConfig>,
}

const fn default_enabled() -> bool {
    true
}

impl McpServerConfig {
    /// The program (first element of [`command`](Self::command)) and its arguments, if a command is
    /// declared.
    #[must_use]
    pub fn program_and_args(&self) -> Option<(&str, &[String])> {
        self.command
            .split_first()
            .map(|(program, args)| (program.as_str(), args))
    }
}

/// Optional OAuth settings for one remote MCP server.
#[derive(Debug, Clone, Deserialize)]
pub struct McpOAuthConfig {
    /// OAuth client id override.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Explicit authorization endpoint override.
    #[serde(default)]
    pub authorize_endpoint: Option<String>,
    /// Explicit token endpoint override.
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// Explicit scope list override.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Preferred OAuth loopback callback port.
    #[serde(default)]
    pub callback_port: Option<u16>,
}

/// A failure parsing the `mcp` config table into typed server entries.
#[derive(Debug, thiserror::Error)]
#[error("invalid `mcp` configuration: {0}")]
pub struct McpConfigError(String);

impl McpConfigError {
    #[must_use]
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Parse the merged `mcp` config table (as a JSON value) into a name-keyed map of server entries.
///
/// A null/absent value yields an empty map. The value must be a JSON object mapping each server
/// name to its [`McpServerConfig`]; anything else (or a malformed entry) is a [`McpConfigError`].
///
/// # Errors
/// Returns [`McpConfigError`] when the `mcp` value is not an object of valid server entries.
pub fn servers_from_value(
    value: &serde_json::Value,
) -> Result<BTreeMap<String, McpServerConfig>, McpConfigError> {
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    if !value.is_object() {
        return Err(McpConfigError::new("expected a table of named servers"));
    }
    serde_json::from_value(value.clone())
        .map_err(|err| McpConfigError::new(format!("could not parse server entries: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_local_stdio_entry_with_defaults() {
        let value = json!({
            "demo": { "type": "local", "command": ["echo-server", "--flag"] }
        });
        let servers = servers_from_value(&value).expect("parse");
        let demo = servers.get("demo").expect("demo present");
        assert_eq!(demo.transport, McpTransport::Local);
        assert_eq!(demo.command, vec!["echo-server", "--flag"]);
        assert!(demo.enabled, "entries are enabled by default");
        assert_eq!(
            demo.program_and_args(),
            Some(("echo-server", &["--flag".to_owned()][..]))
        );
    }

    #[test]
    fn type_defaults_to_local_when_omitted() {
        let value = json!({ "demo": { "command": ["srv"] } });
        let servers = servers_from_value(&value).expect("parse");
        assert_eq!(servers["demo"].transport, McpTransport::Local);
    }

    #[test]
    fn parses_cwd_environment_and_timeout() {
        let value = json!({
            "demo": {
                "type": "local",
                "command": ["srv"],
                "cwd": "/tmp/work",
                "environment": { "FOO": "bar" },
                "timeout": 5
            }
        });
        let demo = &servers_from_value(&value).expect("parse")["demo"];
        assert_eq!(demo.cwd.as_deref(), Some("/tmp/work"));
        assert_eq!(demo.environment.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(demo.timeout, Some(5));
    }

    #[test]
    fn disabled_entry_is_parsed_as_disabled() {
        let value = json!({ "demo": { "command": ["srv"], "enabled": false } });
        assert!(!servers_from_value(&value).expect("parse")["demo"].enabled);
    }

    #[test]
    fn remote_transport_is_recognized() {
        let value = json!({ "api": { "type": "remote" } });
        assert_eq!(
            servers_from_value(&value).expect("parse")["api"].transport,
            McpTransport::Remote
        );
    }

    #[test]
    fn parses_remote_url_and_headers() {
        let value = json!({
            "api": {
                "type": "remote",
                "url": "http://127.0.0.1:3201/mcp",
                "headers": { "X-Api": "abc" }
            }
        });
        let api = &servers_from_value(&value).expect("parse")["api"];
        assert_eq!(api.transport, McpTransport::Remote);
        assert_eq!(api.url.as_deref(), Some("http://127.0.0.1:3201/mcp"));
        assert_eq!(api.headers.get("X-Api").map(String::as_str), Some("abc"));
    }

    #[test]
    fn parses_remote_oauth_overrides() {
        let value = json!({
            "api": {
                "type": "remote",
                "url": "http://127.0.0.1:3201/mcp",
                "oauth": {
                    "client_id": "demo-client",
                    "authorize_endpoint": "http://127.0.0.1:3201/authz",
                    "token_endpoint": "http://127.0.0.1:3201/token",
                    "scopes": ["openid", "profile"],
                    "callback_port": 3227
                }
            }
        });
        let api = &servers_from_value(&value).expect("parse")["api"];
        let oauth = api.oauth.as_ref().expect("oauth");
        assert_eq!(oauth.client_id.as_deref(), Some("demo-client"));
        assert_eq!(
            oauth.authorize_endpoint.as_deref(),
            Some("http://127.0.0.1:3201/authz")
        );
        assert_eq!(
            oauth.token_endpoint.as_deref(),
            Some("http://127.0.0.1:3201/token")
        );
        assert_eq!(oauth.scopes, vec!["openid", "profile"]);
        assert_eq!(oauth.callback_port, Some(3227));
    }

    #[test]
    fn null_value_is_an_empty_map() {
        assert!(
            servers_from_value(&serde_json::Value::Null)
                .expect("parse")
                .is_empty()
        );
    }

    #[test]
    fn non_object_value_is_an_error() {
        assert!(servers_from_value(&json!("nope")).is_err());
    }
}
