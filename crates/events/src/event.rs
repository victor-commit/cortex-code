//! Typed event definitions carried on the [`EventBus`](crate::EventBus).

use protocol::MessagePart;
use serde::{Deserialize, Serialize};

/// The surface that produced an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The interactive terminal UI.
    Tui,
    /// The headless `cortex run` path.
    Run,
    /// The local loopback server.
    Serve,
    /// The Agent Client Protocol stdio server.
    Acp,
    /// The configuration-reporting path.
    Config,
    /// The persisted-session management path (`cortex session`).
    Session,
    /// The agent discovery path (`cortex agent`).
    Agent,
    /// The outbound-network diagnostics path (`cortex net`).
    Net,
    /// The checkpoint undo/redo path (`cortex undo`/`cortex redo`).
    Checkpoint,
    /// The MCP server management path (`cortex mcp`).
    Mcp,
    /// The language-server management path (`cortex lsp`).
    Lsp,
    /// The auth-method listing path (`cortex auth`).
    Auth,
    /// The model-catalogue listing path (`cortex models`).
    Models,
}

impl Surface {
    /// The stable lowercase name of this surface.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Surface::Tui => "tui",
            Surface::Run => "run",
            Surface::Serve => "serve",
            Surface::Acp => "acp",
            Surface::Config => "config",
            Surface::Session => "session",
            Surface::Agent => "agent",
            Surface::Net => "net",
            Surface::Checkpoint => "checkpoint",
            Surface::Mcp => "mcp",
            Surface::Lsp => "lsp",
            Surface::Auth => "auth",
            Surface::Models => "models",
        }
    }
}

/// The phase of a process/surface lifecycle transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lifecycle {
    /// Whether the surface is starting up or shutting down.
    pub phase: LifecyclePhase,
    /// Which surface emitted the transition.
    pub surface: Surface,
    /// The binary version at the time of the transition.
    pub version: String,
}

/// The concrete answer to a permission ask: run the action or block it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAsked {
    /// Owning session id when the ask belongs to a user turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn/message id that originated the ask when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The gated tool (e.g. `bash`, `write`).
    pub tool: String,
    /// The pattern that classified the action as `ask` (the action's subject when defaulted).
    pub pattern: String,
    /// The concrete subject evaluated (e.g. the bash command or the target path).
    pub request: String,
}

/// A pending permission ask was answered with a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReplied {
    /// Owning session id when the reply belongs to a user turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn/message id that originated the reply when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The gated tool (e.g. `bash`, `write`).
    pub tool: String,
    /// The pattern that classified the action as `ask` (the action's subject when defaulted).
    pub pattern: String,
    /// The concrete subject evaluated (e.g. the bash command or the target path).
    pub request: String,
    /// Whether the action was allowed to proceed or blocked.
    pub decision: PermissionDecision,
    /// Optional corrective feedback supplied with a deny decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

/// A connected MCP server's advertised tool set changed at runtime (the server sent a
/// tool-list-changed notification and the catalogue was re-listed), so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolsChanged {
    /// The configured MCP server whose tool set changed.
    pub server: String,
    /// The server-qualified ids of the tools now advertised by that server, after the re-list.
    pub tools: Vec<String>,
}

/// A connected MCP server's advertised resource set changed at runtime, so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResourcesChanged {
    /// The configured MCP server whose resource set changed.
    pub server: String,
    /// The resource URIs now advertised by that server, after the re-list.
    pub resources: Vec<String>,
}

/// A connected MCP server's advertised prompt set changed at runtime, so subscribers can refresh
/// without restarting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptsChanged {
    /// The configured MCP server whose prompt set changed.
    pub server: String,
    /// The prompt names now advertised by that server, after the re-list.
    pub prompts: Vec<String>,
}

/// A model requested a tool call during a turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// Owning session id when the call belongs to a user turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn/message id that originated the call when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Provider-assigned call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// JSON argument string supplied by the model.
    pub arguments: String,
}

/// A tool call completed and produced a model-visible result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultEvent {
    /// Owning session id when the result belongs to a user turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Turn/message id that originated the result when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Provider-assigned call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Whether the tool result is an error.
    pub is_error: bool,
}

/// A server/runner turn started for a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStartedEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
}

/// A fragment of assistant-visible text for one session turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantTextDeltaEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
    /// Assistant text fragment.
    pub text: String,
}

/// A fragment of model reasoning for one session turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantReasoningDeltaEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
    /// Reasoning fragment.
    pub text: String,
}

/// Per-turn token usage reported by the model provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnUsageEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
    /// Prompt/input tokens.
    pub input_tokens: u32,
    /// Completion/output tokens.
    pub output_tokens: u32,
    /// Total tokens.
    pub total_tokens: u32,
    /// Optional provider-reported cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// A normalized turn failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnErrorEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
    /// Stable error class.
    pub error_kind: String,
    /// Human-readable error message.
    pub message: String,
}

/// A turn reached a terminal state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnDoneEvent {
    /// Owning session id.
    pub session_id: String,
    /// User message id that identifies the turn.
    pub message_id: String,
    /// Normalized finish reason (`stop`, `tool_calls`, `aborted`, ...).
    pub reason: String,
    /// Provider round-trips completed before the terminal state.
    pub steps: usize,
}

/// Rich parts persisted for a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagePartsEvent {
    /// Owning session id.
    pub session_id: String,
    /// Persisted message id.
    pub message_id: String,
    /// The typed persisted parts in stored order.
    pub parts: Vec<MessagePart>,
}

/// A typed event delivered on the [`EventBus`](crate::EventBus).
///
/// Each variant pairs an event *kind* with a strongly typed payload, so subscribers
/// (in-process today, the server SSE stream later) observe named fields rather than opaque
/// strings. Variants serialize with an internal `kind` tag, making them SSE/JSON ready.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// A model requested a tool call during a turn.
    #[serde(rename = "tool.call")]
    ToolCall(ToolCallEvent),
    /// A tool call completed and produced a model-visible result.
    #[serde(rename = "tool.result")]
    ToolResult(ToolResultEvent),
    /// A server/runner turn started for a session.
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedEvent),
    /// A fragment of assistant-visible text for one session turn.
    #[serde(rename = "assistant.text_delta")]
    AssistantTextDelta(AssistantTextDeltaEvent),
    /// A fragment of model reasoning for one session turn.
    #[serde(rename = "assistant.reasoning_delta")]
    AssistantReasoningDelta(AssistantReasoningDeltaEvent),
    /// Per-turn token usage reported by the model provider.
    #[serde(rename = "turn.usage")]
    TurnUsage(TurnUsageEvent),
    /// A normalized turn failure.
    #[serde(rename = "turn.error")]
    TurnError(TurnErrorEvent),
    /// A turn reached a terminal state.
    #[serde(rename = "turn.done")]
    TurnDone(TurnDoneEvent),
    /// Rich parts persisted for a message.
    #[serde(rename = "message.parts")]
    MessageParts(MessagePartsEvent),
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
            Event::ToolCall(_) => "tool.call",
            Event::ToolResult(_) => "tool.result",
            Event::TurnStarted(_) => "turn.started",
            Event::AssistantTextDelta(_) => "assistant.text_delta",
            Event::AssistantReasoningDelta(_) => "assistant.reasoning_delta",
            Event::TurnUsage(_) => "turn.usage",
            Event::TurnError(_) => "turn.error",
            Event::TurnDone(_) => "turn.done",
            Event::MessageParts(_) => "message.parts",
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
            session_id: None,
            message_id: None,
            tool: tool.into(),
            pattern: pattern.into(),
            request: request.into(),
        })
    }

    /// Construct a session-correlated `permission.asked` event naming the gated action.
    #[must_use]
    pub fn permission_asked_for_turn(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        tool: impl Into<String>,
        pattern: impl Into<String>,
        request: impl Into<String>,
    ) -> Self {
        Event::PermissionAsked(PermissionAsked {
            session_id: Some(session_id.into()),
            message_id: Some(message_id.into()),
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
        feedback: Option<&str>,
    ) -> Self {
        Event::PermissionReplied(PermissionReplied {
            session_id: None,
            message_id: None,
            tool: tool.into(),
            pattern: pattern.into(),
            request: request.into(),
            decision,
            feedback: feedback.map(ToOwned::to_owned),
        })
    }

    /// Construct a session-correlated `permission.replied` event.
    #[must_use]
    pub fn permission_replied_for_turn(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        tool: impl Into<String>,
        pattern: impl Into<String>,
        request: impl Into<String>,
        decision: PermissionDecision,
        feedback: Option<&str>,
    ) -> Self {
        Event::PermissionReplied(PermissionReplied {
            session_id: Some(session_id.into()),
            message_id: Some(message_id.into()),
            tool: tool.into(),
            pattern: pattern.into(),
            request: request.into(),
            decision,
            feedback: feedback.map(ToOwned::to_owned),
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

    /// Construct a `tool.call` event naming the requested tool and call id.
    #[must_use]
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Event::ToolCall(ToolCallEvent {
            session_id: None,
            message_id: None,
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        })
    }

    /// Construct a session-correlated `tool.call` event.
    #[must_use]
    pub fn tool_call_for_turn(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Event::ToolCall(ToolCallEvent {
            session_id: Some(session_id.into()),
            message_id: Some(message_id.into()),
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        })
    }

    /// Construct a `tool.result` event naming the completed tool call.
    #[must_use]
    pub fn tool_result(id: impl Into<String>, name: impl Into<String>, is_error: bool) -> Self {
        Event::ToolResult(ToolResultEvent {
            session_id: None,
            message_id: None,
            id: id.into(),
            name: name.into(),
            is_error,
        })
    }

    /// Construct a session-correlated `tool.result` event.
    #[must_use]
    pub fn tool_result_for_turn(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Event::ToolResult(ToolResultEvent {
            session_id: Some(session_id.into()),
            message_id: Some(message_id.into()),
            id: id.into(),
            name: name.into(),
            is_error,
        })
    }

    /// Construct a `turn.started` event.
    #[must_use]
    pub fn turn_started(session_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Event::TurnStarted(TurnStartedEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
        })
    }

    /// Construct an `assistant.text_delta` event.
    #[must_use]
    pub fn assistant_text_delta(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Event::AssistantTextDelta(AssistantTextDeltaEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            text: text.into(),
        })
    }

    /// Construct an `assistant.reasoning_delta` event.
    #[must_use]
    pub fn assistant_reasoning_delta(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Event::AssistantReasoningDelta(AssistantReasoningDeltaEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            text: text.into(),
        })
    }

    /// Construct a `turn.usage` event.
    #[must_use]
    pub fn turn_usage(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cost: Option<f64>,
    ) -> Self {
        Event::TurnUsage(TurnUsageEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
        })
    }

    /// Construct a `turn.error` event.
    #[must_use]
    pub fn turn_error(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        error_kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Event::TurnError(TurnErrorEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            error_kind: error_kind.into(),
            message: message.into(),
        })
    }

    /// Construct a `turn.done` event.
    #[must_use]
    pub fn turn_done(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        reason: impl Into<String>,
        steps: usize,
    ) -> Self {
        Event::TurnDone(TurnDoneEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            reason: reason.into(),
            steps,
        })
    }

    /// Construct a `message.parts` event.
    #[must_use]
    pub fn message_parts(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        parts: Vec<MessagePart>,
    ) -> Self {
        Event::MessageParts(MessagePartsEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            parts,
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
        assert_eq!(Surface::Agent.as_str(), "agent");
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
            None,
        );
        assert_eq!(event.kind(), "permission.replied");

        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "permission.replied");
        assert_eq!(value["tool"], "write");
        assert_eq!(value["pattern"], "secrets/**");
        assert_eq!(value["request"], "secrets/k.txt");
        assert_eq!(value["decision"], "deny");
        assert!(value.get("feedback").is_none());
    }

    #[test]
    fn permission_replied_serializes_optional_feedback() {
        let event = Event::permission_replied(
            "bash",
            "rm *",
            "rm target.txt",
            PermissionDecision::Deny,
            Some("Use a dry-run command first."),
        );
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["feedback"], "Use a dry-run command first.");
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

    #[test]
    fn tool_events_serialize_with_named_typed_fields() {
        let call = Event::tool_call("call_1", "cortex_echo", "{\"text\":\"hi\"}");
        assert_eq!(call.kind(), "tool.call");
        let value = serde_json::to_value(&call).expect("serialize");
        assert_eq!(value["kind"], "tool.call");
        assert_eq!(value["id"], "call_1");
        assert_eq!(value["name"], "cortex_echo");

        let result = Event::tool_result("call_1", "cortex_echo", false);
        assert_eq!(result.kind(), "tool.result");
        let value = serde_json::to_value(&result).expect("serialize");
        assert_eq!(value["kind"], "tool.result");
        assert_eq!(value["is_error"], false);
    }
}
