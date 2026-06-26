//! The live, server-qualified MCP catalogue shared across the manager and notification handlers.
//!
//! Each connected server's advertised tools, resources, and prompts are projected into
//! deterministic, sanitized, server-qualified ids (see [`crate::qualify`]) and stored under the
//! server name. Runtime list-changed notifications replace the affected server slice and publish a
//! typed event so subscribers can refresh without restarting.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

use events::{Event, EventBus};
use rmcp::model::{Prompt, Resource, Tool};

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

/// One MCP resource exposed in the session catalogue under a server-qualified synthetic tool id.
#[derive(Debug, Clone)]
pub struct McpResource {
    /// The deterministic, sanitized id used by Cortex to refer to this resource.
    pub id: String,
    /// The server that exports the resource.
    pub server: String,
    /// The resource URI exactly as advertised by the server (used for `resources/read`).
    pub uri: String,
    /// The resource name exactly as advertised by the server.
    pub name: String,
    /// Human-readable title, if the server provided one.
    pub title: Option<String>,
    /// Human-readable description, if the server provided one.
    pub description: Option<String>,
    /// MIME type, if the server provided one.
    pub mime_type: Option<String>,
}

/// One prompt argument advertised by an MCP prompt template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPromptArgument {
    /// The argument name exactly as advertised by the server.
    pub name: String,
    /// Human-readable title, if the server provided one.
    pub title: Option<String>,
    /// Human-readable description, if the server provided one.
    pub description: Option<String>,
    /// Whether the server marks this argument as required.
    pub required: bool,
}

/// One MCP prompt exposed in the session catalogue under a server-qualified synthetic tool id.
#[derive(Debug, Clone)]
pub struct McpPrompt {
    /// The deterministic, sanitized id used by Cortex to refer to this prompt.
    pub id: String,
    /// The server that exports the prompt.
    pub server: String,
    /// The prompt name exactly as advertised by the server (used for `prompts/get`).
    pub name: String,
    /// Human-readable title, if the server provided one.
    pub title: Option<String>,
    /// Human-readable description, if the server provided one.
    pub description: Option<String>,
    /// The prompt's declared arguments.
    pub arguments: Vec<McpPromptArgument>,
}

/// The three initially listed slices returned after a server connects.
#[derive(Debug, Clone, Default)]
pub struct McpServerCatalogue {
    /// Qualified tools listed with `tools/list`.
    pub tools: Vec<McpTool>,
    /// Qualified resources listed with `resources/list`.
    pub resources: Vec<McpResource>,
    /// Qualified prompts listed with `prompts/list`.
    pub prompts: Vec<McpPrompt>,
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

/// Project a server's advertised resources into server-qualified resource ids. The advertised URI
/// and name are preserved verbatim on the entry so user-facing listings match the server.
#[must_use]
pub fn qualify_resources(server: &str, advertised: Vec<Resource>) -> Vec<McpResource> {
    let mut used = HashSet::new();
    advertised
        .into_iter()
        .map(|resource| {
            let id = qualify::disambiguate(
                &qualify::qualified_id(server, &format!("resource_{}", resource.uri)),
                &mut used,
            );
            McpResource {
                id,
                server: server.to_owned(),
                uri: resource.uri.clone(),
                name: resource.name.clone(),
                title: resource.title.clone(),
                description: resource.description.clone(),
                mime_type: resource.mime_type.clone(),
            }
        })
        .collect()
}

/// Project a server's advertised prompts into server-qualified prompt ids. The prompt name and
/// argument metadata are preserved verbatim for `prompts/get` and CLI output.
#[must_use]
pub fn qualify_prompts(server: &str, advertised: Vec<Prompt>) -> Vec<McpPrompt> {
    let mut used = HashSet::new();
    advertised
        .into_iter()
        .map(|prompt| {
            let id = qualify::disambiguate(
                &qualify::qualified_id(server, &format!("prompt_{}", prompt.name)),
                &mut used,
            );
            let arguments = prompt
                .arguments
                .unwrap_or_default()
                .into_iter()
                .map(|argument| McpPromptArgument {
                    name: argument.name,
                    title: argument.title,
                    description: argument.description,
                    required: argument.required.unwrap_or(false),
                })
                .collect();
            McpPrompt {
                id,
                server: server.to_owned(),
                name: prompt.name,
                title: prompt.title,
                description: prompt.description,
                arguments,
            }
        })
        .collect()
}

/// The live catalogue: per-server qualified slices plus an optional event bus on which runtime
/// changes are announced.
#[derive(Clone)]
pub struct LiveCatalogue {
    inner: Arc<Inner>,
}

struct Inner {
    tools: RwLock<BTreeMap<String, Vec<McpTool>>>,
    resources: RwLock<BTreeMap<String, Vec<McpResource>>>,
    prompts: RwLock<BTreeMap<String, Vec<McpPrompt>>>,
    bus: Option<Arc<EventBus>>,
}

impl LiveCatalogue {
    /// Create an empty catalogue. When `bus` is set, runtime list changes publish typed events.
    #[must_use]
    pub fn new(bus: Option<Arc<EventBus>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                tools: RwLock::new(BTreeMap::new()),
                resources: RwLock::new(BTreeMap::new()),
                prompts: RwLock::new(BTreeMap::new()),
                bus,
            }),
        }
    }

    /// Record a server's full catalogue at connect time (no event is published for initial
    /// population).
    pub fn set_initial(&self, server: &str, catalogue: McpServerCatalogue) {
        self.inner
            .tools
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), catalogue.tools);
        self.inner
            .resources
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), catalogue.resources);
        self.inner
            .prompts
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), catalogue.prompts);
    }

    /// Replace a server's tools after a runtime tool-list-changed notification and publish an
    /// `mcp.tools.changed` event naming the server and its new tool ids.
    pub fn update_tools_from_changed(&self, server: &str, tools: Vec<McpTool>) {
        let ids: Vec<String> = tools.iter().map(|tool| tool.id.clone()).collect();
        self.inner
            .tools
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), tools);
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_tools_changed(server, ids));
        }
    }

    /// Replace a server's resources after a runtime resources-list-changed notification and publish
    /// an `mcp.resources.changed` event naming the advertised URIs.
    pub fn update_resources_from_changed(&self, server: &str, resources: Vec<McpResource>) {
        let uris: Vec<String> = resources
            .iter()
            .map(|resource| resource.uri.clone())
            .collect();
        self.inner
            .resources
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), resources);
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_resources_changed(server, uris));
        }
    }

    /// Replace a server's prompts after a runtime prompts-list-changed notification and publish an
    /// `mcp.prompts.changed` event naming the advertised prompt names.
    pub fn update_prompts_from_changed(&self, server: &str, prompts: Vec<McpPrompt>) {
        let names: Vec<String> = prompts.iter().map(|prompt| prompt.name.clone()).collect();
        self.inner
            .prompts
            .write()
            .expect("catalogue lock not poisoned")
            .insert(server.to_owned(), prompts);
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_prompts_changed(server, names));
        }
    }

    /// Publish one redacted `mcp.log` message from a server notification, when an event bus is
    /// attached.
    pub fn publish_log_message(
        &self,
        server: &str,
        level: &str,
        logger: Option<String>,
        data: serde_json::Value,
    ) {
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_log_message(
                server.to_owned(),
                level.to_owned(),
                logger,
                data,
            ));
        }
    }

    /// The current flattened tool catalogue across every server, in stable (server, then
    /// advertised) order.
    #[must_use]
    pub fn flatten_tools(&self) -> Vec<McpTool> {
        self.inner
            .tools
            .read()
            .expect("catalogue lock not poisoned")
            .values()
            .flat_map(|tools| tools.iter().cloned())
            .collect()
    }

    /// The current flattened resource catalogue across every server, in stable order.
    #[must_use]
    pub fn flatten_resources(&self) -> Vec<McpResource> {
        self.inner
            .resources
            .read()
            .expect("catalogue lock not poisoned")
            .values()
            .flat_map(|resources| resources.iter().cloned())
            .collect()
    }

    /// The current flattened prompt catalogue across every server, in stable order.
    #[must_use]
    pub fn flatten_prompts(&self) -> Vec<McpPrompt> {
        self.inner
            .prompts
            .read()
            .expect("catalogue lock not poisoned")
            .values()
            .flat_map(|prompts| prompts.iter().cloned())
            .collect()
    }
}
