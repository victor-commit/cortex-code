//! A single connected MCP server session — local (stdio child) or remote (HTTP) — exposing the
//! initialize handshake, `tools/list`, and `tools/call` over the running session.
//!
//! A local server is spawned as a child process parented under cortex; its PID is recorded so the
//! whole process tree can be terminated on shutdown (no orphan survives). A remote server is reached
//! over HTTP: the client first tries the Streamable HTTP transport and, if the server rejects it,
//! falls back to the legacy HTTP+SSE transport, so either kind of remote server connects. Optional
//! per-server headers and an OAuth bearer token are sent with every remote request. Every session is
//! served with a [`NotifyHandler`] so a server's tool-list-changed notification refreshes the live
//! catalogue without a restart.

use std::collections::HashMap;
use std::ffi::OsString;
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, GetPromptRequestParams, GetPromptResult,
    ReadResourceRequestParams, ReadResourceResult,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::catalogue::{
    LiveCatalogue, McpServerCatalogue, qualify_prompts, qualify_resources, qualify_tools,
};
use crate::config::McpServerConfig;
use crate::error::McpError;
use crate::handler::NotifyHandler;
use crate::transport_sse::SseClientTransport;

/// The default per-operation timeout (handshake and each request) when a server sets none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// A connected MCP server session (stdio or remote).
pub struct McpClient {
    server_name: String,
    pid: Option<u32>,
    service: Mutex<Option<RunningService<RoleClient, NotifyHandler>>>,
    timeout: Duration,
}

impl McpClient {
    /// Spawn the configured local server and complete the MCP initialize handshake.
    ///
    /// The child inherits the cortex environment minus known host secrets, with the configured
    /// `environment` overrides applied and the optional `cwd` honored. After the handshake the
    /// server's tools are listed, qualified, and recorded in `catalogue`.
    ///
    /// # Errors
    /// Returns [`McpError`] when the command is empty, the child cannot be spawned, or the handshake
    /// or tool listing fails or times out.
    pub async fn connect_stdio(
        server_name: &str,
        config: &McpServerConfig,
        catalogue: &LiveCatalogue,
        roots: &[rmcp::model::Root],
    ) -> Result<(Self, McpServerCatalogue), McpError> {
        let (program, args) = config
            .program_and_args()
            .ok_or_else(|| McpError::config(server_name, "the `command` array is empty"))?;

        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        command.env_clear();
        command.envs(child_env());
        for (key, value) in &config.environment {
            command.env(key, value);
        }

        let timeout = Self::timeout_of(config);

        let (transport, _stderr) = TokioChildProcess::builder(command)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|source| McpError::Spawn {
                server: server_name.to_owned(),
                program: program.to_owned(),
                source,
            })?;
        let pid = transport.id();

        let handler = NotifyHandler::new(
            server_name.to_owned(),
            catalogue.clone(),
            timeout,
            roots.to_vec(),
        );
        let service = match tokio::time::timeout(timeout, handler.serve(transport)).await {
            Ok(Ok(service)) => service,
            Ok(Err(err)) => {
                kill_spawned_tree(pid).await;
                return Err(McpError::handshake(server_name, err.to_string()));
            }
            Err(_) => {
                kill_spawned_tree(pid).await;
                return Err(McpError::timeout(server_name, "initialize", timeout));
            }
        };

        let client = Self {
            server_name: server_name.to_owned(),
            pid,
            service: Mutex::new(Some(service)),
            timeout,
        };
        let tools = client.list_and_record(catalogue).await?;
        Ok((client, tools))
    }

    /// Connect to the configured remote server over HTTP and complete the handshake.
    ///
    /// The Streamable HTTP transport is tried first; if the server rejects it (a non-streamable
    /// server) the legacy HTTP+SSE transport is tried. `auth_header` carries an OAuth bearer token
    /// (without the `Bearer ` prefix) when one is available. A server that responds `401` before any
    /// transport connects yields [`McpError::NeedsAuth`].
    ///
    /// # Errors
    /// Returns [`McpError`] when no URL is configured, the server needs authorization, or neither
    /// transport can complete the handshake/listing.
    pub async fn connect_remote(
        server_name: &str,
        config: &McpServerConfig,
        auth_header: Option<String>,
        catalogue: &LiveCatalogue,
        roots: &[rmcp::model::Root],
    ) -> Result<(Self, McpServerCatalogue), McpError> {
        let url = config
            .url
            .as_deref()
            .filter(|url| !url.is_empty())
            .ok_or_else(|| McpError::config(server_name, "a remote server requires a `url`"))?;
        let timeout = Self::timeout_of(config);
        let header_map = build_header_map(server_name, &config.headers, auth_header.as_deref())?;

        match probe(url, &header_map, timeout).await {
            Probe::NeedsAuth(www) => Err(McpError::needs_auth(server_name, www)),
            Probe::Streamable => {
                Self::connect_streamable(
                    server_name,
                    url,
                    &header_map,
                    auth_header,
                    timeout,
                    catalogue,
                    roots,
                )
                .await
            }
            Probe::Sse => {
                Self::connect_sse(server_name, url, header_map, timeout, catalogue, roots).await
            }
            Probe::Unreachable(message) => Err(McpError::handshake(server_name, message)),
        }
    }

    /// Connect over the Streamable HTTP transport.
    async fn connect_streamable(
        server_name: &str,
        url: &str,
        header_map: &HeaderMap,
        auth_header: Option<String>,
        timeout: Duration,
        catalogue: &LiveCatalogue,
        roots: &[rmcp::model::Root],
    ) -> Result<(Self, McpServerCatalogue), McpError> {
        let mut custom: HashMap<HeaderName, HeaderValue> = HashMap::new();
        let oauth_header_supplied = auth_header.is_some();
        for (name, value) in header_map {
            // An OAuth bearer token is supplied via auth_header below. A caller-configured
            // Authorization header, however, is a normal custom header and must survive when no
            // OAuth token is active.
            if name == http::header::AUTHORIZATION && oauth_header_supplied {
                continue;
            }
            custom.insert(name.clone(), value.clone());
        }
        let mut transport_config =
            StreamableHttpClientTransportConfig::with_uri(url.to_owned()).custom_headers(custom);
        if let Some(token) = auth_header {
            transport_config = transport_config.auth_header(token);
        }
        let transport = StreamableHttpClientTransport::from_config(transport_config);

        let handler = NotifyHandler::new(
            server_name.to_owned(),
            catalogue.clone(),
            timeout,
            roots.to_vec(),
        );
        let service = tokio::time::timeout(timeout, handler.serve(transport))
            .await
            .map_err(|_| McpError::timeout(server_name, "initialize", timeout))?
            .map_err(|err| McpError::handshake(server_name, err.to_string()))?;

        let client = Self {
            server_name: server_name.to_owned(),
            pid: None,
            service: Mutex::new(Some(service)),
            timeout,
        };
        let tools = client.list_and_record(catalogue).await?;
        Ok((client, tools))
    }

    /// Connect over the legacy HTTP+SSE transport (fallback).
    async fn connect_sse(
        server_name: &str,
        url: &str,
        header_map: HeaderMap,
        timeout: Duration,
        catalogue: &LiveCatalogue,
        roots: &[rmcp::model::Root],
    ) -> Result<(Self, McpServerCatalogue), McpError> {
        let transport = SseClientTransport::connect(url, header_map, timeout)
            .await
            .map_err(|err| McpError::handshake(server_name, err.to_string()))?;

        let handler = NotifyHandler::new(
            server_name.to_owned(),
            catalogue.clone(),
            timeout,
            roots.to_vec(),
        );
        let service = tokio::time::timeout(timeout, handler.serve(transport))
            .await
            .map_err(|_| McpError::timeout(server_name, "initialize", timeout))?
            .map_err(|err| McpError::handshake(server_name, err.to_string()))?;

        let client = Self {
            server_name: server_name.to_owned(),
            pid: None,
            service: Mutex::new(Some(service)),
            timeout,
        };
        let tools = client.list_and_record(catalogue).await?;
        Ok((client, tools))
    }

    /// List the server's tools, resources, and prompts, qualify them, record them in the
    /// catalogue, and return the full advertised catalogue. `tools/list` is required for the tool
    /// pipeline; prompts/resources are best-effort because many MCP servers do not implement those
    /// capabilities.
    async fn list_and_record(
        &self,
        catalogue: &LiveCatalogue,
    ) -> Result<McpServerCatalogue, McpError> {
        let advertised_tools = match self.list_tools().await {
            Ok(advertised) => advertised,
            Err(err) => {
                self.shutdown().await;
                return Err(err);
            }
        };
        let tools = qualify_tools(&self.server_name, advertised_tools);
        let resources = self
            .list_resources()
            .await
            .map(|advertised| qualify_resources(&self.server_name, advertised))
            .unwrap_or_default();
        let prompts = self
            .list_prompts()
            .await
            .map(|advertised| qualify_prompts(&self.server_name, advertised))
            .unwrap_or_default();
        let server_catalogue = McpServerCatalogue {
            tools,
            resources,
            prompts,
        };
        catalogue.set_initial(&self.server_name, server_catalogue.clone());
        Ok(server_catalogue)
    }

    /// The configured per-operation timeout, or the built-in default.
    fn timeout_of(config: &McpServerConfig) -> Duration {
        config
            .timeout
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// The PID of the spawned child process (stdio servers only).
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Issue `tools/list` (following pagination) and return the server's advertised tools.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed or the request fails/times out.
    pub async fn list_tools(&self) -> Result<Vec<rmcp::model::Tool>, McpError> {
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.list_all_tools())
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "tools/list", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "tools/list", err.to_string()))
    }

    /// Issue `resources/list` (following pagination) and return the server's advertised resources.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed or the request fails/times out.
    pub async fn list_resources(&self) -> Result<Vec<rmcp::model::Resource>, McpError> {
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.list_all_resources())
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "resources/list", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "resources/list", err.to_string()))
    }

    /// Issue `resources/read` for a resource URI and return the server's content.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed or the request fails/times out.
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        let params = ReadResourceRequestParams::new(uri.to_owned());
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.read_resource(params))
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "resources/read", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "resources/read", err.to_string()))
    }

    /// Issue `prompts/list` (following pagination) and return the server's advertised prompts.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed or the request fails/times out.
    pub async fn list_prompts(&self) -> Result<Vec<rmcp::model::Prompt>, McpError> {
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.list_all_prompts())
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "prompts/list", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "prompts/list", err.to_string()))
    }

    /// Issue `prompts/get` with arguments and return the server's prompt messages.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed, arguments are not an object, or the request
    /// fails/times out.
    pub async fn get_prompt(
        &self,
        prompt: &str,
        arguments: serde_json::Value,
    ) -> Result<GetPromptResult, McpError> {
        let arguments = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(McpError::service(
                    &self.server_name,
                    "prompts/get",
                    format!("arguments must be a JSON object, got {other}"),
                ));
            }
        };
        let mut params = GetPromptRequestParams::new(prompt.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.get_prompt(params))
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "prompts/get", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "prompts/get", err.to_string()))
    }

    /// Issue a real `tools/call` for `tool` with `arguments` and return the server's result.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed, the arguments are not an object, or the
    /// request fails/times out.
    pub async fn call_tool(
        &self,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, McpError> {
        let arguments = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return Err(McpError::service(
                    &self.server_name,
                    "tools/call",
                    format!("arguments must be a JSON object, got {other}"),
                ));
            }
        };
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }

        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.call_tool(params))
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "tools/call", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "tools/call", err.to_string()))
    }

    /// Cancel the session and terminate the child and every descendant by exact PID.
    ///
    /// The descendant set is snapshotted before the session is cancelled so a grandchild cannot be
    /// orphaned by the parent dying first; the whole tree is then `SIGKILL`ed by PID. A remote
    /// session has no child, so only the transport is closed.
    pub async fn shutdown(&self) {
        let tree = self.pid.map(crate::process_tree::process_tree);

        {
            let mut guard = self.service.lock().await;
            if let Some(service) = guard.take() {
                if let Err(err) = service.cancel().await {
                    tracing::warn!(
                        target: "cortex::events",
                        server = %self.server_name,
                        "failed to cancel MCP session cleanly: {err}"
                    );
                }
            }
        }

        if let Some(pids) = tree {
            crate::process_tree::kill_all(&pids).await;
        }
    }
}

/// The inherited environment passed to a stdio MCP child: ordinary host variables (notably PATH)
/// are preserved for command resolution, while credential-bearing variables are removed before the
/// configured per-server environment is layered on top.
fn child_env() -> Vec<(OsString, OsString)> {
    filter_secret_env(std::env::vars_os())
}

/// Strip host secrets from an environment by name and by API-key-shaped value. This only filters
/// inherited parent variables; entries explicitly configured under `mcp.<name>.environment` are
/// applied after this filter so users can intentionally pass server-specific configuration.
fn filter_secret_env<I>(vars: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    vars.into_iter()
        .filter(|(name, value)| {
            if redaction::is_secret_env_var(&name.to_string_lossy()) {
                return false;
            }
            !redaction::value_is_secret(&value.to_string_lossy())
        })
        .collect()
}

/// Kill a stdio child and every currently visible descendant after a failed initialize path, where
/// no [`McpClient`] exists yet to run the normal shutdown path.
async fn kill_spawned_tree(pid: Option<u32>) {
    if let Some(pid) = pid {
        let pids = crate::process_tree::process_tree(pid);
        crate::process_tree::kill_all(&pids).await;
    }
}

/// The classification of a remote server from a lightweight probe.
enum Probe {
    /// The server answered `401`/needs-auth; the contained challenge is its `WWW-Authenticate`.
    NeedsAuth(Option<String>),
    /// The server accepts Streamable HTTP.
    Streamable,
    /// The server rejected Streamable HTTP; try the legacy SSE transport.
    Sse,
    /// The server could not be reached at all.
    Unreachable(String),
}

/// Probe a remote MCP endpoint with a single `initialize` POST to classify how to connect.
async fn probe(url: &str, headers: &HeaderMap, timeout: Duration) -> Probe {
    let client = match reqwest::Client::builder().build() {
        Ok(client) => client,
        Err(err) => return Probe::Unreachable(err.to_string()),
    };
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "cortex", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    let request = client
        .post(url)
        .headers(headers.clone())
        .header(http::header::ACCEPT, "application/json, text/event-stream")
        .json(&body);

    let response = match tokio::time::timeout(timeout, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => return Probe::Unreachable(err.to_string()),
        Err(_) => {
            return Probe::Unreachable("the remote server did not respond in time".to_owned());
        }
    };

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        let www = response
            .headers()
            .get(http::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return Probe::NeedsAuth(www);
    }
    if status.is_success() {
        return Probe::Streamable;
    }
    // Method-not-allowed / not-found / not-implemented signal a non-streamable endpoint; fall back
    // to the legacy SSE transport. Any other non-success status also gets the SSE attempt.
    Probe::Sse
}

/// Build the HTTP header map sent with every remote request: the configured headers plus an optional
/// `Authorization: Bearer <token>`.
fn build_header_map(
    server_name: &str,
    headers: &std::collections::BTreeMap<String, String>,
    auth_header: Option<&str>,
) -> Result<HeaderMap, McpError> {
    let mut map = HeaderMap::new();
    for (key, value) in headers {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| McpError::config(server_name, format!("invalid header name `{key}`")))?;
        let value = HeaderValue::from_str(value).map_err(|_| {
            McpError::config(server_name, format!("invalid header value for `{key}`"))
        })?;
        map.insert(name, value);
    }
    if let Some(token) = auth_header {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| McpError::config(server_name, "invalid OAuth token"))?;
        map.insert(http::header::AUTHORIZATION, value);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_openrouter_key() -> String {
        format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
    }

    #[test]
    fn filters_inherited_parent_secrets_but_keeps_ordinary_env() {
        let key = fake_openrouter_key();
        let vars = vec![
            ("OPENROUTER_API_KEY".into(), key.clone().into()),
            ("ANTHROPIC_API_KEY".into(), "plain-parent-secret".into()),
            (
                "CORTEX_MCP_PARENT_SECRET".into(),
                "plain-parent-secret".into(),
            ),
            ("LEAKED_COPY".into(), key.clone().into()),
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/tmp/home".into()),
            ("CORTEX_MCP_ECHO_LOG".into(), "/tmp/echo.log".into()),
        ];

        let filtered = filter_secret_env(vars);
        let names = filtered
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        for stripped in [
            "OPENROUTER_API_KEY",
            "ANTHROPIC_API_KEY",
            "CORTEX_MCP_PARENT_SECRET",
            "LEAKED_COPY",
        ] {
            assert!(
                !names.contains(&stripped.to_owned()),
                "{stripped} must not be inherited by an MCP child"
            );
        }
        for kept in ["PATH", "HOME", "CORTEX_MCP_ECHO_LOG"] {
            assert!(
                names.contains(&kept.to_owned()),
                "{kept} should survive inherited env filtering"
            );
        }
        for (_, value) in &filtered {
            assert!(
                !value.to_string_lossy().contains("deadbeef"),
                "no key-shaped secret value survives"
            );
        }
    }
}
