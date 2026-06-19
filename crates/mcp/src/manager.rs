//! The MCP client manager: connect every configured server, build the server-qualified tool
//! catalogue for the session, route a tool call to its owning server, and shut every server down.
//!
//! At session start [`McpManager::connect`] spawns and handshakes each enabled local server,
//! issues `tools/list`, and registers every advertised tool under a deterministic server-qualified
//! id (see [`crate::qualify`]). A connection or listing failure is recorded as a failed status and
//! never aborts the others, so one broken server does not take down the rest. The manager owns the
//! connected clients and terminates them all on [`shutdown`](McpManager::shutdown).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rmcp::model::CallToolResult;

use crate::client::McpClient;
use crate::config::{McpServerConfig, McpTransport};
use crate::error::McpError;
use crate::qualify;
use crate::result::{RelayedResult, relay};

/// The connection state of a configured server, as reported by `cortex mcp list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The server was spawned and the handshake + tool listing succeeded.
    Connected,
    /// The server entry was disabled and never spawned.
    Disabled,
    /// The server could not be connected (spawn/handshake/listing failed).
    Failed,
}

impl ConnectionState {
    /// The stable lowercase label for this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ConnectionState::Connected => "connected",
            ConnectionState::Disabled => "disabled",
            ConnectionState::Failed => "failed",
        }
    }
}

/// One MCP tool exposed in the session catalogue under its server-qualified id.
#[derive(Debug, Clone)]
pub struct McpTool {
    /// The deterministic, sanitized, server-qualified id the model calls (e.g. `demo_add`).
    pub id: String,
    /// The server that exports the tool.
    pub server: String,
    /// The tool's own name as advertised by the server (used for the `tools/call`).
    pub tool: String,
    /// The tool's model-facing description, if the server provided one.
    pub description: Option<String>,
    /// The tool's JSON Schema input parameters.
    pub input_schema: serde_json::Value,
}

/// The reported status of a configured server after a connect attempt.
#[derive(Debug, Clone)]
pub struct ServerStatus {
    /// The configured server name.
    pub name: String,
    /// The connection state.
    pub state: ConnectionState,
    /// The PID of the spawned child, when connected.
    pub pid: Option<u32>,
    /// A human-readable reason when the connection failed.
    pub error: Option<String>,
    /// The tools the server exposed (empty unless connected).
    pub tools: Vec<McpTool>,
}

/// The connected MCP servers for a session and their qualified tool catalogue.
pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
    tools: Vec<McpTool>,
    statuses: Vec<ServerStatus>,
}

impl McpManager {
    /// Connect every enabled local server in `servers`, building the qualified tool catalogue.
    ///
    /// Each server is connected independently: a spawn/handshake/listing failure is recorded as a
    /// [`ConnectionState::Failed`] status and the remaining servers still connect. Disabled entries
    /// are skipped. Remote entries are recorded as failed with a clear note until the remote
    /// transport is wired in by a later milestone.
    pub async fn connect(servers: BTreeMap<String, McpServerConfig>) -> Self {
        let mut clients = HashMap::new();
        let mut tools = Vec::new();
        let mut statuses = Vec::new();
        let mut used_ids: HashSet<String> = HashSet::new();

        for (name, config) in servers {
            if !config.enabled {
                statuses.push(ServerStatus {
                    name,
                    state: ConnectionState::Disabled,
                    pid: None,
                    error: None,
                    tools: Vec::new(),
                });
                continue;
            }

            if config.transport != McpTransport::Local {
                statuses.push(ServerStatus {
                    name,
                    state: ConnectionState::Failed,
                    pid: None,
                    error: Some("remote MCP transport is not supported yet".to_owned()),
                    tools: Vec::new(),
                });
                continue;
            }

            match Self::connect_one(&name, &config, &mut used_ids).await {
                Ok((client, server_tools)) => {
                    let pid = client.pid();
                    clients.insert(name.clone(), Arc::new(client));
                    tools.extend(server_tools.iter().cloned());
                    statuses.push(ServerStatus {
                        name,
                        state: ConnectionState::Connected,
                        pid,
                        error: None,
                        tools: server_tools,
                    });
                }
                Err(err) => {
                    statuses.push(ServerStatus {
                        name,
                        state: ConnectionState::Failed,
                        pid: None,
                        error: Some(err.to_string()),
                        tools: Vec::new(),
                    });
                }
            }
        }

        Self {
            clients,
            tools,
            statuses,
        }
    }

    /// Connect one server and project its advertised tools into the qualified catalogue.
    async fn connect_one(
        name: &str,
        config: &McpServerConfig,
        used_ids: &mut HashSet<String>,
    ) -> Result<(McpClient, Vec<McpTool>), McpError> {
        let client = McpClient::connect_stdio(name, config).await?;
        let advertised = match client.list_tools().await {
            Ok(advertised) => advertised,
            Err(err) => {
                client.shutdown().await;
                return Err(err);
            }
        };

        let tools = advertised
            .into_iter()
            .map(|tool| {
                let id = qualify::disambiguate(&qualify::qualified_id(name, &tool.name), used_ids);
                McpTool {
                    id,
                    server: name.to_owned(),
                    tool: tool.name.to_string(),
                    description: tool.description.map(|d| d.to_string()),
                    input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
                }
            })
            .collect();

        Ok((client, tools))
    }

    /// The session's qualified MCP tool catalogue, in stable order.
    #[must_use]
    pub fn tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// The per-server connection statuses (connected/disabled/failed), in configured order.
    #[must_use]
    pub fn statuses(&self) -> &[ServerStatus] {
        &self.statuses
    }

    /// Whether `id` resolves to a connected MCP tool.
    #[must_use]
    pub fn has_tool(&self, id: &str) -> bool {
        self.tools.iter().any(|tool| tool.id == id)
    }

    /// Perform a real `tools/call` for the qualified tool `id` and relay the server's result.
    ///
    /// # Errors
    /// Returns [`McpError`] when `id` does not resolve, its server is not connected, or the call
    /// fails/times out.
    pub async fn call(
        &self,
        id: &str,
        arguments: serde_json::Value,
    ) -> Result<RelayedResult, McpError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.id == id)
            .ok_or_else(|| McpError::unknown_tool(id))?;
        let client = self
            .clients
            .get(&tool.server)
            .ok_or_else(|| McpError::closed(&tool.server))?;
        let result: CallToolResult = client.call_tool(&tool.tool, arguments).await?;
        Ok(relay(&result))
    }

    /// Terminate every connected server (by cancelling its session, which kills the child).
    pub async fn shutdown(&self) {
        for client in self.clients.values() {
            client.shutdown().await;
        }
    }
}
