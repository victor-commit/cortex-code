//! TUI-owned MCP runtime wiring: connect configured servers once and forward live catalogue events.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use events::{Event, EventBus};

/// A connected MCP manager plus a UI-thread event stream for live catalogue changes.
pub(crate) struct McpRuntime {
    pub(crate) manager: Arc<mcp::McpManager>,
    pub(crate) events: Receiver<Event>,
    pub(crate) runtime: tokio::runtime::Runtime,
}

/// Connect configured MCP servers for the TUI process.
///
/// Returns `None` when no servers are configured or configuration cannot be resolved. Individual
/// server failures are represented in the manager's statuses and do not block the TUI.
pub(crate) fn connect_from_config() -> Option<McpRuntime> {
    let effective = effective_config()?;
    let servers = mcp_servers_from_config(&effective);
    if servers.is_empty() {
        return None;
    }

    let (tx, rx) = mpsc::channel();
    let bus = Arc::new(EventBus::new());
    bus.subscribe(Arc::new(move |event: &Event| {
        let _ = tx.send(event.clone());
    }));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .ok()?;
    let manager = runtime.block_on(mcp::McpManager::connect_with(
        servers,
        mcp::McpConnectOptions {
            bus: Some(bus),
            credentials: credential_store_from_process(),
            roots: std::env::current_dir().ok().into_iter().collect(),
        },
    ));

    Some(McpRuntime {
        manager: Arc::new(manager),
        events: rx,
        runtime,
    })
}

/// Shut down a connected MCP manager, terminating stdio descendants by exact PID.
pub(crate) fn shutdown(manager: Arc<mcp::McpManager>, runtime: tokio::runtime::Runtime) {
    runtime.block_on(manager.shutdown());
}

fn effective_config() -> Option<settings::EffectiveConfig> {
    settings::ConfigInputs::from_process(BTreeMap::new())
        .ok()
        .and_then(|inputs| settings::load(inputs).ok())
}

fn mcp_servers_from_config(
    effective: &settings::EffectiveConfig,
) -> BTreeMap<String, mcp::McpServerConfig> {
    let Some(value) = effective.get("mcp") else {
        return BTreeMap::new();
    };
    let json = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    match mcp::servers_from_value(&json) {
        Ok(servers) => servers,
        Err(err) => {
            eprintln!("cortex tui: ignoring invalid MCP configuration: {err}");
            BTreeMap::new()
        }
    }
}

fn credential_store_from_process() -> Option<Arc<mcp::CredentialStore>> {
    db::DbPathInputs::from_process()
        .resolve()
        .map(|db_path| db_path.with_file_name("mcp-oauth.json"))
        .map(mcp::CredentialStore::at)
        .map(Arc::new)
}
