//! Compatibility event families that broaden the SSE/event vocabulary beyond the turn-centric
//! core events.
//!
//! These payloads back the message/part lifecycle, the granular `session.next.*` streaming
//! markers, the session lifecycle (status/idle/error/diff/compacted), and the per-subsystem
//! families (question, todo, file, LSP, VCS, project, command, installation, worktree, workspace,
//! PTY, and TUI). Each is a strongly typed payload so a typed client can decode every family into a
//! distinct variant rather than special-casing server-only names. They serialize flat under the
//! shared `kind` tag of [`crate::Event`].

use protocol::MessagePart;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A session-scoped reference with no further payload (`session.updated`, `session.idle`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    /// Owning session id.
    pub session_id: String,
}

/// A persisted message changed (created or rewritten) for a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageUpdated {
    /// Owning session id (matches the REST session id).
    pub session_id: String,
    /// Persisted message id (matches the REST message id).
    pub message_id: String,
    /// Message role (`user`, `assistant`, ...).
    pub role: String,
    /// Persistence timestamp when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_created: Option<String>,
}

/// A persisted message was removed from a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRemoved {
    /// Owning session id.
    pub session_id: String,
    /// Removed message id.
    pub message_id: String,
}

/// A persisted message part changed (created or rewritten).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessagePartUpdated {
    /// Owning session id.
    pub session_id: String,
    /// Owning message id.
    pub message_id: String,
    /// Stable part id (best-effort; empty when persistence has not assigned one yet).
    pub part_id: String,
    /// The typed part payload.
    pub part: MessagePart,
}

/// A persisted message part was removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePartRemoved {
    /// Owning session id.
    pub session_id: String,
    /// Owning message id.
    pub message_id: String,
    /// Removed part id.
    pub part_id: String,
}

/// The start or end boundary of a granular streaming channel for one turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamBoundary {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
}

/// A granular streaming text/reasoning fragment for one turn (`session.next.*.delta`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamDelta {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// The (already-redacted) fragment.
    pub text: String,
}

/// A tool call started streaming for one turn (`session.next.tool.started`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStarted {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// Provider-assigned call id.
    pub id: String,
    /// Tool name.
    pub name: String,
}

/// A tool call finished for one turn (`session.next.tool.ended`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEnded {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// Provider-assigned call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Whether the tool reported an error.
    pub is_error: bool,
}

/// A step lifecycle boundary for one turn (`session.next.step.started`/`finished`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepEvent {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// 1-based step number.
    pub step: usize,
}

/// The runner scheduled a retry attempt for a turn (`session.next.retried`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRetried {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// 1-based retry attempt about to run.
    pub attempt: usize,
    /// Configured maximum retry attempts.
    pub max_attempts: usize,
    /// Delay before the next attempt, in milliseconds.
    pub delay_ms: u64,
    /// Stable retry classification (`rate_limit`, `service_unavailable`, ...).
    pub classification: String,
}

/// The turn's effective context-window usage changed (`session.next.context.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContext {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// Tokens accounted to the turn so far.
    pub tokens: u32,
    /// The model's context-window limit when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u32>,
}

/// The turn's active model was selected/switched (`session.next.model.switched`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionModelSwitched {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// The fully-qualified model id now in effect.
    pub model: String,
}

/// The turn's active agent was selected/switched (`session.next.agent.switched`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAgentSwitched {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id.
    pub message_id: String,
    /// The agent persona now in effect.
    pub agent: String,
}

/// A session's busy/idle status changed (`session.status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatusEvent {
    /// Owning session id.
    pub session_id: String,
    /// Stable status string (`busy` or `idle`).
    pub status: String,
}

/// A normalized session-level error (`session.error`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionErrorEvent {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id when the error belongs to a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Stable error class.
    pub error_kind: String,
    /// Human-readable, secret-scrubbed detail.
    pub message: String,
}

/// A session's working-tree diff summary changed (`session.diff`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDiffEvent {
    /// Owning session id.
    pub session_id: String,
    /// The changed file paths.
    #[serde(default)]
    pub files: Vec<String>,
}

/// A session's context was compacted through an LLM summary (`session.compacted`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCompactedEvent {
    /// Owning session id.
    pub session_id: String,
    /// Turn/message id that triggered the compaction.
    pub message_id: String,
    /// The tail-boundary token where preserved tail context begins.
    pub tail_start_id: String,
    /// The model used for compaction summarization, when explicitly selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A question was asked of the user (`question.asked`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAsked {
    /// Owning session id when the question belongs to a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Stable question id.
    pub id: String,
    /// The question text.
    pub question: String,
    /// Offered answer options, when enumerated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

/// A question was answered (`question.replied`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionReplied {
    /// Owning session id when the question belonged to a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Stable question id.
    pub id: String,
    /// The chosen answer.
    pub answer: String,
}

/// A question was rejected/dismissed (`question.rejected`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionRejected {
    /// Owning session id when the question belonged to a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Stable question id.
    pub id: String,
    /// Optional rejection reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A session's todo list changed (`todo.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoUpdated {
    /// Owning session id.
    pub session_id: String,
    /// The current todo items as a typed JSON array.
    pub todos: JsonValue,
}

/// A file was written/edited by a tool or route (`file.edited`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdited {
    /// Owning session id when the edit belongs to a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The edited path.
    pub path: String,
    /// The edit kind (`write` or `edit`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_kind: Option<String>,
}

/// A watched file changed on disk (`file.watcher.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWatcherUpdated {
    /// The changed path.
    pub path: String,
    /// The change kind (`created`, `modified`, `deleted`).
    pub change: String,
}

/// An LSP server produced new state/diagnostics (`lsp.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspUpdated {
    /// The LSP server name when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// The document path the update applies to, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The number of diagnostics now reported for the path, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_count: Option<usize>,
}

/// The VCS branch/dirty state changed (`vcs.branch.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VcsBranchUpdated {
    /// The current branch name.
    pub branch: String,
    /// Whether the working tree is dirty, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
}

/// A project's state changed (`project.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectUpdated {
    /// The project root path.
    pub path: String,
}

/// A command was executed (`command.executed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandExecuted {
    /// Owning session id when the command belongs to a session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The (redacted) command line.
    pub command: String,
    /// The process exit code, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// An installation/version transition (`installation.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationUpdated {
    /// The version now installed/active.
    pub version: String,
}

/// A worktree became ready (`worktree.ready`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeReady {
    /// The worktree path.
    pub path: String,
}

/// A worktree operation failed (`worktree.failed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeFailed {
    /// The worktree path.
    pub path: String,
    /// The secret-scrubbed failure detail.
    pub message: String,
}

/// A worktree was removed (`worktree.removed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRemoved {
    /// The removed worktree path.
    pub path: String,
}

/// A workspace became ready or failed (`workspace.ready`/`workspace.failed`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEvent {
    /// The workspace id when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// A workspace status transition (`workspace.status`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    /// The stable status string.
    pub status: String,
}

/// A PTY was created or updated (`pty.created`/`pty.updated`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyEvent {
    /// The PTY id.
    pub id: String,
    /// The shell program when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

/// A PTY was removed (`pty.deleted`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyRef {
    /// The PTY id.
    pub id: String,
}

/// A PTY process exited (`pty.exited`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyExited {
    /// The PTY id.
    pub id: String,
    /// The process exit code, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

/// A remote TUI requested text appended to the prompt (`tui.prompt.append`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiPromptAppend {
    /// The text to append.
    pub text: String,
}

/// A remote TUI requested a command be executed (`tui.command.execute`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiCommandExecute {
    /// The command to execute.
    pub command: String,
}

/// A remote TUI requested a toast be shown (`tui.toast.show`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiToastShow {
    /// The toast message.
    pub message: String,
    /// The toast level when known (`info`, `warn`, `error`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

/// A remote TUI requested a session be selected (`tui.session.select`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiSessionSelect {
    /// The session id to select.
    pub session_id: String,
}
