//! Typed event definitions carried on the [`EventBus`](crate::EventBus).

use serde::Serialize;

/// The surface that produced an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The interactive terminal UI.
    Tui,
    /// The headless `cortex run` path.
    Run,
    /// The local loopback server.
    Serve,
    /// The configuration-reporting path.
    Config,
    /// The persisted-session management path (`cortex session`).
    Session,
    /// The outbound-network diagnostics path (`cortex net`).
    Net,
    /// The checkpoint undo/redo path (`cortex undo`/`cortex redo`).
    Checkpoint,
    /// The MCP server management path (`cortex mcp`).
    Mcp,
    /// The language-server management path (`cortex lsp`).
    Lsp,
}

impl Surface {
    /// The stable lowercase name of this surface.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Surface::Tui => "tui",
            Surface::Run => "run",
            Surface::Serve => "serve",
            Surface::Config => "config",
            Surface::Session => "session",
            Surface::Net => "net",
            Surface::Checkpoint => "checkpoint",
            Surface::Mcp => "mcp",
            Surface::Lsp => "lsp",
        }
    }
}

/// The phase of a process/surface lifecycle transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecyclePhase {
    /// A surface is starting up.
    Startup,
    /// A surface is shutting down.
    Shutdown,
}

impl LifecyclePhase {
    /// The stable lowercase name of this phase.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            LifecyclePhase::Startup => "startup",
            LifecyclePhase::Shutdown => "shutdown",
        }
    }
}

/// A process/surface lifecycle event (e.g. a surface starting up).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Lifecycle {
    /// Whether the surface is starting up or shutting down.
    pub phase: LifecyclePhase,
    /// Which surface emitted the transition.
    pub surface: Surface,
    /// The binary version at the time of the transition.
    pub version: String,
}

/// The concrete answer to a permission ask: run the action or block it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// The gated action may proceed.
    Allow,
    /// The gated action is blocked.
    Deny,
}

impl PermissionDecision {
    /// The stable lowercase name of this decision.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PermissionDecision::Allow => "allow",
            PermissionDecision::Deny => "deny",
        }
    }
}

/// A gated tool action was suspended pending a permission decision.
///
/// The payload names the tool and the pattern (the matching rule's pattern, or the action's
/// subject when no rule matched and the default `ask` applied) plus the concrete request subject,
/// so a subscriber can surface or audit exactly what is awaiting approval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PermissionAsked {
    /// The gated tool (e.g. `bash`, `write`).
    pub tool: String,
    /// The pattern that classified the action as `ask` (the action's subject when defaulted).
    pub pattern: String,
    /// The concrete subject evaluated (e.g. the bash command or the target path).
    pub request: String,
}

/// A pending permission ask was answered with a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PermissionReplied {
    /// The gated tool (e.g. `bash`, `write`).
    pub tool: String,
    /// The pattern that classified the action as `ask` (the action's subject when defaulted).
    pub pattern: String,
    /// The concrete subject evaluated (e.g. the bash command or the target path).
    pub request: String,
    /// Whether the action was allowed to proceed or blocked.
    pub decision: PermissionDecision,
}

/// A connected MCP server's advertised tool set changed at runtime (the server sent a
/// tool-list-changed notification and the catalogue was re-listed), so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct McpToolsChanged {
    /// The configured MCP server whose tool set changed.
    pub server: String,
    /// The server-qualified ids of the tools now advertised by that server, after the re-list.
    pub tools: Vec<String>,
}

/// A connected MCP server's advertised resource set changed at runtime, so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct McpResourcesChanged {
    /// The configured MCP server whose resource set changed.
    pub server: String,
    /// The resource URIs now advertised by that server, after the re-list.
    pub resources: Vec<String>,
}

/// A connected MCP server's advertised prompt set changed at runtime, so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct McpPromptsChanged {
    /// The configured MCP server whose prompt set changed.
    pub server: String,
    /// The prompt names now advertised by that server, after the re-list.
    pub prompts: Vec<String>,
}

/// A typed event delivered on the [`EventBus`](crate::EventBus).
///
/// Each variant pairs an event *kind* with a strongly typed payload, so subscribers
/// (in-process today, the server SSE stream later) observe named fields rather than opaque
/// strings. Variants serialize with an internal `kind` tag, making them SSE/JSON ready.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A process/surface lifecycle transition.
    Lifecycle(Lifecycle),
    /// A gated tool action was suspended pending a permission decision.
    #[serde(rename = "permission.asked")]
    PermissionAsked(PermissionAsked),
    /// A pending permission ask was answered with a decision.
    #[serde(rename = "permission.replied")]
    PermissionReplied(PermissionReplied),
    /// A connected MCP server's advertised tool set changed at runtime.
    #[serde(rename = "mcp.tools.changed")]
    McpToolsChanged(McpToolsChanged),
    /// A connected MCP server's advertised resource set changed at runtime.
    #[serde(rename = "mcp.resources.changed")]
    McpResourcesChanged(McpResourcesChanged),
    /// A connected MCP server's advertised prompt set changed at runtime.
    #[serde(rename = "mcp.prompts.changed")]
    McpPromptsChanged(McpPromptsChanged),
}

impl Event {
    /// The stable wire/topic name of this event's kind.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Event::Lifecycle(_) => "lifecycle",
            Event::PermissionAsked(_) => "permission.asked",
            Event::PermissionReplied(_) => "permission.replied",
            Event::McpToolsChanged(_) => "mcp.tools.changed",
            Event::McpResourcesChanged(_) => "mcp.resources.changed",
            Event::McpPromptsChanged(_) => "mcp.prompts.changed",
        }
    }

    /// Construct a lifecycle event.
    #[must_use]
    pub fn lifecycle(phase: LifecyclePhase, surface: Surface, version: impl Into<String>) -> Self {
        Event::Lifecycle(Lifecycle {
            phase,
            surface,
            version: version.into(),
        })
    }

    /// Construct a `permission.asked` event naming the gated tool, the matching pattern, and the
    /// concrete request subject.
    #[must_use]
    pub fn permission_asked(
        tool: impl Into<String>,
        pattern: impl Into<String>,
        request: impl Into<String>,
    ) -> Self {
        Event::PermissionAsked(PermissionAsked {
            tool: tool.into(),
            pattern: pattern.into(),
            request: request.into(),
        })
    }

    /// Construct a `permission.replied` event carrying the decision for a prior ask.
    #[must_use]
    pub fn permission_replied(
        tool: impl Into<String>,
        pattern: impl Into<String>,
        request: impl Into<String>,
        decision: PermissionDecision,
    ) -> Self {
        Event::PermissionReplied(PermissionReplied {
            tool: tool.into(),
            pattern: pattern.into(),
            request: request.into(),
            decision,
        })
    }

    /// Construct an `mcp.tools.changed` event naming the server and its re-listed tool ids.
    #[must_use]
    pub fn mcp_tools_changed(server: impl Into<String>, tools: Vec<String>) -> Self {
        Event::McpToolsChanged(McpToolsChanged {
            server: server.into(),
            tools,
        })
    }

    /// Construct an `mcp.resources.changed` event naming the server and its re-listed resource URIs.
    #[must_use]
    pub fn mcp_resources_changed(server: impl Into<String>, resources: Vec<String>) -> Self {
        Event::McpResourcesChanged(McpResourcesChanged {
            server: server.into(),
            resources,
        })
    }

    /// Construct an `mcp.prompts.changed` event naming the server and its re-listed prompt names.
    #[must_use]
    pub fn mcp_prompts_changed(server: impl Into<String>, prompts: Vec<String>) -> Self {
        Event::McpPromptsChanged(McpPromptsChanged {
            server: server.into(),
            prompts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_stable() {
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0");
        assert_eq!(event.kind(), "lifecycle");
    }

    #[test]
    fn lifecycle_serializes_with_named_typed_fields() {
        let event = Event::lifecycle(LifecyclePhase::Startup, Surface::Run, "0.1.0");
        let value = serde_json::to_value(&event).expect("serialize");

        assert_eq!(value["kind"], "lifecycle");
        assert_eq!(value["phase"], "startup");
        assert_eq!(value["surface"], "run");
        assert_eq!(value["version"], "0.1.0");
    }

    #[test]
    fn phase_and_surface_names_match_serialization() {
        assert_eq!(LifecyclePhase::Startup.as_str(), "startup");
        assert_eq!(LifecyclePhase::Shutdown.as_str(), "shutdown");
        assert_eq!(Surface::Tui.as_str(), "tui");
        assert_eq!(Surface::Serve.as_str(), "serve");
        assert_eq!(Surface::Config.as_str(), "config");
    }

    #[test]
    fn permission_asked_serializes_with_named_typed_fields() {
        let event = Event::permission_asked("bash", "rm *", "rm target.txt");
        assert_eq!(event.kind(), "permission.asked");

        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "permission.asked");
        assert_eq!(value["tool"], "bash");
        assert_eq!(value["pattern"], "rm *");
        assert_eq!(value["request"], "rm target.txt");
    }

    #[test]
    fn permission_replied_carries_the_decision() {
        let event = Event::permission_replied(
            "write",
            "secrets/**",
            "secrets/k.txt",
            PermissionDecision::Deny,
        );
        assert_eq!(event.kind(), "permission.replied");

        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "permission.replied");
        assert_eq!(value["tool"], "write");
        assert_eq!(value["pattern"], "secrets/**");
        assert_eq!(value["request"], "secrets/k.txt");
        assert_eq!(value["decision"], "deny");
    }

    #[test]
    fn permission_decision_names_match_serialization() {
        assert_eq!(PermissionDecision::Allow.as_str(), "allow");
        assert_eq!(PermissionDecision::Deny.as_str(), "deny");
    }

    #[test]
    fn mcp_tools_changed_serializes_with_named_typed_fields() {
        let event =
            Event::mcp_tools_changed("demo", vec!["demo_a".to_owned(), "demo_b".to_owned()]);
        assert_eq!(event.kind(), "mcp.tools.changed");

        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "mcp.tools.changed");
        assert_eq!(value["server"], "demo");
        assert_eq!(value["tools"][0], "demo_a");
        assert_eq!(value["tools"][1], "demo_b");
    }

    #[test]
    fn mcp_resources_and_prompts_changed_serialize_with_named_typed_fields() {
        let resources = Event::mcp_resources_changed("demo", vec!["notes://demo".to_owned()]);
        assert_eq!(resources.kind(), "mcp.resources.changed");
        let value = serde_json::to_value(&resources).expect("serialize");
        assert_eq!(value["kind"], "mcp.resources.changed");
        assert_eq!(value["resources"][0], "notes://demo");

        let prompts = Event::mcp_prompts_changed("demo", vec!["greet".to_owned()]);
        assert_eq!(prompts.kind(), "mcp.prompts.changed");
        let value = serde_json::to_value(&prompts).expect("serialize");
        assert_eq!(value["kind"], "mcp.prompts.changed");
        assert_eq!(value["prompts"][0], "greet");
    }
}
