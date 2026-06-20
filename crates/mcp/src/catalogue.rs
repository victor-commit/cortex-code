//! The live, server-qualified MCP tool catalogue shared across the manager and the per-connection
//! notification handlers.
//!
//! Each connected server's advertised tools are projected into deterministic, sanitized,
//! server-qualified ids (see [`crate::qualify`]) and stored under the server name. When a server
//! sends a tool-list-changed notification its [`crate::handler`] re-lists the tools and calls
//! [`LiveCatalogue::update_from_changed`], which replaces that server's slice and publishes an
//! `mcp.tools.changed` event so subscribers (the TUI, the server SSE stream, headless event
//! consumers) reflect the new set without a restart.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use events::{Event, EventBus};
use rmcp::model::Tool;

use crate::qualify;

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

/// Project a server's advertised `rmcp` tools into the server-qualified catalogue entries,
/// disambiguating any ids that sanitize to the same string within this server.
#[must_use]
pub fn qualify_tools(server: &str, advertised: Vec<Tool>) -> Vec<McpTool> {
    let mut used = HashSet::new();
    advertised
        .into_iter()
        .map(|tool| {
            let id = qualify::disambiguate(&qualify::qualified_id(server, &tool.name), &mut used);
            McpTool {
                id,
                server: server.to_owned(),
                tool: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()),
                input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
            }
        })
        .collect()
}

/// The live tool catalogue: per-server qualified tool slices plus an optional event bus on which a
/// runtime change is announced.
#[derive(Clone)]
pub struct LiveCatalogue {
    inner: Arc<Inner>,
}

struct Inner {
    servers: RwLock<BTreeMap<String, Vec<McpTool>>>,
    bus: Option<Arc<EventBus>>,
}

impl LiveCatalogue {
    /// Create an empty catalogue. When `bus` is set, a runtime tool-set change publishes an
    /// `mcp.tools.changed` event.
    #[must_use]
    pub fn new(bus: Option<Arc<EventBus>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                servers: RwLock::new(BTreeMap::new()),
                bus,
            }),
        }
    }

    /// Record a server's tools at connect time (no event is published for the initial population).
    pub fn set_initial(&self, server: &str, tools: Vec<McpTool>) {
        self.inner
            .servers
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), tools);
    }

    /// Replace a server's tools after a runtime tool-list-changed notification and publish an
    /// `mcp.tools.changed` event naming the server and its new tool ids.
    pub fn update_from_changed(&self, server: &str, tools: Vec<McpTool>) {
        let ids: Vec<String> = tools.iter().map(|tool| tool.id.clone()).collect();
        self.inner
            .servers
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), tools);
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_tools_changed(server, ids));
        }
    }

    /// The current flattened catalogue across every server, in stable (server, then advertised)
    /// order.
    #[must_use]
    pub fn flatten(&self) -> Vec<McpTool> {
        self.inner
            .servers
            .read()
            .expect("catalogue lock not poisoned")
            .values()
            .flat_map(|tools| tools.iter().cloned())
            .collect()
    }
}
