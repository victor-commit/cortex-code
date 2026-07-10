//! The live, server-qualified MCP catalogue shared across the manager and notification handlers.
//!
//! Each connected server's advertised tools, resources, and prompts are projected into
//! deterministic, sanitized, server-qualified ids (see [`crate::qualify`]) and stored under the
//! server name. Runtime list-changed notifications replace the affected server slice and publish a
//! typed event so subscribers can refresh without restarting.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, RwLock};

use events::{Event, EventBus};
use rmcp::model::{Prompt, Resource, Tool};

use crate::qualify;

/// One MCP tool exposed in the session catalogue under its server-qualified id.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// The one typed destination for a model-visible capability id.
///
/// IDs are never dispatched by probing capability kinds in a precedence order. Instead, the
/// catalogue allocates one id for one route and both headless and TUI consumers resolve that route
/// exactly once before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityRoute {
    /// An id reserved for a built-in tool. MCP must never claim this id.
    Builtin,
    /// A real `tools/call` on one connected MCP server.
    McpTool {
        /// The configured MCP server name.
        server: String,
        /// The raw tool name that must be sent to `tools/call`.
        tool: String,
    },
    /// A real `resources/read` on one connected MCP server.
    McpResource {
        /// The configured MCP server name.
        server: String,
        /// The raw resource URI that must be sent to `resources/read`.
        uri: String,
    },
    /// A real `prompts/get` on one connected MCP server.
    McpPrompt {
        /// The configured MCP server name.
        server: String,
        /// The raw prompt name that must be sent to `prompts/get`.
        name: String,
    },
}

impl CapabilityRoute {
    /// Whether this route is backed by an MCP server rather than a reserved built-in id.
    #[must_use]
    pub const fn is_mcp(&self) -> bool {
        !matches!(self, Self::Builtin)
    }
}

/// One MCP capability projected into a model tool definition.
///
/// This shared materialization record supplies headless and TUI definitions, while
/// [`CapabilityRoute`] remains in the manager's one route index and is resolved only at dispatch.
#[derive(Debug, Clone)]
pub struct McpCapability {
    /// The unique model-visible tool id.
    pub id: String,
    /// Model-facing description.
    pub description: String,
    /// Model-facing JSON Schema parameters.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CapabilityKind {
    Tool,
    Resource,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CapabilityIdentity {
    kind: CapabilityKind,
    server: String,
    target: String,
}

impl CapabilityIdentity {
    fn tool(server: &str, tool: &str) -> Self {
        Self {
            kind: CapabilityKind::Tool,
            server: server.to_owned(),
            target: tool.to_owned(),
        }
    }

    fn resource(server: &str, uri: &str) -> Self {
        Self {
            kind: CapabilityKind::Resource,
            server: server.to_owned(),
            target: uri.to_owned(),
        }
    }

    fn prompt(server: &str, name: &str) -> Self {
        Self {
            kind: CapabilityKind::Prompt,
            server: server.to_owned(),
            target: name.to_owned(),
        }
    }
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

/// Project a server's advertised `rmcp` tools into raw server-qualified catalogue entries.
///
/// These preliminary ids are candidates only. [`LiveCatalogue`] is the sole global allocator and
/// replaces them atomically after it sees every capability kind plus reserved built-in ids.
#[must_use]
pub fn qualify_tools(server: &str, advertised: Vec<Tool>) -> Vec<McpTool> {
    advertised
        .into_iter()
        .map(|tool| McpTool {
            id: qualify::qualified_id(server, &tool.name),
            server: server.to_owned(),
            tool: tool.name.to_string(),
            description: tool.description.map(|d| d.to_string()),
            input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
        })
        .collect()
}

/// Project a server's advertised resources into raw server-qualified resource entries. The
/// advertised URI and name are preserved verbatim on the entry so user-facing listings match the
/// server. Final ids are allocated by [`LiveCatalogue`].
#[must_use]
pub fn qualify_resources(server: &str, advertised: Vec<Resource>) -> Vec<McpResource> {
    advertised
        .into_iter()
        .map(|resource| McpResource {
            id: qualify::qualified_id(server, &format!("resource_{}", resource.uri)),
            server: server.to_owned(),
            uri: resource.uri.clone(),
            name: resource.name.clone(),
            title: resource.title.clone(),
            description: resource.description.clone(),
            mime_type: resource.mime_type.clone(),
        })
        .collect()
}

/// Project a server's advertised prompts into raw server-qualified prompt entries. The prompt name
/// and argument metadata are preserved verbatim for `prompts/get` and CLI output. Final ids are
/// allocated by [`LiveCatalogue`].
#[must_use]
pub fn qualify_prompts(server: &str, advertised: Vec<Prompt>) -> Vec<McpPrompt> {
    advertised
        .into_iter()
        .map(|prompt| {
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
                id: qualify::qualified_id(server, &format!("prompt_{}", prompt.name)),
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
    state: RwLock<CatalogueState>,
    bus: Option<Arc<EventBus>>,
}

struct CatalogueState {
    tools: BTreeMap<String, Vec<McpTool>>,
    resources: BTreeMap<String, Vec<McpResource>>,
    prompts: BTreeMap<String, Vec<McpPrompt>>,
    /// Every id ever assigned in this manager lifetime, including removed capabilities. Retaining
    /// tombstones prevents a live update from silently retargeting a previously advertised id.
    assignments: BTreeMap<CapabilityIdentity, String>,
    /// The current exact route for each model-visible id, including reserved built-ins.
    routes: BTreeMap<String, CapabilityRoute>,
    builtin_ids: BTreeSet<String>,
}

impl LiveCatalogue {
    /// Create an empty catalogue. Call [`Self::with_reserved_ids`] when a surface knows its built-in
    /// model tool ids before connecting MCP servers.
    #[must_use]
    pub fn new(bus: Option<Arc<EventBus>>) -> Self {
        Self::with_reserved_ids(bus, std::iter::empty::<String>())
    }

    /// Create an empty catalogue with model-visible built-in ids reserved before any MCP capability
    /// is assigned. The reservation set is also exposed through [`Self::route_for_id`] as
    /// [`CapabilityRoute::Builtin`].
    #[must_use]
    pub fn with_reserved_ids(
        bus: Option<Arc<EventBus>>,
        builtin_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        let builtin_ids = builtin_ids.into_iter().collect::<BTreeSet<_>>();
        let routes = builtin_ids
            .iter()
            .map(|id| (id.clone(), CapabilityRoute::Builtin))
            .collect();
        Self {
            inner: Arc::new(Inner {
                state: RwLock::new(CatalogueState {
                    tools: BTreeMap::new(),
                    resources: BTreeMap::new(),
                    prompts: BTreeMap::new(),
                    assignments: BTreeMap::new(),
                    routes,
                    builtin_ids,
                }),
                bus,
            }),
        }
    }

    /// Record a server's full catalogue at connect time (no event is published for initial
    /// population) and return the globally allocated entries for that server.
    pub fn set_initial(&self, server: &str, catalogue: McpServerCatalogue) -> McpServerCatalogue {
        let mut state = self
            .inner
            .state
            .write()
            .expect("catalogue lock not poisoned");
        state.tools.insert(
            server.to_owned(),
            dedup_by_identity(catalogue.tools, |tool| {
                CapabilityIdentity::tool(server, &tool.tool)
            }),
        );
        state.resources.insert(
            server.to_owned(),
            dedup_by_identity(catalogue.resources, |resource| {
                CapabilityIdentity::resource(server, &resource.uri)
            }),
        );
        state.prompts.insert(
            server.to_owned(),
            dedup_by_identity(catalogue.prompts, |prompt| {
                CapabilityIdentity::prompt(server, &prompt.name)
            }),
        );
        Self::reallocate(&mut state);
        server_catalogue(&state, server)
    }

    /// Replace a server's tools after a runtime tool-list-changed notification and publish an
    /// `mcp.tools.changed` event naming the server and its new tool ids.
    pub fn update_tools_from_changed(&self, server: &str, tools: Vec<McpTool>) {
        let ids = {
            let mut state = self
                .inner
                .state
                .write()
                .expect("catalogue lock not poisoned");
            let tools =
                dedup_by_identity(tools, |tool| CapabilityIdentity::tool(server, &tool.tool));
            if !same_tools(state.tools.get(server).map(Vec::as_slice), &tools) {
                state.tools.insert(server.to_owned(), tools);
                Self::reallocate(&mut state);
            }
            state
                .tools
                .get(server)
                .into_iter()
                .flatten()
                .map(|tool| tool.id.clone())
                .collect()
        };
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_tools_changed(server, ids));
        }
    }

    /// Replace a server's resources after a runtime resources-list-changed notification and publish
    /// an `mcp.resources.changed` event naming the advertised URIs.
    pub fn update_resources_from_changed(&self, server: &str, resources: Vec<McpResource>) {
        let uris = {
            let mut state = self
                .inner
                .state
                .write()
                .expect("catalogue lock not poisoned");
            let resources = dedup_by_identity(resources, |resource| {
                CapabilityIdentity::resource(server, &resource.uri)
            });
            if !same_resources(state.resources.get(server).map(Vec::as_slice), &resources) {
                state.resources.insert(server.to_owned(), resources);
                Self::reallocate(&mut state);
            }
            state
                .resources
                .get(server)
                .into_iter()
                .flatten()
                .map(|resource| resource.uri.clone())
                .collect()
        };
        if let Some(bus) = &self.inner.bus {
            bus.publish(&Event::mcp_resources_changed(server, uris));
        }
    }

    /// Replace a server's prompts after a runtime prompts-list-changed notification and publish an
    /// `mcp.prompts.changed` event naming the advertised prompt names.
    pub fn update_prompts_from_changed(&self, server: &str, prompts: Vec<McpPrompt>) {
        let names = {
            let mut state = self
                .inner
                .state
                .write()
                .expect("catalogue lock not poisoned");
            let prompts = dedup_by_identity(prompts, |prompt| {
                CapabilityIdentity::prompt(server, &prompt.name)
            });
            if !same_prompts(state.prompts.get(server).map(Vec::as_slice), &prompts) {
                state.prompts.insert(server.to_owned(), prompts);
                Self::reallocate(&mut state);
            }
            state
                .prompts
                .get(server)
                .into_iter()
                .flatten()
                .map(|prompt| prompt.name.clone())
                .collect()
        };
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
            .state
            .read()
            .expect("catalogue lock not poisoned")
            .tools
            .values()
            .flat_map(|tools| tools.iter().cloned())
            .collect()
    }

    /// The current flattened resource catalogue across every server, in stable order.
    #[must_use]
    pub fn flatten_resources(&self) -> Vec<McpResource> {
        self.inner
            .state
            .read()
            .expect("catalogue lock not poisoned")
            .resources
            .values()
            .flat_map(|resources| resources.iter().cloned())
            .collect()
    }

    /// The current flattened prompt catalogue across every server, in stable order.
    #[must_use]
    pub fn flatten_prompts(&self) -> Vec<McpPrompt> {
        self.inner
            .state
            .read()
            .expect("catalogue lock not poisoned")
            .prompts
            .values()
            .flat_map(|prompts| prompts.iter().cloned())
            .collect()
    }

    /// Resolve one id through the single typed route index.
    #[must_use]
    pub fn route_for_id(&self, id: &str) -> Option<CapabilityRoute> {
        self.inner
            .state
            .read()
            .expect("catalogue lock not poisoned")
            .routes
            .get(id)
            .cloned()
    }

    /// Return all currently advertised MCP capabilities as shared model materialization records.
    #[must_use]
    pub fn capabilities(&self) -> Vec<McpCapability> {
        let state = self
            .inner
            .state
            .read()
            .expect("catalogue lock not poisoned");
        let mut capabilities = Vec::new();
        for tools in state.tools.values() {
            capabilities.extend(tools.iter().map(|tool| McpCapability {
                id: tool.id.clone(),
                description: tool.description.clone().unwrap_or_default(),
                parameters: tool.input_schema.clone(),
            }));
        }
        for resources in state.resources.values() {
            capabilities.extend(resources.iter().map(|resource| McpCapability {
                id: resource.id.clone(),
                description: resource.description.clone().unwrap_or_else(|| {
                    format!(
                        "Read MCP resource `{}` from server `{}`.",
                        resource.uri, resource.server
                    )
                }),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            }));
        }
        for prompts in state.prompts.values() {
            capabilities.extend(prompts.iter().map(|prompt| {
                let properties = prompt
                    .arguments
                    .iter()
                    .map(|argument| {
                        let mut schema = serde_json::Map::new();
                        schema.insert(
                            "type".to_owned(),
                            serde_json::Value::String("string".to_owned()),
                        );
                        if let Some(description) = &argument.description {
                            schema.insert(
                                "description".to_owned(),
                                serde_json::Value::String(description.clone()),
                            );
                        }
                        (argument.name.clone(), serde_json::Value::Object(schema))
                    })
                    .collect::<serde_json::Map<_, _>>();
                let required = prompt
                    .arguments
                    .iter()
                    .filter(|argument| argument.required)
                    .map(|argument| serde_json::Value::String(argument.name.clone()))
                    .collect::<Vec<_>>();
                McpCapability {
                    id: prompt.id.clone(),
                    description: prompt.description.clone().unwrap_or_else(|| {
                        format!(
                            "Fetch MCP prompt `{}` from server `{}`.",
                            prompt.name, prompt.server
                        )
                    }),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": properties,
                        "required": required,
                        "additionalProperties": false
                    }),
                }
            }));
        }
        capabilities
    }

    fn reallocate(state: &mut CatalogueState) {
        let mut candidates = BTreeMap::new();
        for (server, tools) in &state.tools {
            for tool in tools {
                candidates.insert(
                    CapabilityIdentity::tool(server, &tool.tool),
                    qualify::qualified_id(server, &tool.tool),
                );
            }
        }
        for (server, resources) in &state.resources {
            for resource in resources {
                candidates.insert(
                    CapabilityIdentity::resource(server, &resource.uri),
                    qualify::qualified_id(server, &format!("resource_{}", resource.uri)),
                );
            }
        }
        for (server, prompts) in &state.prompts {
            for prompt in prompts {
                candidates.insert(
                    CapabilityIdentity::prompt(server, &prompt.name),
                    qualify::qualified_id(server, &format!("prompt_{}", prompt.name)),
                );
            }
        }

        let mut used = state.builtin_ids.iter().cloned().collect::<HashSet<_>>();
        used.extend(state.assignments.values().cloned());
        let mut pending = candidates
            .iter()
            .filter(|(identity, _)| !state.assignments.contains_key(*identity))
            .map(|(identity, candidate)| (identity.clone(), candidate.clone()))
            .collect::<Vec<_>>();
        pending.sort_by(
            |(left_identity, left_candidate), (right_identity, right_candidate)| {
                left_candidate
                    .cmp(right_candidate)
                    .then_with(|| left_identity.server.cmp(&right_identity.server))
                    .then_with(|| left_identity.target.cmp(&right_identity.target))
                    .then_with(|| left_identity.kind.cmp(&right_identity.kind))
            },
        );
        for (identity, candidate) in pending {
            let id = qualify::disambiguate(&candidate, &mut used);
            state.assignments.insert(identity, id);
        }

        state.routes = state
            .builtin_ids
            .iter()
            .map(|id| (id.clone(), CapabilityRoute::Builtin))
            .collect();

        for (server, tools) in &mut state.tools {
            for tool in tools {
                let identity = CapabilityIdentity::tool(server, &tool.tool);
                let id = state
                    .assignments
                    .get(&identity)
                    .expect("every current tool identity was allocated")
                    .clone();
                state.routes.insert(
                    id.clone(),
                    CapabilityRoute::McpTool {
                        server: server.clone(),
                        tool: tool.tool.clone(),
                    },
                );
                tool.id = id;
            }
        }
        for (server, resources) in &mut state.resources {
            for resource in resources {
                let identity = CapabilityIdentity::resource(server, &resource.uri);
                let id = state
                    .assignments
                    .get(&identity)
                    .expect("every current resource identity was allocated")
                    .clone();
                state.routes.insert(
                    id.clone(),
                    CapabilityRoute::McpResource {
                        server: server.clone(),
                        uri: resource.uri.clone(),
                    },
                );
                resource.id = id;
            }
        }
        for (server, prompts) in &mut state.prompts {
            for prompt in prompts {
                let identity = CapabilityIdentity::prompt(server, &prompt.name);
                let id = state
                    .assignments
                    .get(&identity)
                    .expect("every current prompt identity was allocated")
                    .clone();
                state.routes.insert(
                    id.clone(),
                    CapabilityRoute::McpPrompt {
                        server: server.clone(),
                        name: prompt.name.clone(),
                    },
                );
                prompt.id = id;
            }
        }
    }
}

fn server_catalogue(state: &CatalogueState, server: &str) -> McpServerCatalogue {
    McpServerCatalogue {
        tools: state.tools.get(server).cloned().unwrap_or_default(),
        resources: state.resources.get(server).cloned().unwrap_or_default(),
        prompts: state.prompts.get(server).cloned().unwrap_or_default(),
    }
}

fn dedup_by_identity<T>(entries: Vec<T>, identity: impl Fn(&T) -> CapabilityIdentity) -> Vec<T> {
    let mut seen = BTreeSet::new();
    entries
        .into_iter()
        .filter(|entry| seen.insert(identity(entry)))
        .collect()
}

fn same_tools(current: Option<&[McpTool]>, incoming: &[McpTool]) -> bool {
    current.is_some_and(|current| {
        current.len() == incoming.len()
            && current.iter().zip(incoming).all(|(left, right)| {
                left.server == right.server
                    && left.tool == right.tool
                    && left.description == right.description
                    && left.input_schema == right.input_schema
            })
    })
}

fn same_resources(current: Option<&[McpResource]>, incoming: &[McpResource]) -> bool {
    current.is_some_and(|current| {
        current.len() == incoming.len()
            && current.iter().zip(incoming).all(|(left, right)| {
                left.server == right.server
                    && left.uri == right.uri
                    && left.name == right.name
                    && left.title == right.title
                    && left.description == right.description
                    && left.mime_type == right.mime_type
            })
    })
}

fn same_prompts(current: Option<&[McpPrompt]>, incoming: &[McpPrompt]) -> bool {
    current.is_some_and(|current| {
        current.len() == incoming.len()
            && current.iter().zip(incoming).all(|(left, right)| {
                left.server == right.server
                    && left.name == right.name
                    && left.title == right.title
                    && left.description == right.description
                    && left.arguments == right.arguments
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, tool: &str) -> McpTool {
        McpTool {
            id: id.to_owned(),
            server: "demo".to_owned(),
            tool: tool.to_owned(),
            description: None,
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    fn resource(id: &str, uri: &str) -> McpResource {
        McpResource {
            id: id.to_owned(),
            server: "demo".to_owned(),
            uri: uri.to_owned(),
            name: "notes".to_owned(),
            title: None,
            description: None,
            mime_type: None,
        }
    }

    fn prompt(id: &str, name: &str) -> McpPrompt {
        McpPrompt {
            id: id.to_owned(),
            server: "demo".to_owned(),
            name: name.to_owned(),
            title: None,
            description: None,
            arguments: Vec::new(),
        }
    }

    #[test]
    fn globally_allocates_unique_ids_across_mcp_capability_kinds() {
        let catalogue = LiveCatalogue::new(None);
        catalogue.set_initial(
            "demo",
            McpServerCatalogue {
                tools: vec![tool("demo_resource_notes___demo", "resource_notes://demo")],
                resources: vec![resource("demo_resource_notes___demo", "notes://demo")],
                prompts: vec![prompt(
                    "demo_resource_notes___demo",
                    "resource_notes://demo",
                )],
            },
        );

        let mut ids = catalogue
            .flatten_tools()
            .into_iter()
            .map(|entry| entry.id)
            .chain(
                catalogue
                    .flatten_resources()
                    .into_iter()
                    .map(|entry| entry.id),
            )
            .chain(
                catalogue
                    .flatten_prompts()
                    .into_iter()
                    .map(|entry| entry.id),
            )
            .collect::<Vec<_>>();
        let total = ids.len();
        ids.sort();
        ids.dedup();

        assert_eq!(
            ids.len(),
            total,
            "every advertised MCP capability must own a unique model-visible id"
        );
    }
}
