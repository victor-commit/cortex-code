//! Typed event definitions carried on the [`EventBus`](crate::EventBus).

use protocol::{McpToolResult, MessagePart, ToolResultPart};
use serde::{Deserialize, Serialize};

use crate::families::{
    CommandExecuted, FileEdited, FileWatcherUpdated, InstallationUpdated, LspUpdated,
    MessagePartRemoved, MessagePartUpdated, MessageRemoved, MessageUpdated, ProjectUpdated,
    PtyEvent, PtyExited, PtyRef, QuestionAsked, QuestionRejected, QuestionReplied,
    SessionAgentSwitched, SessionCompactedEvent, SessionContext, SessionDiffEvent,
    SessionErrorEvent, SessionModelSwitched, SessionRef, SessionRetried, SessionStatusEvent,
    StepEvent, StreamBoundary, StreamDelta, TodoUpdated, ToolEnded, ToolStarted, TuiCommandExecute,
    TuiPromptAppend, TuiSessionSelect, TuiToastShow, VcsBranchUpdated, WorkspaceEvent,
    WorkspaceStatus, WorktreeFailed, WorktreeReady, WorktreeRemoved, MissionStarted,
    MissionWorkerStarted, MissionWorkerCompleted, MissionWorkerFailed, MissionProgress,
    MissionValidation, MissionFinished,
};

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
    /// The missions engine path (`cortex mission`).
    Mission,
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
            Surface::Mission => "mission",
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

/// One MCP server `notifications/message` payload bridged from the MCP client notification stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpLogMessage {
    /// The configured MCP server that emitted the notification.
    pub server: String,
    /// The MCP logging severity level (`debug`, `info`, `warning`, ...).
    pub level: String,
    /// Optional logger/source field from the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    /// The redacted notification payload.
    pub data: serde_json::Value,
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
    /// Bounded model-facing result parts, retained for immediate surface rendering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<ToolResultPart>,
    /// Complete typed MCP source result, when the tool call was routed to MCP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_result: Option<McpToolResult>,
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
    /// Prompt-cache write tokens (cache creation), when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    /// Prompt-cache read tokens, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
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
    /// A connected MCP server emitted a `notifications/message` log payload.
    #[serde(rename = "mcp.log")]
    McpLogMessage(McpLogMessage),
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
    /// A persisted message changed (created or rewritten).
    #[serde(rename = "message.updated")]
    MessageUpdated(MessageUpdated),
    /// A persisted message was removed.
    #[serde(rename = "message.removed")]
    MessageRemoved(MessageRemoved),
    /// A persisted message part changed.
    #[serde(rename = "message.part.updated")]
    MessagePartUpdated(MessagePartUpdated),
    /// A persisted message part was removed.
    #[serde(rename = "message.part.removed")]
    MessagePartRemoved(MessagePartRemoved),
    /// The granular text channel started streaming for a turn.
    #[serde(rename = "session.next.text.start")]
    SessionNextTextStart(StreamBoundary),
    /// A granular text fragment for a turn.
    #[serde(rename = "session.next.text.delta")]
    SessionNextTextDelta(StreamDelta),
    /// The granular text channel finished for a turn.
    #[serde(rename = "session.next.text.end")]
    SessionNextTextEnd(StreamBoundary),
    /// The granular reasoning channel started streaming for a turn.
    #[serde(rename = "session.next.reasoning.start")]
    SessionNextReasoningStart(StreamBoundary),
    /// A granular reasoning fragment for a turn.
    #[serde(rename = "session.next.reasoning.delta")]
    SessionNextReasoningDelta(StreamDelta),
    /// The granular reasoning channel finished for a turn.
    #[serde(rename = "session.next.reasoning.end")]
    SessionNextReasoningEnd(StreamBoundary),
    /// A tool call started streaming for a turn.
    #[serde(rename = "session.next.tool.started")]
    SessionNextToolStarted(ToolStarted),
    /// A tool call finished for a turn.
    #[serde(rename = "session.next.tool.ended")]
    SessionNextToolEnded(ToolEnded),
    /// A step started for a turn.
    #[serde(rename = "session.next.step.started")]
    SessionNextStepStarted(StepEvent),
    /// A step finished for a turn.
    #[serde(rename = "session.next.step.finished")]
    SessionNextStepFinished(StepEvent),
    /// The runner scheduled a retry attempt for a turn.
    #[serde(rename = "session.next.retried")]
    SessionNextRetried(SessionRetried),
    /// The turn's effective context-window usage changed.
    #[serde(rename = "session.next.context.updated")]
    SessionNextContextUpdated(SessionContext),
    /// The turn's active model was selected/switched.
    #[serde(rename = "session.next.model.switched")]
    SessionNextModelSwitched(SessionModelSwitched),
    /// The turn's active agent was selected/switched.
    #[serde(rename = "session.next.agent.switched")]
    SessionNextAgentSwitched(SessionAgentSwitched),
    /// A persisted session changed.
    #[serde(rename = "session.updated")]
    SessionUpdated(SessionRef),
    /// A session's busy/idle status changed.
    #[serde(rename = "session.status")]
    SessionStatus(SessionStatusEvent),
    /// A session became idle (its active turn finished).
    #[serde(rename = "session.idle")]
    SessionIdle(SessionRef),
    /// A normalized session-level error.
    #[serde(rename = "session.error")]
    SessionError(SessionErrorEvent),
    /// A session's working-tree diff summary changed.
    #[serde(rename = "session.diff")]
    SessionDiff(SessionDiffEvent),
    /// A session's context was compacted through an LLM summary.
    #[serde(rename = "session.compacted")]
    SessionCompacted(SessionCompactedEvent),
    /// A question was asked of the user.
    #[serde(rename = "question.asked")]
    QuestionAsked(QuestionAsked),
    /// A question was answered.
    #[serde(rename = "question.replied")]
    QuestionReplied(QuestionReplied),
    /// A question was rejected/dismissed.
    #[serde(rename = "question.rejected")]
    QuestionRejected(QuestionRejected),
    /// A session's todo list changed.
    #[serde(rename = "todo.updated")]
    TodoUpdated(TodoUpdated),
    /// A file was written/edited.
    #[serde(rename = "file.edited")]
    FileEdited(FileEdited),
    /// A watched file changed on disk.
    #[serde(rename = "file.watcher.updated")]
    FileWatcherUpdated(FileWatcherUpdated),
    /// An LSP server produced new state/diagnostics.
    #[serde(rename = "lsp.updated")]
    LspUpdated(LspUpdated),
    /// The VCS branch/dirty state changed.
    #[serde(rename = "vcs.branch.updated")]
    VcsBranchUpdated(VcsBranchUpdated),
    /// A project's state changed.
    #[serde(rename = "project.updated")]
    ProjectUpdated(ProjectUpdated),
    /// A command was executed.
    #[serde(rename = "command.executed")]
    CommandExecuted(CommandExecuted),
    /// An installation/version transition.
    #[serde(rename = "installation.updated")]
    InstallationUpdated(InstallationUpdated),
    /// A worktree became ready.
    #[serde(rename = "worktree.ready")]
    WorktreeReady(WorktreeReady),
    /// A worktree operation failed.
    #[serde(rename = "worktree.failed")]
    WorktreeFailed(WorktreeFailed),
    /// A worktree was removed.
    #[serde(rename = "worktree.removed")]
    WorktreeRemoved(WorktreeRemoved),
    /// A workspace became ready.
    #[serde(rename = "workspace.ready")]
    WorkspaceReady(WorkspaceEvent),
    /// A workspace operation failed.
    #[serde(rename = "workspace.failed")]
    WorkspaceFailed(WorkspaceEvent),
    /// A workspace status transition.
    #[serde(rename = "workspace.status")]
    WorkspaceStatus(WorkspaceStatus),
    /// A PTY was created.
    #[serde(rename = "pty.created")]
    PtyCreated(PtyEvent),
    /// A PTY was updated.
    #[serde(rename = "pty.updated")]
    PtyUpdated(PtyEvent),
    /// A PTY was removed.
    #[serde(rename = "pty.deleted")]
    PtyDeleted(PtyRef),
    /// A PTY process exited.
    #[serde(rename = "pty.exited")]
    PtyExited(PtyExited),
    /// A remote TUI requested text appended to the prompt.
    #[serde(rename = "tui.prompt.append")]
    TuiPromptAppend(TuiPromptAppend),
    /// A remote TUI requested a command be executed.
    #[serde(rename = "tui.command.execute")]
    TuiCommandExecute(TuiCommandExecute),
    /// A remote TUI requested a toast be shown.
    #[serde(rename = "tui.toast.show")]
    TuiToastShow(TuiToastShow),
    /// A remote TUI requested a session be selected.
    #[serde(rename = "tui.session.select")]
    TuiSessionSelect(TuiSessionSelect),
    /// A mission run started.
    #[serde(rename = "mission.started")]
    MissionStarted(MissionStarted),
    /// A mission worker started a feature.
    #[serde(rename = "mission.worker.started")]
    MissionWorkerStarted(MissionWorkerStarted),
    /// A mission worker completed a feature.
    #[serde(rename = "mission.worker.completed")]
    MissionWorkerCompleted(MissionWorkerCompleted),
    /// A mission worker failed a feature (siblings continue).
    #[serde(rename = "mission.worker.failed")]
    MissionWorkerFailed(MissionWorkerFailed),
    /// A mission progress log entry.
    #[serde(rename = "mission.progress")]
    MissionProgress(MissionProgress),
    /// A milestone validation gate completed.
    #[serde(rename = "mission.validation")]
    MissionValidation(MissionValidation),
    /// A mission run finished.
    #[serde(rename = "mission.finished")]
    MissionFinished(MissionFinished),
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
            Event::McpLogMessage(_) => "mcp.log",
            Event::ToolCall(_) => "tool.call",
            Event::ToolResult(_) => "tool.result",
            Event::TurnStarted(_) => "turn.started",
            Event::AssistantTextDelta(_) => "assistant.text_delta",
            Event::AssistantReasoningDelta(_) => "assistant.reasoning_delta",
            Event::TurnUsage(_) => "turn.usage",
            Event::TurnError(_) => "turn.error",
            Event::TurnDone(_) => "turn.done",
            Event::MessageParts(_) => "message.parts",
            Event::MessageUpdated(_) => "message.updated",
            Event::MessageRemoved(_) => "message.removed",
            Event::MessagePartUpdated(_) => "message.part.updated",
            Event::MessagePartRemoved(_) => "message.part.removed",
            Event::SessionNextTextStart(_) => "session.next.text.start",
            Event::SessionNextTextDelta(_) => "session.next.text.delta",
            Event::SessionNextTextEnd(_) => "session.next.text.end",
            Event::SessionNextReasoningStart(_) => "session.next.reasoning.start",
            Event::SessionNextReasoningDelta(_) => "session.next.reasoning.delta",
            Event::SessionNextReasoningEnd(_) => "session.next.reasoning.end",
            Event::SessionNextToolStarted(_) => "session.next.tool.started",
            Event::SessionNextToolEnded(_) => "session.next.tool.ended",
            Event::SessionNextStepStarted(_) => "session.next.step.started",
            Event::SessionNextStepFinished(_) => "session.next.step.finished",
            Event::SessionNextRetried(_) => "session.next.retried",
            Event::SessionNextContextUpdated(_) => "session.next.context.updated",
            Event::SessionNextModelSwitched(_) => "session.next.model.switched",
            Event::SessionNextAgentSwitched(_) => "session.next.agent.switched",
            Event::SessionUpdated(_) => "session.updated",
            Event::SessionStatus(_) => "session.status",
            Event::SessionIdle(_) => "session.idle",
            Event::SessionError(_) => "session.error",
            Event::SessionDiff(_) => "session.diff",
            Event::SessionCompacted(_) => "session.compacted",
            Event::QuestionAsked(_) => "question.asked",
            Event::QuestionReplied(_) => "question.replied",
            Event::QuestionRejected(_) => "question.rejected",
            Event::TodoUpdated(_) => "todo.updated",
            Event::FileEdited(_) => "file.edited",
            Event::FileWatcherUpdated(_) => "file.watcher.updated",
            Event::LspUpdated(_) => "lsp.updated",
            Event::VcsBranchUpdated(_) => "vcs.branch.updated",
            Event::ProjectUpdated(_) => "project.updated",
            Event::CommandExecuted(_) => "command.executed",
            Event::InstallationUpdated(_) => "installation.updated",
            Event::WorktreeReady(_) => "worktree.ready",
            Event::WorktreeFailed(_) => "worktree.failed",
            Event::WorktreeRemoved(_) => "worktree.removed",
            Event::WorkspaceReady(_) => "workspace.ready",
            Event::WorkspaceFailed(_) => "workspace.failed",
            Event::WorkspaceStatus(_) => "workspace.status",
            Event::PtyCreated(_) => "pty.created",
            Event::PtyUpdated(_) => "pty.updated",
            Event::PtyDeleted(_) => "pty.deleted",
            Event::PtyExited(_) => "pty.exited",
            Event::TuiPromptAppend(_) => "tui.prompt.append",
            Event::TuiCommandExecute(_) => "tui.command.execute",
            Event::TuiToastShow(_) => "tui.toast.show",
            Event::TuiSessionSelect(_) => "tui.session.select",
            Event::MissionStarted(_) => "mission.started",
            Event::MissionWorkerStarted(_) => "mission.worker.started",
            Event::MissionWorkerCompleted(_) => "mission.worker.completed",
            Event::MissionWorkerFailed(_) => "mission.worker.failed",
            Event::MissionProgress(_) => "mission.progress",
            Event::MissionValidation(_) => "mission.validation",
            Event::MissionFinished(_) => "mission.finished",
        }
    }

    /// Every stable event-kind name in the taxonomy, in declaration order.
    ///
    /// This catalogue backs spec/SDK discovery: a typed client generated from the OpenAPI document
    /// can enumerate the families it must decode, and a server-side test can assert the OpenAPI
    /// listing stays in lockstep with the typed [`Event`] variants.
    #[must_use]
    pub const fn all_kinds() -> &'static [&'static str] {
        &[
            "lifecycle",
            "permission.asked",
            "permission.replied",
            "mcp.tools.changed",
            "mcp.resources.changed",
            "mcp.prompts.changed",
            "mcp.log",
            "tool.call",
            "tool.result",
            "turn.started",
            "assistant.text_delta",
            "assistant.reasoning_delta",
            "turn.usage",
            "turn.error",
            "turn.done",
            "message.parts",
            "message.updated",
            "message.removed",
            "message.part.updated",
            "message.part.removed",
            "session.next.text.start",
            "session.next.text.delta",
            "session.next.text.end",
            "session.next.reasoning.start",
            "session.next.reasoning.delta",
            "session.next.reasoning.end",
            "session.next.tool.started",
            "session.next.tool.ended",
            "session.next.step.started",
            "session.next.step.finished",
            "session.next.retried",
            "session.next.context.updated",
            "session.next.model.switched",
            "session.next.agent.switched",
            "session.updated",
            "session.status",
            "session.idle",
            "session.error",
            "session.diff",
            "session.compacted",
            "question.asked",
            "question.replied",
            "question.rejected",
            "todo.updated",
            "file.edited",
            "file.watcher.updated",
            "lsp.updated",
            "vcs.branch.updated",
            "project.updated",
            "command.executed",
            "installation.updated",
            "worktree.ready",
            "worktree.failed",
            "worktree.removed",
            "workspace.ready",
            "workspace.failed",
            "workspace.status",
            "pty.created",
            "pty.updated",
            "pty.deleted",
            "pty.exited",
            "tui.prompt.append",
            "tui.command.execute",
            "tui.toast.show",
            "tui.session.select",
            "mission.started",
            "mission.worker.started",
            "mission.worker.completed",
            "mission.worker.failed",
            "mission.progress",
            "mission.validation",
            "mission.finished",
        ]
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

    /// Construct an `mcp.log` event carrying one MCP `notifications/message` payload.
    #[must_use]
    pub fn mcp_log_message(
        server: impl Into<String>,
        level: impl Into<String>,
        logger: Option<String>,
        data: serde_json::Value,
    ) -> Self {
        Event::McpLogMessage(McpLogMessage {
            server: server.into(),
            level: level.into(),
            logger,
            data,
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
        Self::tool_result_with_parts(id, name, Vec::new(), None, is_error)
    }

    /// Construct a `tool.result` event with both the bounded result projection and, when present,
    /// a lossless typed MCP source result.
    #[must_use]
    pub fn tool_result_with_parts(
        id: impl Into<String>,
        name: impl Into<String>,
        parts: Vec<ToolResultPart>,
        mcp_result: Option<McpToolResult>,
        is_error: bool,
    ) -> Self {
        Event::ToolResult(ToolResultEvent {
            session_id: None,
            message_id: None,
            id: id.into(),
            name: name.into(),
            parts,
            mcp_result,
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
        Self::tool_result_for_turn_with_parts(
            session_id,
            message_id,
            id,
            name,
            Vec::new(),
            None,
            is_error,
        )
    }

    /// Construct a session-correlated typed `tool.result` event.
    #[must_use]
    pub fn tool_result_for_turn_with_parts(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        parts: Vec<ToolResultPart>,
        mcp_result: Option<McpToolResult>,
        is_error: bool,
    ) -> Self {
        Event::ToolResult(ToolResultEvent {
            session_id: Some(session_id.into()),
            message_id: Some(message_id.into()),
            id: id.into(),
            name: name.into(),
            parts,
            mcp_result,
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
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn turn_usage(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_creation_input_tokens: Option<u32>,
        cache_read_input_tokens: Option<u32>,
        cost: Option<f64>,
    ) -> Self {
        Event::TurnUsage(TurnUsageEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            input_tokens,
            output_tokens,
            total_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
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

    /// Construct a `message.updated` event.
    #[must_use]
    pub fn message_updated(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        role: impl Into<String>,
        time_created: Option<String>,
    ) -> Self {
        Event::MessageUpdated(MessageUpdated {
            session_id: session_id.into(),
            message_id: message_id.into(),
            role: role.into(),
            time_created,
        })
    }

    /// Construct a `message.part.updated` event.
    #[must_use]
    pub fn message_part_updated(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        part: MessagePart,
    ) -> Self {
        let part_id = part.id.clone();
        Event::MessagePartUpdated(MessagePartUpdated {
            session_id: session_id.into(),
            message_id: message_id.into(),
            part_id,
            part,
        })
    }

    /// Construct a `message.removed` event.
    #[must_use]
    pub fn message_removed(session_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Event::MessageRemoved(MessageRemoved {
            session_id: session_id.into(),
            message_id: message_id.into(),
        })
    }

    /// Construct a `message.part.removed` event.
    #[must_use]
    pub fn message_part_removed(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        part_id: impl Into<String>,
    ) -> Self {
        Event::MessagePartRemoved(MessagePartRemoved {
            session_id: session_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
        })
    }

    /// Construct a `session.updated` event (a session's metadata changed).
    #[must_use]
    pub fn session_updated(session_id: impl Into<String>) -> Self {
        Event::SessionUpdated(SessionRef {
            session_id: session_id.into(),
        })
    }

    /// Construct a `session.status` event.
    #[must_use]
    pub fn session_status(session_id: impl Into<String>, status: impl Into<String>) -> Self {
        Event::SessionStatus(SessionStatusEvent {
            session_id: session_id.into(),
            status: status.into(),
        })
    }

    /// Construct a `session.idle` event.
    #[must_use]
    pub fn session_idle(session_id: impl Into<String>) -> Self {
        Event::SessionIdle(SessionRef {
            session_id: session_id.into(),
        })
    }

    /// Construct a `session.error` event.
    #[must_use]
    pub fn session_error(
        session_id: impl Into<String>,
        message_id: Option<String>,
        error_kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Event::SessionError(SessionErrorEvent {
            session_id: session_id.into(),
            message_id,
            error_kind: error_kind.into(),
            message: message.into(),
        })
    }

    /// Construct a `session.compacted` event.
    #[must_use]
    pub fn session_compacted(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        tail_start_id: impl Into<String>,
        model: Option<String>,
    ) -> Self {
        Event::SessionCompacted(SessionCompactedEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            tail_start_id: tail_start_id.into(),
            model,
        })
    }

    /// Construct a `session.next.step.started` event.
    #[must_use]
    pub fn session_step_started(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        step: usize,
    ) -> Self {
        Event::SessionNextStepStarted(StepEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            step,
        })
    }

    /// Construct a `session.next.step.finished` event.
    #[must_use]
    pub fn session_step_finished(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        step: usize,
    ) -> Self {
        Event::SessionNextStepFinished(StepEvent {
            session_id: session_id.into(),
            message_id: message_id.into(),
            step,
        })
    }

    /// Construct a `session.next.retried` event.
    #[must_use]
    pub fn session_retried(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        attempt: usize,
        max_attempts: usize,
        delay_ms: u64,
        classification: impl Into<String>,
    ) -> Self {
        Event::SessionNextRetried(SessionRetried {
            session_id: session_id.into(),
            message_id: message_id.into(),
            attempt,
            max_attempts,
            delay_ms,
            classification: classification.into(),
        })
    }

    /// Construct a `session.next.context.updated` event.
    #[must_use]
    pub fn session_context_updated(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        tokens: u32,
        context_limit: Option<u32>,
    ) -> Self {
        Event::SessionNextContextUpdated(SessionContext {
            session_id: session_id.into(),
            message_id: message_id.into(),
            tokens,
            context_limit,
        })
    }

    /// Construct a `session.next.model.switched` event.
    #[must_use]
    pub fn session_model_switched(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Event::SessionNextModelSwitched(SessionModelSwitched {
            session_id: session_id.into(),
            message_id: message_id.into(),
            model: model.into(),
        })
    }

    /// Construct a `session.next.agent.switched` event.
    #[must_use]
    pub fn session_agent_switched(
        session_id: impl Into<String>,
        message_id: impl Into<String>,
        agent: impl Into<String>,
    ) -> Self {
        Event::SessionNextAgentSwitched(SessionAgentSwitched {
            session_id: session_id.into(),
            message_id: message_id.into(),
            agent: agent.into(),
        })
    }

    /// Construct a `command.executed` event.
    #[must_use]
    pub fn command_executed(
        session_id: Option<String>,
        command: impl Into<String>,
        exit_code: Option<i32>,
    ) -> Self {
        Event::CommandExecuted(CommandExecuted {
            session_id,
            command: command.into(),
            exit_code,
        })
    }

    /// Construct a `file.edited` event.
    #[must_use]
    pub fn file_edited(
        session_id: Option<String>,
        path: impl Into<String>,
        edit_kind: Option<String>,
    ) -> Self {
        Event::FileEdited(FileEdited {
            session_id,
            path: path.into(),
            edit_kind,
        })
    }

    /// Construct a `question.asked` event.
    #[must_use]
    pub fn question_asked(
        session_id: Option<String>,
        id: impl Into<String>,
        question: impl Into<String>,
        options: Vec<String>,
    ) -> Self {
        Event::QuestionAsked(QuestionAsked {
            session_id,
            id: id.into(),
            question: question.into(),
            options,
        })
    }

    /// Construct a `question.replied` event.
    #[must_use]
    pub fn question_replied(
        session_id: Option<String>,
        id: impl Into<String>,
        answer: impl Into<String>,
    ) -> Self {
        Event::QuestionReplied(QuestionReplied {
            session_id,
            id: id.into(),
            answer: answer.into(),
        })
    }

    /// Construct a `question.rejected` event.
    #[must_use]
    pub fn question_rejected(
        session_id: Option<String>,
        id: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        Event::QuestionRejected(QuestionRejected {
            session_id,
            id: id.into(),
            reason,
        })
    }

    /// Construct a `todo.updated` event.
    #[must_use]
    pub fn todo_updated(session_id: impl Into<String>, todos: serde_json::Value) -> Self {
        Event::TodoUpdated(TodoUpdated {
            session_id: session_id.into(),
            todos,
        })
    }

    /// Construct a `worktree.ready` event naming the worktree path.
    #[must_use]
    pub fn worktree_ready(path: impl Into<String>) -> Self {
        Event::WorktreeReady(WorktreeReady { path: path.into() })
    }

    /// Construct a `worktree.failed` event naming the worktree path and the scrubbed failure detail.
    #[must_use]
    pub fn worktree_failed(path: impl Into<String>, message: impl Into<String>) -> Self {
        Event::WorktreeFailed(WorktreeFailed {
            path: path.into(),
            message: message.into(),
        })
    }

    /// Construct a `worktree.removed` event naming the removed worktree path.
    #[must_use]
    pub fn worktree_removed(path: impl Into<String>) -> Self {
        Event::WorktreeRemoved(WorktreeRemoved { path: path.into() })
    }

    /// Construct a `mission.started` event.
    #[must_use]
    pub fn mission_started(
        mission_id: impl Into<String>,
        mission_dir: impl Into<String>,
        max_parallel: usize,
    ) -> Self {
        Event::MissionStarted(MissionStarted {
            mission_id: mission_id.into(),
            mission_dir: mission_dir.into(),
            max_parallel,
        })
    }

    /// Construct a `mission.worker.started` event.
    #[must_use]
    pub fn mission_worker_started(
        mission_id: impl Into<String>,
        worker_id: impl Into<String>,
        feature_id: impl Into<String>,
        live_count: usize,
        max_parallel: usize,
    ) -> Self {
        Event::MissionWorkerStarted(MissionWorkerStarted {
            mission_id: mission_id.into(),
            worker_id: worker_id.into(),
            feature_id: feature_id.into(),
            live_count,
            max_parallel,
        })
    }

    /// Construct a `mission.worker.completed` event.
    #[must_use]
    pub fn mission_worker_completed(
        mission_id: impl Into<String>,
        worker_id: impl Into<String>,
        feature_id: impl Into<String>,
        live_count: usize,
    ) -> Self {
        Event::MissionWorkerCompleted(MissionWorkerCompleted {
            mission_id: mission_id.into(),
            worker_id: worker_id.into(),
            feature_id: feature_id.into(),
            live_count,
        })
    }

    /// Construct a `mission.worker.failed` event.
    #[must_use]
    pub fn mission_worker_failed(
        mission_id: impl Into<String>,
        worker_id: impl Into<String>,
        feature_id: impl Into<String>,
        message: impl Into<String>,
        live_count: usize,
    ) -> Self {
        Event::MissionWorkerFailed(MissionWorkerFailed {
            mission_id: mission_id.into(),
            worker_id: worker_id.into(),
            feature_id: feature_id.into(),
            message: message.into(),
            live_count,
        })
    }

    /// Construct a `mission.progress` event.
    #[must_use]
    pub fn mission_progress(
        mission_id: impl Into<String>,
        worker_id: impl Into<String>,
        feature_id: impl Into<String>,
        message: impl Into<String>,
        live_count: usize,
    ) -> Self {
        Event::MissionProgress(MissionProgress {
            mission_id: mission_id.into(),
            worker_id: worker_id.into(),
            feature_id: feature_id.into(),
            message: message.into(),
            live_count,
        })
    }

    /// Construct a `mission.validation` event.
    #[must_use]
    pub fn mission_validation(
        mission_id: impl Into<String>,
        milestone: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Event::MissionValidation(MissionValidation {
            mission_id: mission_id.into(),
            milestone: milestone.into(),
            outcome: outcome.into(),
        })
    }

    /// Construct a `mission.finished` event.
    #[must_use]
    pub fn mission_finished(
        mission_id: impl Into<String>,
        status: impl Into<String>,
        completed: usize,
        failed: usize,
    ) -> Self {
        Event::MissionFinished(MissionFinished {
            mission_id: mission_id.into(),
            status: status.into(),
            completed,
            failed,
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
    fn mcp_log_message_serializes_with_named_typed_fields() {
        let event = Event::mcp_log_message(
            "demo",
            "warning",
            Some("fixture".to_owned()),
            serde_json::json!({"message":"redacted"}),
        );
        assert_eq!(event.kind(), "mcp.log");
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["kind"], "mcp.log");
        assert_eq!(value["server"], "demo");
        assert_eq!(value["level"], "warning");
        assert_eq!(value["logger"], "fixture");
        assert_eq!(value["data"]["message"], "redacted");
    }

    #[test]
    fn tool_events_serialize_with_named_typed_fields() {
        let call = Event::tool_call("call_1", "cortex_echo", "{\"text\":\"hi\"}");
        assert_eq!(call.kind(), "tool.call");
        let value = serde_json::to_value(&call).expect("serialize");
        assert_eq!(value["kind"], "tool.call");
        assert_eq!(value["id"], "call_1");
        assert_eq!(value["name"], "cortex_echo");

        let result = Event::tool_result_with_parts(
            "call_1",
            "cortex_echo",
            vec![ToolResultPart::text("bounded projection")],
            Some(McpToolResult {
                blocks: vec![protocol::McpContentBlock::Text {
                    text: protocol::McpPayload::Inline("source text".to_owned()),
                    annotations: None,
                    metadata: None,
                }],
                structured_content: None,
                metadata: None,
                is_error: Some(false),
            }),
            false,
        );
        assert_eq!(result.kind(), "tool.result");
        let value = serde_json::to_value(&result).expect("serialize");
        assert_eq!(value["kind"], "tool.result");
        assert_eq!(value["is_error"], false);
        assert_eq!(value["parts"][0]["text"], "bounded projection");
        assert_eq!(value["mcp_result"]["blocks"][0]["text"], "source text");
    }

    #[test]
    fn all_kinds_is_unique_and_covers_the_reference_families() {
        let kinds = Event::all_kinds();
        let unique: std::collections::BTreeSet<&str> = kinds.iter().copied().collect();
        assert_eq!(
            unique.len(),
            kinds.len(),
            "the event taxonomy must not advertise duplicate kinds"
        );
        for required in [
            "message.updated",
            "message.part.updated",
            "session.next.text.start",
            "session.next.text.delta",
            "session.next.text.end",
            "session.next.tool.started",
            "session.next.tool.ended",
            "session.next.step.started",
            "session.next.retried",
            "session.next.context.updated",
            "session.next.model.switched",
            "session.next.agent.switched",
            "session.status",
            "session.idle",
            "session.error",
            "session.diff",
            "session.compacted",
            "question.asked",
            "todo.updated",
            "file.edited",
            "lsp.updated",
            "vcs.branch.updated",
            "project.updated",
            "command.executed",
            "installation.updated",
            "worktree.ready",
            "worktree.removed",
            "workspace.status",
            "pty.created",
            "tui.toast.show",
        ] {
            assert!(
                unique.contains(required),
                "taxonomy is missing required reference family {required}"
            );
        }
    }

    #[test]
    fn reference_family_constructors_carry_their_serde_tag() {
        let cases: Vec<(Event, &str)> = vec![
            (
                Event::message_updated("s", "m", "assistant", None),
                "message.updated",
            ),
            (Event::session_status("s", "busy"), "session.status"),
            (Event::session_idle("s"), "session.idle"),
            (
                Event::session_model_switched("s", "m", "google/gemini-2.5-flash"),
                "session.next.model.switched",
            ),
            (
                Event::session_compacted("s", "m", "tail-1", None),
                "session.compacted",
            ),
            (
                Event::command_executed(None, "echo hi", Some(0)),
                "command.executed",
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(event.kind(), expected, "kind() reports the family tag");
            let value = serde_json::to_value(&event).expect("serialize");
            assert_eq!(
                value["kind"], expected,
                "serde tag matches kind() for {expected}"
            );
            let decoded: Event = serde_json::from_value(value).expect("round-trip decode");
            assert_eq!(
                decoded.kind(),
                expected,
                "round-trips into the typed variant"
            );
        }
    }

    #[test]
    fn worktree_family_constructors_serialize_with_named_typed_fields() {
        let ready = Event::worktree_ready("/tmp/wt-feature");
        assert_eq!(ready.kind(), "worktree.ready");
        let value = serde_json::to_value(&ready).expect("serialize");
        assert_eq!(value["kind"], "worktree.ready");
        assert_eq!(value["path"], "/tmp/wt-feature");

        let removed = Event::worktree_removed("/tmp/wt-feature");
        assert_eq!(removed.kind(), "worktree.removed");
        let value = serde_json::to_value(&removed).expect("serialize");
        assert_eq!(value["kind"], "worktree.removed");
        assert_eq!(value["path"], "/tmp/wt-feature");
        let decoded: Event = serde_json::from_value(value).expect("round-trip decode");
        assert_eq!(decoded.kind(), "worktree.removed");

        let failed = Event::worktree_failed("/tmp/wt-feature", "fatal: already exists");
        assert_eq!(failed.kind(), "worktree.failed");
        let value = serde_json::to_value(&failed).expect("serialize");
        assert_eq!(value["kind"], "worktree.failed");
        assert_eq!(value["path"], "/tmp/wt-feature");
        assert_eq!(value["message"], "fatal: already exists");
    }
}
