//! MCP catalog route that exposes the same MCP servers, resources, and prompts as the CLI and TUI
//! surfaces.
//!
//! `/mcp` lists every configured MCP server with its connection status, and includes the flattened
//! resources and prompts advertised across all connected servers. The route merges server status
//! from a fresh connect with the project config, so a server client sees the same MCP topology as
//! `cortex mcp list` and the TUI `/mcp` view. Tools are intentionally omitted because they are
//! already surfaced through the main tool catalog and session dispatcher.
//!
//! Connecting publishes the live `mcp.tools.changed`/`mcp.resources.changed`/`mcp.prompts.changed`
//! catalogue events onto the server event bus, so a client watching `GET /event` observes the same
//! resource and prompt entries this route returns.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{Json, extract::State};
use events::Event;
use serde::Serialize;

use super::{ApiError, ServerState, config_api_error};

#[derive(Debug, Serialize)]
pub(super) struct McpCatalogResponse {
    servers: Vec<McpServerEntry>,
    resources: Vec<McpResourceEntry>,
    prompts: Vec<McpPromptEntry>,
}

#[derive(Debug, Serialize)]
struct McpServerEntry {
    name: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpResourceEntry {
    id: String,
    server: String,
    uri: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpPromptEntry {
    id: String,
    server: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    arguments: Vec<McpPromptArgumentEntry>,
}

#[derive(Debug, Serialize)]
struct McpPromptArgumentEntry {
    name: String,
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// `GET /mcp` — list configured MCP servers with their connection status, plus the flattened
/// resource and prompt catalogs across all connected servers.
///
/// This route connects every configured server fresh (like `cortex mcp list`), reports each
/// server's status, and flattens the advertised resources and prompts. Tools are omitted because
/// they are surfaced through the main tool catalog. The connection is ephemeral: servers are
/// shut down after listing, so no long-lived MCP children remain attached to the server process.
pub(super) async fn list_mcp(
    State(state): State<ServerState>,
) -> Result<Json<McpCatalogResponse>, ApiError> {
    let inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective = settings::load(inputs).map_err(config_api_error)?;
    let mut servers = mcp_servers_from_config(&effective);

    // Honor the network kill-switch: a remote MCP server would open a socket, so when the network
    // is disabled drop enabled remote entries before connecting and keep the route fail-closed.
    if redaction::guard_outbound("MCP remote discovery/connect").is_err() {
        servers
            .retain(|_, config| !(config.enabled && config.transport == mcp::McpTransport::Remote));
    }

    if servers.is_empty() {
        return Ok(Json(McpCatalogResponse {
            servers: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
        }));
    }

    let manager = mcp::McpManager::connect_with(
        servers,
        mcp::McpConnectOptions {
            bus: Some(state.event_bus.clone()),
            credentials: credential_store_from_state(&state),
            roots: vec![state.cwd.clone()],
        },
    )
    .await;

    let server_entries: Vec<McpServerEntry> = manager
        .statuses()
        .iter()
        .map(|status| McpServerEntry {
            name: redaction::redact(&status.name),
            status: status.state.as_str().to_owned(),
            pid: status.pid,
            error: status.public_error().map(|err| redaction::redact(&err)),
        })
        .collect();

    let resources = manager.resources();
    let prompts = manager.prompts();
    let tools = manager.tools();

    // Announce the discovered catalogues on the event bus so `GET /event` subscribers observe the
    // same resource and prompt entries this route returns (matching ids, redacted at the SSE
    // boundary). Grouped per server to mirror the live list-changed notification shape.
    publish_catalogue_events(&state, manager.statuses(), &tools, &resources, &prompts);

    let resource_entries: Vec<McpResourceEntry> = resources
        .into_iter()
        .map(|resource| McpResourceEntry {
            id: redaction::redact(&resource.id),
            server: redaction::redact(&resource.server),
            uri: redaction::redact(&resource.uri),
            name: redaction::redact(&resource.name),
            description: resource.description.map(|d| redaction::redact(&d)),
            mime_type: resource.mime_type.map(|m| redaction::redact(&m)),
        })
        .collect();

    let prompt_entries: Vec<McpPromptEntry> = prompts
        .into_iter()
        .map(|prompt| McpPromptEntry {
            id: redaction::redact(&prompt.id),
            server: redaction::redact(&prompt.server),
            name: redaction::redact(&prompt.name),
            description: prompt.description.map(|d| redaction::redact(&d)),
            arguments: prompt
                .arguments
                .into_iter()
                .map(|arg| McpPromptArgumentEntry {
                    name: redaction::redact(&arg.name),
                    required: arg.required,
                    description: arg.description.map(|d| redaction::redact(&d)),
                })
                .collect(),
        })
        .collect();

    manager.shutdown().await;

    Ok(Json(McpCatalogResponse {
        servers: server_entries,
        resources: resource_entries,
        prompts: prompt_entries,
    }))
}

/// Publish per-server catalogue events for every connected server so SSE subscribers see the same
/// tool/resource/prompt entries the route returns.
fn publish_catalogue_events(
    state: &ServerState,
    statuses: &[mcp::ServerStatus],
    tools: &[mcp::McpTool],
    resources: &[mcp::McpResource],
    prompts: &[mcp::McpPrompt],
) {
    for status in statuses {
        if status.state != mcp::ConnectionState::Connected {
            continue;
        }
        let tool_ids: Vec<String> = tools
            .iter()
            .filter(|tool| tool.server == status.name)
            .map(|tool| tool.id.clone())
            .collect();
        state
            .event_bus
            .publish(&Event::mcp_tools_changed(status.name.clone(), tool_ids));

        let resource_uris: Vec<String> = resources
            .iter()
            .filter(|resource| resource.server == status.name)
            .map(|resource| resource.uri.clone())
            .collect();
        state.event_bus.publish(&Event::mcp_resources_changed(
            status.name.clone(),
            resource_uris,
        ));

        let prompt_names: Vec<String> = prompts
            .iter()
            .filter(|prompt| prompt.server == status.name)
            .map(|prompt| prompt.name.clone())
            .collect();
        state.event_bus.publish(&Event::mcp_prompts_changed(
            status.name.clone(),
            prompt_names,
        ));
    }
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
            eprintln!("cortex serve /mcp: ignoring invalid MCP configuration: {err}");
            BTreeMap::new()
        }
    }
}

fn credential_store_from_state(state: &ServerState) -> Option<Arc<mcp::CredentialStore>> {
    state
        .db_path()
        .parent()
        .map(|parent| parent.join("mcp-oauth.json"))
        .map(mcp::CredentialStore::at)
        .map(Arc::new)
}
