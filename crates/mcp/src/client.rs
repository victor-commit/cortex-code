//! A single connected stdio MCP server: spawn the child, complete the initialize handshake, and
//! expose `tools/list` and `tools/call` over the running session.
//!
//! The client spawns the configured command as a child process parented under cortex, runs the
//! `rmcp` client handshake over the child's stdio, and records the child PID. The running session is
//! held behind a mutex so a call serializes against shutdown; [`shutdown`](McpClient::shutdown)
//! cancels the session, which closes the transport and terminates the child (no orphan survives).

use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::config::McpServerConfig;
use crate::error::McpError;

/// The default per-operation timeout (handshake and each request) when a server sets none.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Environment variables never forwarded into a spawned MCP child, so host secrets do not leak.
const STRIPPED_ENV: &[&str] = &["OPENROUTER_API_KEY"];

/// A connected stdio MCP server session.
pub struct McpClient {
    server_name: String,
    pid: Option<u32>,
    service: Mutex<Option<RunningService<RoleClient, ()>>>,
    timeout: Duration,
}

impl McpClient {
    /// Spawn the configured local server and complete the MCP initialize handshake.
    ///
    /// The child inherits the cortex environment minus known host secrets, with the configured
    /// `environment` overrides applied and the optional `cwd` honored. The handshake is bounded by
    /// the configured `timeout` (or [`DEFAULT_TIMEOUT`]).
    ///
    /// # Errors
    /// Returns [`McpError`] when the command is empty, the child cannot be spawned, or the handshake
    /// fails or times out.
    pub async fn connect_stdio(
        server_name: &str,
        config: &McpServerConfig,
    ) -> Result<Self, McpError> {
        let (program, args) = config
            .program_and_args()
            .ok_or_else(|| McpError::config(server_name, "the `command` array is empty"))?;

        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        for stripped in STRIPPED_ENV {
            command.env_remove(stripped);
        }
        for (key, value) in &config.environment {
            command.env(key, value);
        }

        let timeout = config
            .timeout
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TIMEOUT);

        let (transport, _stderr) = TokioChildProcess::builder(command)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|source| McpError::Spawn {
                server: server_name.to_owned(),
                program: program.to_owned(),
                source,
            })?;
        let pid = transport.id();

        let service = tokio::time::timeout(timeout, ().serve(transport))
            .await
            .map_err(|_| McpError::timeout(server_name, "initialize", timeout))?
            .map_err(|err| McpError::handshake(server_name, err.to_string()))?;

        Ok(Self {
            server_name: server_name.to_owned(),
            pid,
            service: Mutex::new(Some(service)),
            timeout,
        })
    }

    /// The PID of the spawned child process, if the OS reported one.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Issue `tools/list` (following pagination) and return the server's advertised tools.
    ///
    /// # Errors
    /// Returns [`McpError`] when the session is closed or the request fails/times out.
    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpError> {
        let guard = self.service.lock().await;
        let service = guard
            .as_ref()
            .ok_or_else(|| McpError::closed(&self.server_name))?;
        tokio::time::timeout(self.timeout, service.list_all_tools())
            .await
            .map_err(|_| McpError::timeout(&self.server_name, "tools/list", self.timeout))?
            .map_err(|err| McpError::service(&self.server_name, "tools/list", err.to_string()))
    }

    /// Issue a real `tools/call` for `tool` with `arguments` and return the server's result.
    ///
    /// `arguments` must be a JSON object (or null/empty for a no-argument tool).
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

    /// Cancel the session and terminate the child, awaiting cleanup so no orphan process survives.
    pub async fn shutdown(&self) {
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
}
