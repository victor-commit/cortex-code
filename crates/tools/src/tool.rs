//! The tool abstraction: the [`Tool`] trait, the per-session [`ToolContext`], and the structured
//! [`ToolOutput`] every tool produces.
//!
//! A [`Tool`] advertises a name, a model-facing description, and a JSON-Schema parameter object,
//! and executes an assembled arguments object against a [`ToolContext`]. The context carries the
//! session working directory, the per-session read-tracking set (the freshness ledger that the
//! mutating tools consult before overwriting a file), and the optional permission engine the tool
//! consults before producing an effect. A tool never aborts the turn: a failure is returned as a
//! [`ToolOutput`] with [`is_error`](ToolOutput::is_error) set so the model can recover.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use permissions::{PermissionEngine, request_for};
use protocol::{TodoItem, ToolResultPart};

use crate::bash::BackgroundJobs;

/// A diagnostic an LSP server reports for a file a mutating tool just wrote or edited.
///
/// The mutating tools re-inject these into their result so the model sees an error it introduced.
/// Positions are 1-based for model readability; `severity` is a short label (for example
/// `error`/`warning`/`information`/`hint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The severity label (e.g. `error`, `warning`).
    pub severity: String,
    /// The 1-based line the diagnostic range starts on.
    pub line: usize,
    /// The 1-based character the diagnostic range starts at.
    pub character: usize,
    /// The 1-based line the diagnostic range ends on.
    pub end_line: usize,
    /// The 1-based character the diagnostic range ends at.
    pub end_character: usize,
    /// Optional server/source label (e.g. `rustc`, `Ruff`).
    pub source: Option<String>,
    /// Optional diagnostic code.
    pub code: Option<String>,
    /// The human-readable diagnostic message.
    pub message: String,
}

impl Diagnostic {
    /// Build a diagnostic at `line` with `severity` and `message`.
    #[must_use]
    pub fn new(severity: impl Into<String>, line: usize, message: impl Into<String>) -> Self {
        Self::with_range(severity, line, 1, line, 1, message)
    }

    /// Build a diagnostic with a precise 1-based range.
    #[must_use]
    pub fn with_range(
        severity: impl Into<String>,
        line: usize,
        character: usize,
        end_line: usize,
        end_character: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: severity.into(),
            line,
            character,
            end_line,
            end_character,
            source: None,
            code: None,
            message: message.into(),
        }
    }

    /// Add source/code metadata from a language server.
    #[must_use]
    pub fn with_metadata(mut self, source: Option<String>, code: Option<String>) -> Self {
        self.source = source;
        self.code = code;
        self
    }

    /// Render the diagnostic as one stable, model-readable line including severity and precise range.
    #[must_use]
    pub fn render(&self) -> String {
        let code = self
            .code
            .as_deref()
            .map(|code| format!(" [{code}]"))
            .unwrap_or_default();
        let source = self
            .source
            .as_deref()
            .map(|source| format!(" ({source})"))
            .unwrap_or_default();
        if self.line == self.end_line {
            format!(
                "{}{code}{source} at line {}, character {}-{}: {}",
                self.severity, self.line, self.character, self.end_character, self.message
            )
        } else {
            format!(
                "{}{code}{source} at line {}, character {} to line {}, character {}: {}",
                self.severity,
                self.line,
                self.character,
                self.end_line,
                self.end_character,
                self.message
            )
        }
    }
}

/// Post-mutation diagnostics plus an optional non-diagnostic note (for example, a missing or
/// network-disabled language server). A note is surfaced to the model without pretending it is a
/// file diagnostic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticFeedback {
    /// Real file diagnostics reported by the language server.
    pub diagnostics: Vec<Diagnostic>,
    /// Optional informational note about LSP availability.
    pub note: Option<String>,
}

impl DiagnosticFeedback {
    /// Feedback with only diagnostics and no note.
    #[must_use]
    pub fn diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            diagnostics,
            note: None,
        }
    }

    /// Append an informational note to this feedback, preserving any existing note.
    pub fn push_note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if note.is_empty() {
            return;
        }
        match &mut self.note {
            Some(existing) if !existing.is_empty() => {
                existing.push('\n');
                existing.push_str(&note);
            }
            Some(existing) => *existing = note,
            None => self.note = Some(note),
        }
    }
}

/// The post-write/post-edit downstream seam: a formatter pass over the written file and the LSP
/// diagnostics re-injected into the tool result.
///
/// The mutating tools (`write`, later `edit`) call these after a successful mutation. The
/// implementation is owned outside `tools`: surfaces attach formatter and/or LSP implementations
/// when those capabilities are available. With no hooks attached, [`format`](MutationHooks::format)
/// is a no-op (the file is written verbatim) and [`diagnostics`](MutationHooks::diagnostics)
/// returns none (a write with no LSP available succeeds unchanged with no error). The default method
/// bodies make a partial implementation (formatter-only, or LSP-only) trivial.
pub trait MutationHooks: Send + Sync {
    /// Reformat the just-written `path` in place. The default leaves the file untouched.
    fn format(&self, _path: &Path) {}

    /// Reformat the just-written `path` and optionally return a model-visible note, for example a
    /// formatter failure that preserved the raw written bytes. The default adapts older
    /// implementations that only override [`format`](MutationHooks::format).
    fn format_feedback(&self, path: &Path) -> Option<String> {
        self.format(path);
        None
    }

    /// Diagnostics and optional LSP availability note for the just-written `path` to re-inject into
    /// the tool result. The default returns none (no LSP available is not an error).
    fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
        DiagnosticFeedback::diagnostics(self.diagnostics(path))
    }

    /// Diagnostics for older hook implementations. New implementations should override
    /// [`diagnostic_feedback`](MutationHooks::diagnostic_feedback) so they can also surface a clean
    /// no-LSP note when appropriate.
    fn diagnostics(&self, _path: &Path) -> Vec<Diagnostic> {
        Vec::new()
    }
}

/// Read-only LSP operation requested by the `lsp` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspOperation {
    /// Return hover text for a symbol position.
    Hover,
    /// Resolve definition locations for a symbol position.
    Definition,
    /// Resolve reference locations for a symbol position.
    References,
    /// List document symbols for a file.
    DocumentSymbols,
    /// Search workspace symbols by query.
    WorkspaceSymbols,
}

/// Parsed request passed from the model-facing `lsp` tool to the surface-provided LSP service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspRequest {
    /// Operation to run.
    pub operation: LspOperation,
    /// Optional file path. Required for hover/definition/references/document symbols, optional for
    /// workspace symbols as a server-selection hint.
    pub file_path: Option<String>,
    /// 1-based line for position-based operations.
    pub line: Option<u32>,
    /// 1-based character for position-based operations.
    pub character: Option<u32>,
    /// Workspace-symbol query.
    pub query: Option<String>,
    /// Whether references should include the declaration (default true).
    pub include_declaration: bool,
}

/// Surface-owned LSP query seam used by the read-only `lsp` tool.
///
/// The tool layer owns argument parsing and tool materialization; the CLI/TUI/server surface owns
/// the real language-server process pool and cleanup, so it implements this trait and attaches it to
/// the [`ToolContext`].
pub trait LspService: Send + Sync {
    /// Execute one LSP request and return the exact tool output to show the model.
    fn query(&self, request: LspRequest) -> ToolOutput;
}

/// The pre-mutation checkpoint seam the mutating file tools call before they touch the working
/// tree.
///
/// Before `write`/`edit`/`apply_patch` (and downstream formatters) change a file, they call
/// [`record_pre_image`](CheckpointRecorder::record_pre_image) with the file they are about to
/// mutate so a snapshot of its prior state is captured for `/undo`. The implementation is owned
/// outside `tools` (the `checkpoints` crate, bridged by the CLI) so the tool layer stays decoupled
/// from the shadow-git store. With no recorder attached the call is a no-op, so a tool runs
/// unchanged until a surface wires checkpointing in. Capture is best-effort by contract: it never
/// blocks or fails a mutation (a capture failure degrades undo for the turn, surfaced when the turn
/// is finalized by the owner).
pub trait CheckpointRecorder: Send + Sync {
    /// Capture the pre-mutation image of `path` (a resolved absolute path) the first time the
    /// current turn touches it. MUST be called before the mutation's bytes hit the disk.
    fn record_pre_image(&self, path: &Path);
}

/// The per-session persistence seam the `todo` tool writes through.
///
/// The `todo` tool maintains a structured task list scoped to the active session. Persistence is
/// owned outside `tools` (the `db` repository, bridged by the CLI) so the tool stays decoupled from
/// SQLite: it reads and replaces the list through this trait keyed by the session id. The list is
/// replaced wholesale on each write (the model commits the complete list it wants), and reads
/// return the stored items in order. With no store attached (e.g. a context built without a durable
/// session) the `todo` tool reports a clear error rather than silently dropping the list.
pub trait TodoStore: Send + Sync {
    /// The session's todo items in stored order (empty for a session with none).
    ///
    /// # Errors
    /// Returns a human-readable message when the list cannot be read.
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String>;

    /// Replace the session's entire todo list with `items`, in order.
    ///
    /// # Errors
    /// Returns a human-readable message when the list cannot be written.
    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String>;
}

/// The structured result of running a tool.
///
/// The parts are surfaced verbatim to machine consumers; [`model_content`](ToolOutput::model_content)
/// renders the text the model reads back. A non-text attachment (an image/PDF) is carried as a typed
/// part so binary bytes never enter the text channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// The structured result parts (text and/or attachment references).
    pub parts: Vec<ToolResultPart>,
    /// Whether the tool reported an error (the turn still continues).
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful result carrying a single text part.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![ToolResultPart::text(text)],
            is_error: false,
        }
    }

    /// An error result carrying a single text part describing the failure.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            parts: vec![ToolResultPart::text(text)],
            is_error: true,
        }
    }

    /// A successful result built from explicit parts.
    #[must_use]
    pub fn parts(parts: Vec<ToolResultPart>) -> Self {
        Self {
            parts,
            is_error: false,
        }
    }

    /// The text the model reads back: every text part joined by newlines, with any attachment
    /// described by its media type so a text-only transcript still records what was returned.
    #[must_use]
    pub fn model_content(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for part in &self.parts {
            match part {
                ToolResultPart::Text { text } => lines.push(text.clone()),
                ToolResultPart::Image { media_type, path } => {
                    lines.push(format!("[image attachment: {path} ({media_type})]"));
                }
                ToolResultPart::Document { media_type, path } => {
                    lines.push(format!("[document attachment: {path} ({media_type})]"));
                }
            }
        }
        lines.join("\n")
    }
}

/// One model-selectable subagent target the `task` tool may spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTarget {
    /// The canonical agent name the model passes as the `agent` argument.
    pub name: String,
    /// A short description shown in the `task` tool description so the model can choose a target.
    pub description: String,
}

impl TaskTarget {
    /// Build a task target from a name and description.
    #[must_use]
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
        }
    }
}

/// A subagent invocation requested by the `task` tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRequest {
    /// The target subagent type to run.
    pub agent: String,
    /// The work delegated to the child session.
    pub prompt: String,
}

impl TaskRequest {
    /// Build a task request.
    #[must_use]
    pub fn new(agent: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            prompt: prompt.into(),
        }
    }
}

/// Surface-owned subagent runner used by the `task` tool.
///
/// The tool layer owns argument parsing and permission checks, but the surface owns durable
/// sessions, provider credentials, and process cleanup. A surface attaches a runner to
/// [`ToolContext`], and the `task` tool calls it to execute the child turn for real.
pub trait TaskRunner: Send + Sync {
    /// Run one subagent request and return the model-visible task result.
    fn run_task<'a>(&'a self, request: TaskRequest) -> BoxFuture<'a, ToolOutput>;
}

/// The per-session execution context shared across every tool call in a turn.
///
/// It resolves relative paths against the session working directory, tracks which files have been
/// read (so a later mutating tool can require a prior read), and optionally gates a tool's effect
/// through the permission engine.
#[derive(Clone)]
pub struct ToolContext {
    cwd: PathBuf,
    reads: Arc<Mutex<HashSet<PathBuf>>>,
    permissions: Option<Arc<PermissionEngine>>,
    hooks: Option<Arc<dyn MutationHooks>>,
    lsp: Option<Arc<dyn LspService>>,
    checkpoints: Option<Arc<dyn CheckpointRecorder>>,
    session_id: Option<String>,
    todos: Option<Arc<dyn TodoStore>>,
    task_runner: Option<Arc<dyn TaskRunner>>,
    background_jobs: Arc<BackgroundJobs>,
}

impl ToolContext {
    /// Build a context rooted at `cwd` with an empty read-tracking set, no permission gating, no
    /// post-mutation hooks, no session, no todo store, and a fresh background-job registry.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            reads: Arc::new(Mutex::new(HashSet::new())),
            permissions: None,
            hooks: None,
            lsp: None,
            checkpoints: None,
            session_id: None,
            todos: None,
            task_runner: None,
            background_jobs: Arc::new(BackgroundJobs::new()),
        }
    }

    /// Attach a permission engine that gates each tool's effect (builder style).
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<PermissionEngine>) -> Self {
        self.permissions = Some(engine);
        self
    }

    /// Set the session id tool calls are scoped to (builder style). The `todo` tool keys its
    /// per-session list off this id.
    #[must_use]
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach the per-session todo persistence the `todo` tool writes through (builder style). With
    /// none attached the `todo` tool reports a clear error rather than dropping the list.
    #[must_use]
    pub fn with_todo_store(mut self, store: Arc<dyn TodoStore>) -> Self {
        self.todos = Some(store);
        self
    }

    /// Attach the surface-owned subagent runner used by the `task` tool (builder style).
    #[must_use]
    pub fn with_task_runner(mut self, runner: Arc<dyn TaskRunner>) -> Self {
        self.task_runner = Some(runner);
        self
    }

    /// The session id tool calls are scoped to, if one was set.
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The attached per-session todo store, if one was set.
    #[must_use]
    pub fn todo_store(&self) -> Option<&Arc<dyn TodoStore>> {
        self.todos.as_ref()
    }

    /// The attached subagent runner, if the surface supports the `task` tool.
    #[must_use]
    pub fn task_runner(&self) -> Option<&Arc<dyn TaskRunner>> {
        self.task_runner.as_ref()
    }

    /// The background-job registry tracking long-running `bash` jobs spawned this session, so a
    /// surface can inspect and reap them on shutdown (no orphan process survives).
    #[must_use]
    pub fn background_jobs(&self) -> &Arc<BackgroundJobs> {
        &self.background_jobs
    }

    /// Attach the post-mutation downstream hooks (formatter + LSP diagnostics) used after a write or
    /// edit (builder style). With none attached the hooks are no-ops.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Arc<dyn MutationHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Attach a read-only LSP service for the `lsp` navigation tool.
    #[must_use]
    pub fn with_lsp_service(mut self, service: Arc<dyn LspService>) -> Self {
        self.lsp = Some(service);
        self
    }

    /// The attached read-only LSP service, if any.
    #[must_use]
    pub fn lsp_service(&self) -> Option<&Arc<dyn LspService>> {
        self.lsp.as_ref()
    }

    /// Attach the pre-mutation checkpoint recorder the mutating tools call before they touch a file
    /// (builder style). With none attached [`checkpoint_before`](ToolContext::checkpoint_before) is
    /// a no-op, so a tool runs unchanged until a surface wires checkpointing in.
    #[must_use]
    pub fn with_checkpoints(mut self, recorder: Arc<dyn CheckpointRecorder>) -> Self {
        self.checkpoints = Some(recorder);
        self
    }

    /// Run the post-write formatter over `path` (a resolved absolute path) through the attached
    /// hooks. A no-op when no hooks are attached, so the file is left exactly as written.
    pub fn format_after_write(&self, path: &Path) -> Option<String> {
        let target = self.permission_target_for_path(path);
        if !self.permits("format", &target) {
            return None;
        }
        if let Some(hooks) = &self.hooks {
            return hooks.format_feedback(path);
        }
        None
    }

    /// Collect LSP diagnostics and an optional availability note for `path` through the attached
    /// hooks. Returns empty feedback when no hooks are attached.
    #[must_use]
    pub fn diagnostic_feedback_after_write(&self, path: &Path) -> DiagnosticFeedback {
        self.hooks
            .as_ref()
            .map(|hooks| hooks.diagnostic_feedback(path))
            .unwrap_or_default()
    }

    /// Collect LSP diagnostics for `path` through the attached hooks.
    /// Returns none when no hooks are attached (no LSP available is not an error).
    #[must_use]
    pub fn diagnostics_after_write(&self, path: &Path) -> Vec<Diagnostic> {
        self.diagnostic_feedback_after_write(path).diagnostics
    }

    /// Record the pre-mutation snapshot of `path` (a resolved absolute path) through the attached
    /// checkpoint recorder, so the prior state is recoverable via `/undo`. MUST be called before the
    /// mutation writes. A no-op when no recorder is attached, and best-effort otherwise (it never
    /// blocks or fails the mutation).
    pub fn checkpoint_before(&self, path: &Path) {
        if let Some(recorder) = &self.checkpoints {
            recorder.record_pre_image(path);
        }
    }

    /// The session working directory relative paths resolve against.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Resolve `path` against the working directory: an absolute path is returned as-is; a relative
    /// path is joined onto the working directory so the same relative path denotes the same file
    /// across every tool in the session.
    #[must_use]
    pub fn resolve(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        }
    }

    /// Report a permission/IO reason when the OS would not accept new content at `resolved`, so a
    /// mutating tool refuses cleanly and leaves the existing file byte-for-byte intact instead of
    /// relying on the write syscall to fail.
    ///
    /// An existing file whose permission bits are read-only, or a not-yet-existing target whose
    /// existing parent directory is read-only, yields a reason. This pre-check is required because a
    /// privileged process (e.g. `root`) bypasses the permission bits, so the write syscall would
    /// otherwise silently succeed against a target the contract treats as OS-unwritable. Returns
    /// `None` when the target is writable (or when writability cannot be determined here, in which
    /// case the write syscall's own error still surfaces a clean failure).
    #[must_use]
    pub fn writability_error(&self, resolved: &Path) -> Option<String> {
        match fs::metadata(resolved) {
            Ok(metadata) => metadata
                .permissions()
                .readonly()
                .then(|| format!("{} is read-only (permission denied)", resolved.display())),
            Err(_) => resolved.parent().and_then(|parent| {
                fs::metadata(parent).ok().and_then(|meta| {
                    meta.permissions().readonly().then(|| {
                        format!(
                            "the parent directory of {} is read-only (permission denied)",
                            resolved.display()
                        )
                    })
                })
            }),
        }
    }

    /// Record that `path` (a resolved absolute path) has been read this session.
    pub fn record_read(&self, path: impl Into<PathBuf>) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path.into());
        }
    }

    /// Whether `path` (a resolved absolute path) has been read this session.
    #[must_use]
    pub fn was_read(&self, path: &Path) -> bool {
        self.reads
            .lock()
            .map(|reads| reads.contains(path))
            .unwrap_or(false)
    }

    /// Whether the permission engine (when attached) allows `tool` to act on `target`.
    ///
    /// With no engine attached every action is allowed, so a tool can run unconditionally until the
    /// enforcement layer wires a real engine in. With an engine attached the action's subject is
    /// scoped by the tool (a file tool scopes by path) and resolved to allow/deny.
    #[must_use]
    pub fn permits(&self, tool: &str, target: &str) -> bool {
        match &self.permissions {
            Some(engine) => engine.evaluate(&request_for(tool, target)).is_allowed(),
            None => true,
        }
    }

    fn permission_target_for_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.cwd)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }
}

/// A callable agent tool.
pub trait Tool: Send + Sync {
    /// The tool name the model calls (e.g. `read`).
    fn name(&self) -> &str;

    /// The model-facing description of when and how to use the tool.
    fn description(&self) -> &str;

    /// The JSON Schema describing the tool's arguments object.
    fn parameters(&self) -> serde_json::Value;

    /// Execute the tool with an assembled `arguments` object against `ctx`.
    fn execute<'a>(
        &'a self,
        arguments: serde_json::Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, ToolOutput>;
}
