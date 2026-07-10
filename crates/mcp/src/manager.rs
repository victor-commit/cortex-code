//! The MCP client manager: connect every configured server (stdio or remote), own the live
//! server-qualified tool catalogue, route a tool call to its owning server, and shut every server
//! down (terminating stdio descendants by PID).
//!
//! At session start [`McpManager::connect`] connects each enabled server independently: a
//! connection or listing failure is recorded as a failed status and never aborts the others. Remote
//! servers are reached over HTTP (Streamable HTTP with SSE fallback); a server that requires OAuth
//! and has no usable stored token is recorded as `needs_auth` rather than failed. The catalogue is
//! *live*: a server that announces a tool-list change has its slice refreshed (and an
//! `mcp.tools.changed` event published) without a restart.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use events::EventBus;
use rmcp::model::CallToolResult;

use crate::catalogue::{LiveCatalogue, McpPrompt, McpResource, McpTool};
use crate::client::McpClient;
use crate::config::{McpServerConfig, McpTransport};
use crate::credentials::CredentialStore;
use crate::error::McpError;
use crate::oauth;
use crate::result::{RelayedResult, relay, relay_get_prompt, relay_read_resource};

/// Clock skew (seconds) used to refresh an OAuth token slightly before it actually expires.
const TOKEN_EXPIRY_SKEW_SECS: u64 = 30;

/// The connection state of a configured server, as reported by `cortex mcp list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The server was reached and the handshake + tool listing succeeded.
    Connected,
    /// The server entry was disabled and never connected.
    Disabled,
    /// A remote server requires OAuth authorization before it will serve.
    NeedsAuth,
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
            ConnectionState::NeedsAuth => "needs_auth",
            ConnectionState::Failed => "failed",
        }
    }
}

/// The reported status of a configured server after a connect attempt.
#[derive(Debug, Clone)]
pub struct ServerStatus {
    /// The configured server name.
    pub name: String,
    /// The connection state.
    pub state: ConnectionState,
    /// The PID of the spawned child, when connected over stdio.
    pub pid: Option<u32>,
    /// A human-readable reason when the connection failed or needs auth.
    pub error: Option<String>,
    /// The tools the server exposed at connect time (empty unless connected).
    pub tools: Vec<McpTool>,
    /// The resources the server exposed at connect time (empty unless connected or unsupported).
    pub resources: Vec<McpResource>,
    /// The prompts the server exposed at connect time (empty unless connected or unsupported).
    pub prompts: Vec<McpPrompt>,
}

impl ServerStatus {
    /// Return the error detail safe to serialize or render on a user-facing surface.
    ///
    /// OAuth challenges are untrusted transport metadata. The connection state retains the typed
    /// `needs_auth` signal, while every public surface receives only this stable action text.
    #[must_use]
    pub fn public_error(&self) -> Option<String> {
        if self.state == ConnectionState::NeedsAuth {
            return Some(format!(
                "authorization required (run `cortex mcp auth {}`)",
                self.name
            ));
        }
        self.error.clone()
    }
}

/// Options controlling how the manager connects (event bus + credential store).
#[derive(Clone, Default)]
pub struct McpConnectOptions {
    /// The event bus on which `mcp.tools.changed` is published when a server's tool set changes.
    pub bus: Option<Arc<EventBus>>,
    /// The OAuth credential store consulted (and refreshed) for remote servers.
    pub credentials: Option<Arc<CredentialStore>>,
    /// Client roots exposed to MCP servers that request `roots/list`.
    ///
    /// Paths are normalized to absolute file URIs before connecting each server. When empty, the
    /// current working directory is used.
    pub roots: Vec<PathBuf>,
}

/// The connected MCP servers for a session and their live qualified tool catalogue.
pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
    catalogue: LiveCatalogue,
    statuses: Vec<ServerStatus>,
}

/// Own connected clients until [`McpManager`] can take responsibility for normal shutdown.
///
/// Connecting a later server awaits while earlier stdio clients are already live. If setup is
/// cancelled at that point, dropping a plain `HashMap` would only drop the client handles and could
/// orphan their descendants. This guard snapshots and kills each owned stdio tree by exact PID
/// unless ownership is explicitly transferred into the returned manager.
#[derive(Default)]
struct ConnectingClientsGuard {
    clients: HashMap<String, Arc<McpClient>>,
    transferred: bool,
}

impl ConnectingClientsGuard {
    fn insert(&mut self, name: String, client: Arc<McpClient>) {
        self.clients.insert(name, client);
    }

    fn transfer(mut self) -> HashMap<String, Arc<McpClient>> {
        self.transferred = true;
        std::mem::take(&mut self.clients)
    }
}

impl Drop for ConnectingClientsGuard {
    fn drop(&mut self) {
        if !self.transferred {
            for client in self.clients.values() {
                client.reap_tree_now();
            }
        }
    }
}

impl McpManager {
    /// Connect every enabled server in `servers` with default options (no event bus or credential
    /// store).
    pub async fn connect(servers: BTreeMap<String, McpServerConfig>) -> Self {
        Self::connect_with(servers, McpConnectOptions::default()).await
    }

    /// Connect every enabled server in `servers`, building the live qualified tool catalogue.
    ///
    /// Each server is connected independently: a failure is recorded as a status and the remaining
    /// servers still connect. Disabled entries are skipped. A remote server requiring auth with no
    /// usable token is recorded `needs_auth`.
    pub async fn connect_with(
        servers: BTreeMap<String, McpServerConfig>,
        options: McpConnectOptions,
    ) -> Self {
        let catalogue = LiveCatalogue::new(options.bus.clone());
        let roots = roots_from_options(&options.roots);
        let mut connecting_clients = ConnectingClientsGuard::default();
        let mut statuses = Vec::new();

        for (name, config) in servers {
            if !config.enabled {
                statuses.push(ServerStatus {
                    name,
                    state: ConnectionState::Disabled,
                    pid: None,
                    error: None,
                    tools: Vec::new(),
                    resources: Vec::new(),
                    prompts: Vec::new(),
                });
                continue;
            }

            let outcome = match config.transport {
                McpTransport::Local => {
                    McpClient::connect_stdio(&name, &config, &catalogue, &roots).await
                }
                McpTransport::Remote => {
                    let auth = resolve_auth(&name, &options.credentials).await;
                    McpClient::connect_remote(&name, &config, auth, &catalogue, &roots).await
                }
            };

            match outcome {
                Ok((client, server_catalogue)) => {
                    let pid = client.pid();
                    connecting_clients.insert(name.clone(), Arc::new(client));
                    statuses.push(ServerStatus {
                        name,
                        state: ConnectionState::Connected,
                        pid,
                        error: None,
                        tools: server_catalogue.tools,
                        resources: server_catalogue.resources,
                        prompts: server_catalogue.prompts,
                    });
                }
                Err(err @ McpError::NeedsAuth { .. }) => {
                    statuses.push(ServerStatus {
                        name,
                        state: ConnectionState::NeedsAuth,
                        pid: None,
                        error: Some(err.to_string()),
                        tools: Vec::new(),
                        resources: Vec::new(),
                        prompts: Vec::new(),
                    });
                }
                Err(err) => {
                    statuses.push(ServerStatus {
                        name,
                        state: ConnectionState::Failed,
                        pid: None,
                        error: Some(err.to_string()),
                        tools: Vec::new(),
                        resources: Vec::new(),
                        prompts: Vec::new(),
                    });
                }
            }
        }

        Self {
            clients: connecting_clients.transfer(),
            catalogue,
            statuses,
        }
    }

    /// The session's live qualified MCP tool catalogue (reflecting any runtime tool-set changes).
    #[must_use]
    pub fn tools(&self) -> Vec<McpTool> {
        self.catalogue.flatten_tools()
    }

    /// The session's live qualified MCP resource catalogue.
    #[must_use]
    pub fn resources(&self) -> Vec<McpResource> {
        self.catalogue.flatten_resources()
    }

    /// The session's live qualified MCP prompt catalogue.
    #[must_use]
    pub fn prompts(&self) -> Vec<McpPrompt> {
        self.catalogue.flatten_prompts()
    }

    /// The per-server connection statuses (connected/disabled/needs_auth/failed), in configured
    /// order.
    #[must_use]
    pub fn statuses(&self) -> &[ServerStatus] {
        &self.statuses
    }

    /// Whether `id` resolves to a currently-advertised MCP tool.
    #[must_use]
    pub fn has_tool(&self, id: &str) -> bool {
        self.catalogue
            .flatten_tools()
            .iter()
            .any(|tool| tool.id == id)
    }

    /// Return the live catalogue entry for a qualified tool id.
    #[must_use]
    pub fn tool_info(&self, id: &str) -> Option<crate::catalogue::McpTool> {
        self.catalogue
            .flatten_tools()
            .into_iter()
            .find(|tool| tool.id == id)
    }

    /// Whether `id` resolves to a currently-advertised MCP resource.
    #[must_use]
    pub fn has_resource(&self, id: &str) -> bool {
        self.catalogue
            .flatten_resources()
            .iter()
            .any(|resource| resource.id == id || resource.uri == id)
    }

    /// Whether `id` resolves to a currently-advertised MCP prompt.
    #[must_use]
    pub fn has_prompt(&self, id: &str) -> bool {
        self.catalogue
            .flatten_prompts()
            .iter()
            .any(|prompt| prompt.id == id || prompt.name == id)
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
        let tools = self.catalogue.flatten_tools();
        let tool = tools
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

    /// Perform a real `resources/read` for the qualified resource `id` and relay the content.
    ///
    /// # Errors
    /// Returns [`McpError`] when `id` does not resolve, its server is not connected, or the read
    /// fails/times out.
    pub async fn read_resource(&self, id: &str) -> Result<RelayedResult, McpError> {
        let resources = self.catalogue.flatten_resources();
        let resource = resources
            .iter()
            .find(|resource| resource.id == id || resource.uri == id)
            .ok_or_else(|| McpError::unknown_resource(id))?;
        let client = self
            .clients
            .get(&resource.server)
            .ok_or_else(|| McpError::closed(&resource.server))?;
        let result = client.read_resource(&resource.uri).await?;
        Ok(relay_read_resource(&result))
    }

    /// Perform a real `prompts/get` for the qualified prompt `id` with `arguments` and relay the
    /// returned prompt messages.
    ///
    /// # Errors
    /// Returns [`McpError`] when `id` does not resolve, its server is not connected, or the get
    /// fails/times out.
    pub async fn get_prompt(
        &self,
        id: &str,
        arguments: serde_json::Value,
    ) -> Result<RelayedResult, McpError> {
        let prompts = self.catalogue.flatten_prompts();
        let prompt = prompts
            .iter()
            .find(|prompt| prompt.id == id || prompt.name == id)
            .ok_or_else(|| McpError::unknown_prompt(id))?;
        let client = self
            .clients
            .get(&prompt.server)
            .ok_or_else(|| McpError::closed(&prompt.server))?;
        let result = client.get_prompt(&prompt.name, arguments).await?;
        Ok(relay_get_prompt(&result))
    }

    /// Terminate every connected server (cancelling its session and killing stdio descendants by
    /// PID).
    pub async fn shutdown(&self) {
        for client in self.clients.values() {
            client.shutdown().await;
        }
    }
}

fn roots_from_options(configured: &[PathBuf]) -> Vec<rmcp::model::Root> {
    let mut roots: Vec<PathBuf> = if configured.is_empty() {
        std::env::current_dir().ok().into_iter().collect()
    } else {
        configured.to_vec()
    };
    if roots.is_empty() {
        return Vec::new();
    }

    roots.sort();
    roots.dedup();

    let mut uris = std::collections::BTreeSet::new();
    let mut resolved = Vec::new();
    for root in roots {
        if let Some(uri) = to_root_uri(&root)
            && uris.insert(uri.clone())
        {
            let mut entry = rmcp::model::Root::new(uri);
            if let Some(name) = root_name(&root) {
                entry = entry.with_name(name);
            }
            resolved.push(entry);
        }
    }
    resolved
}

fn to_root_uri(path: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let canonical = std::fs::canonicalize(&absolute).unwrap_or(absolute);
    url::Url::from_file_path(canonical)
        .ok()
        .map(|uri| uri.to_string())
}

fn root_name(path: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    absolute
        .file_name()
        .map(|segment| segment.to_string_lossy().to_string())
        .filter(|segment| !segment.is_empty())
}

/// Resolve the OAuth bearer token for a remote server from the credential store, refreshing it in
/// place when it is expired. Returns `None` when no usable token is available (the connect attempt
/// then surfaces `needs_auth` if the server requires it).
async fn resolve_auth(server: &str, credentials: &Option<Arc<CredentialStore>>) -> Option<String> {
    let store = credentials.as_ref()?;
    let stored = store.get(server).ok().flatten()?;
    if !stored.is_expired(TOKEN_EXPIRY_SKEW_SECS) {
        return Some(stored.access_token);
    }
    // Expired: refresh if possible, persisting the new credential so the next connect reuses it.
    if stored.refresh_token.is_some() {
        match oauth::refresh(&stored).await {
            Ok(refreshed) => {
                let token = refreshed.access_token.clone();
                let _ = store.put(server, refreshed);
                return Some(token);
            }
            Err(err) => {
                tracing::warn!(
                    target: "cortex::events",
                    server = %server,
                    "failed to refresh MCP OAuth token: {err}"
                );
            }
        }
    }
    None
}
