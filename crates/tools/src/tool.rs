//! The tool abstraction: the [`Tool`] trait, the per-session [`ToolContext`], and the structured
//! [`ToolOutput`] every tool produces.
//!
//! A [`Tool`] advertises a name, a model-facing description, and a JSON-Schema parameter object,
//! and executes an assembled arguments object against a [`ToolContext`]. The context carries the
//! session working directory, the per-session read-tracking set (the freshness ledger that the
//! mutating tools consult before overwriting a file), and the optional permission engine the tool
//! consults before producing an effect. A tool never aborts the turn: a failure is returned as a
//! [`ToolOutput`] with [`is_error`](ToolOutput::is_error) set so the model can recover.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use permissions::{PermissionEngine, ToolScope, request_for, scope_of};
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

/// Result of checking whether one tool action is permitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionCheck {
    /// Whether the action is permitted to proceed.
    pub allowed: bool,
    /// Model-visible deny message when `allowed` is false.
    pub message: Option<String>,
    /// Optional corrective feedback carried with the deny.
    pub feedback: Option<String>,
}

impl PermissionCheck {
    #[must_use]
    fn allowed() -> Self {
        Self::allowed_with_feedback(None)
    }

    #[must_use]
    fn allowed_with_feedback(feedback: Option<String>) -> Self {
        Self {
            allowed: true,
            message: None,
            feedback: feedback
                .map(|text| redaction::redact(&text))
                .filter(|text| !text.is_empty()),
        }
    }

    #[must_use]
    fn denied(tool: &str, target: &str, feedback: Option<String>) -> Self {
        let feedback = feedback
            .map(|text| redaction::redact(&text))
            .filter(|text| !text.is_empty());
        let mut message = format!("{tool}: permission denied for {target}");
        if let Some(text) = feedback.as_deref() {
            message.push_str("\nCorrection from user: ");
            message.push_str(text);
        }
        Self {
            allowed: false,
            message: Some(message),
            feedback,
        }
    }
}

#[derive(Debug, Default)]
struct PlanningState {
    active: bool,
}

/// The per-session execution context shared across every tool call in a turn.
///
/// It resolves relative paths against the session working directory, tracks which files have been
/// read (so a later mutating tool can require a prior read), and optionally gates a tool's effect
/// through the permission engine.
#[derive(Clone)]
pub struct ToolContext {
    cwd: PathBuf,
    external_allowlist_roots: Arc<Vec<PathBuf>>,
    reads: Arc<Mutex<HashSet<PathBuf>>>,
    permissions: Option<Arc<PermissionEngine>>,
    audit: Option<Arc<audit::AuditSink>>,
    hooks: Option<Arc<dyn MutationHooks>>,
    lsp: Option<Arc<dyn LspService>>,
    checkpoints: Option<Arc<dyn CheckpointRecorder>>,
    session_id: Option<String>,
    todos: Option<Arc<dyn TodoStore>>,
    task_runner: Option<Arc<dyn TaskRunner>>,
    background_jobs: Arc<BackgroundJobs>,
    shell_env: Arc<Vec<(OsString, OsString)>>,
    planning: Arc<Mutex<PlanningState>>,
    interactive_questions: bool,
}

impl ToolContext {
    /// Build a context rooted at `cwd` with an empty read-tracking set, no permission gating, no
    /// post-mutation hooks, no session, no todo store, and a fresh background-job registry.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        let cwd = canonicalize_identity_path(&cwd.into());
        let external_allowlist_roots = Arc::new(default_external_allowlist_roots(&cwd));
        Self {
            cwd,
            external_allowlist_roots,
            reads: Arc::new(Mutex::new(HashSet::new())),
            permissions: None,
            audit: None,
            hooks: None,
            lsp: None,
            checkpoints: None,
            session_id: None,
            todos: None,
            task_runner: None,
            background_jobs: Arc::new(BackgroundJobs::new()),
            shell_env: Arc::new(Vec::new()),
            planning: Arc::new(Mutex::new(PlanningState::default())),
            interactive_questions: false,
        }
    }

    /// Attach a permission engine that gates each tool's effect (builder style).
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<PermissionEngine>) -> Self {
        self.permissions = Some(engine);
        self
    }

    /// Attach the audit sink that records real tool, permission, network, and MCP activity.
    #[must_use]
    pub fn with_audit(mut self, audit: Arc<audit::AuditSink>) -> Self {
        self.audit = Some(audit);
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

    /// Attach additional environment variables to every `bash` child spawned by this context.
    ///
    /// The base bash environment is still secret-stripped first; these values are intentionally
    /// supplied by the current surface (for example, a plugin `shell.env` hook) and therefore are
    /// appended afterward.
    #[must_use]
    pub fn with_shell_env(mut self, env: Vec<(OsString, OsString)>) -> Self {
        self.shell_env = Arc::new(env);
        self
    }

    /// Enable or disable interactive question flows for this surface (builder style).
    ///
    /// Headless runs keep this disabled and fail closed; interactive surfaces (TUI/server) enable
    /// it so the `question` tool can request user input.
    #[must_use]
    pub fn with_interactive_questions(mut self, interactive: bool) -> Self {
        self.interactive_questions = interactive;
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

    /// Additional environment variables to inject into `bash` children.
    #[must_use]
    pub fn shell_env(&self) -> &[(OsString, OsString)] {
        self.shell_env.as_slice()
    }

    /// Whether this context represents an interactive surface that can ask user questions.
    #[must_use]
    pub fn interactive_questions_enabled(&self) -> bool {
        self.interactive_questions
            && self
                .permissions
                .as_ref()
                .is_some_and(|engine| engine.has_responder())
    }

    /// The attached audit sink, if the current surface records audit events.
    #[must_use]
    pub fn audit_sink(&self) -> Option<&Arc<audit::AuditSink>> {
        self.audit.as_ref()
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

    /// Enter model-visible planning mode for this turn/session context.
    pub fn enter_planning_mode(&self) {
        if let Ok(mut planning) = self.planning.lock() {
            planning.active = true;
        }
    }

    /// Exit planning mode. Returns an error when an approval flag is required but missing.
    pub fn exit_planning_mode(&self, approved: bool) -> Result<(), String> {
        if !approved {
            return Err(
                "plan-exit: set `approved` to true after the user accepts the plan".to_owned(),
            );
        }
        if let Ok(mut planning) = self.planning.lock() {
            planning.active = false;
        }
        Ok(())
    }

    /// Whether planning mode is currently active.
    #[must_use]
    pub fn planning_mode_active(&self) -> bool {
        self.planning
            .lock()
            .map(|planning| planning.active)
            .unwrap_or(false)
    }

    /// Refuse mutating actions while planning mode is active.
    pub fn ensure_not_planning(&self, tool: &str) -> Result<(), String> {
        if self.planning_mode_active() {
            return Err(format!(
                "{tool}: planning mode is active, call `plan-exit` or `plan_exit` with approved=true before mutating"
            ));
        }
        Ok(())
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
        let expanded = expand_user_path(path);
        let candidate = Path::new(&expanded);
        let resolved = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        };
        canonicalize_identity_path(&resolved)
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
        let key = self.identity_key(&path.into());
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(key);
        }
    }

    /// Whether `path` (a resolved absolute path) has been read this session.
    #[must_use]
    pub fn was_read(&self, path: &Path) -> bool {
        let key = self.identity_key(path);
        self.reads
            .lock()
            .map(|reads| reads.contains(&key))
            .unwrap_or(false)
    }

    /// Whether the permission engine (when attached) allows `tool` to act on `target`.
    ///
    /// With no engine attached every action is allowed, so a tool can run unconditionally until the
    /// enforcement layer wires a real engine in. With an engine attached the action's subject is
    /// scoped by the tool (a file tool scopes by path) and resolved to allow/deny.
    #[must_use]
    pub fn permits(&self, tool: &str, target: &str) -> bool {
        self.permission_check(tool, target).allowed
    }

    /// Require permission for `tool` acting on `target`, returning a model-visible deny message
    /// when blocked.
    pub fn ensure_permitted(&self, tool: &str, target: &str) -> Result<(), String> {
        let check = self.permission_check(tool, target);
        if check.allowed {
            Ok(())
        } else {
            Err(check
                .message
                .unwrap_or_else(|| format!("{tool}: permission denied for {target}")))
        }
    }

    /// Resolve permission for one action and return both the decision and any corrective feedback.
    #[must_use]
    pub fn permission_check(&self, tool: &str, target: &str) -> PermissionCheck {
        match &self.permissions {
            Some(engine) => self.permission_check_with_engine(engine, tool, target),
            None => PermissionCheck::allowed(),
        }
    }

    fn permission_target_for_path(&self, path: &Path) -> String {
        self.permission_subject_for_resolved_path(path)
    }

    fn permission_check_with_engine(
        &self,
        engine: &PermissionEngine,
        tool: &str,
        target: &str,
    ) -> PermissionCheck {
        if matches!(scope_of(tool), Some(ToolScope::Path)) {
            let resolved = self.resolve(target);
            if let Some(denied) = self.permit_external_directory(engine, &resolved) {
                return PermissionCheck::denied(tool, target, denied.feedback);
            }
            let scoped_target = self.permission_subject_for_resolved_path(&resolved);
            let request = request_for(tool, &scoped_target);
            let evaluation = self.evaluate_permission(engine, request);
            if evaluation.outcome.is_allowed() {
                return PermissionCheck::allowed_with_feedback(evaluation.feedback);
            }
            return PermissionCheck::denied(tool, target, evaluation.feedback);
        }
        let evaluation = self.evaluate_permission(engine, request_for(tool, target));
        if evaluation.outcome.is_allowed() {
            PermissionCheck::allowed_with_feedback(evaluation.feedback)
        } else {
            PermissionCheck::denied(tool, target, evaluation.feedback)
        }
    }

    fn evaluate_permission(
        &self,
        engine: &PermissionEngine,
        request: permissions::PermissionRequest,
    ) -> permissions::PermissionEvaluation {
        let evaluation = engine.evaluate_detail(&request);
        if let Some(audit) = &self.audit {
            audit.record_permission_decision(
                &request.tool,
                &evaluation.pattern,
                &request.subject,
                evaluation.action.as_str(),
                evaluation.outcome.as_str(),
            );
        }
        evaluation
    }

    fn permit_external_directory(
        &self,
        engine: &PermissionEngine,
        resolved: &Path,
    ) -> Option<permissions::PermissionEvaluation> {
        if resolved.starts_with(&self.cwd) || self.is_allowlisted_external_path(resolved) {
            return None;
        }
        let boundary = if resolved.is_dir() {
            resolved.to_path_buf()
        } else {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| resolved.to_path_buf())
        };
        let resource = boundary.join("*").to_string_lossy().to_string();
        let evaluation =
            self.evaluate_permission(engine, request_for("external_directory", &resource));
        (!evaluation.outcome.is_allowed()).then_some(evaluation)
    }

    fn permission_subject_for_resolved_path(&self, path: &Path) -> String {
        match path.strip_prefix(&self.cwd) {
            Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
            Ok(relative) => relative.to_string_lossy().to_string(),
            Err(_) => path.to_string_lossy().to_string(),
        }
    }

    fn is_allowlisted_external_path(&self, path: &Path) -> bool {
        self.external_allowlist_roots
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn identity_key(&self, path: &Path) -> PathBuf {
        let anchored = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        canonicalize_identity_path(&anchored)
    }
}

/// Canonicalize `path` for identity comparisons, including symlink aliases that point at a target
/// path that does not exist yet (for example, a future write target behind a dangling symlink).
fn canonicalize_identity_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    resolve_symlink_alias_path(path).unwrap_or_else(|| canonicalize_via_existing_ancestor(path))
}

const MAX_SYMLINK_EXPANSIONS: usize = 40;

/// Resolve symlink aliases even when the final target does not exist yet, returning an absolute
/// path that can be used for permission matching and read-ledger identity.
fn resolve_symlink_alias_path(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    resolve_symlink_alias_path_inner(absolute, 0)
}

fn resolve_symlink_alias_path_inner(path: PathBuf, depth: usize) -> Option<PathBuf> {
    if depth > MAX_SYMLINK_EXPANSIONS {
        return None;
    }

    let mut resolved = PathBuf::new();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
        use std::path::Component;

        match component {
            Component::Prefix(_) | Component::RootDir => {
                resolved.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                let candidate = resolved.join(name);
                if let Ok(metadata) = fs::symlink_metadata(&candidate)
                    && metadata.file_type().is_symlink()
                {
                    let target = fs::read_link(&candidate).ok()?;
                    let mut rerooted = if target.is_absolute() {
                        target
                    } else {
                        resolved.join(target)
                    };
                    for remaining in components {
                        rerooted.push(remaining.as_os_str());
                    }
                    return resolve_symlink_alias_path_inner(rerooted, depth + 1);
                }
                resolved.push(name);
            }
        }
    }

    Some(resolved)
}

/// Fallback canonicalization by resolving the nearest existing ancestor and appending the
/// unresolved suffix.
fn canonicalize_via_existing_ancestor(path: &Path) -> PathBuf {
    let mut probe = path.to_path_buf();
    let mut suffix = Vec::new();
    while !probe.exists() {
        let Some(name) = probe.file_name().map(|name| name.to_os_string()) else {
            break;
        };
        suffix.push(name);
        if !probe.pop() {
            break;
        }
    }

    if let Ok(mut canonical) = fs::canonicalize(&probe) {
        for component in suffix.iter().rev() {
            canonical.push(component);
        }
        canonical
    } else {
        path.to_path_buf()
    }
}

fn default_external_allowlist_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_allowlist_root(&mut roots, std::env::temp_dir());

    let mut inputs = match settings::ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs,
        Err(_) => return roots,
    };
    inputs.cwd = cwd.to_path_buf();

    let discovery = skills::discover(&inputs);
    for skill in discovery.skills.values() {
        if let Some(base_dir) = skill.source.parent() {
            push_allowlist_root(&mut roots, base_dir.to_path_buf());
            push_allowlist_root(&mut roots, base_dir.join("reference"));
            push_allowlist_root(&mut roots, base_dir.join("references"));
        }
    }

    if let Ok(effective) = settings::load(inputs.clone()) {
        for key in ["references", "reference"] {
            if let Some(value) = effective.get(key) {
                append_reference_roots(&mut roots, value, cwd, inputs.home.as_deref());
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

fn append_reference_roots(
    roots: &mut Vec<PathBuf>,
    value: &toml::Value,
    cwd: &Path,
    home: Option<&Path>,
) {
    match value {
        toml::Value::String(raw) => {
            if let Some(path) = resolve_local_reference_path(raw, cwd, home) {
                push_allowlist_root(roots, path);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                append_reference_roots(roots, value, cwd, home);
            }
        }
        toml::Value::Table(table) => {
            if let Some(path) = table.get("path").and_then(toml::Value::as_str)
                && let Some(path) = resolve_local_reference_path(path, cwd, home)
            {
                push_allowlist_root(roots, path);
            }
            for value in table.values() {
                append_reference_roots(roots, value, cwd, home);
            }
        }
        _ => {}
    }
}

fn resolve_local_reference_path(value: &str, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.starts_with("~/") {
        return home.map(|home| home.join(&value[2..]));
    }
    if value.starts_with('/') {
        return Some(PathBuf::from(value));
    }
    if value.starts_with("./") || value.starts_with("../") {
        return Some(cwd.join(value));
    }
    None
}

fn push_allowlist_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    roots.push(canonicalize_identity_path(&root));
}

fn expand_user_path(path: &str) -> String {
    let trimmed = path.trim();
    let Some(home) = std::env::var_os("HOME") else {
        return trimmed.to_owned();
    };
    let home = PathBuf::from(home).to_string_lossy().to_string();
    if trimmed == "~" {
        return home;
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    if trimmed == "$HOME" || trimmed == "${HOME}" {
        return home;
    }
    if let Some(rest) = trimmed.strip_prefix("$HOME/") {
        return format!("{home}/{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix("${HOME}/") {
        return format!("{home}/{rest}");
    }
    trimmed.to_owned()
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

#[cfg(test)]
mod tests {
    use super::*;
    use permissions::{
        Action, Decision, PermissionEngine, PermissionRequest, Responder, Rule, Ruleset,
        ScopedDecision,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-tool-audit-{tag}-{}-{unique}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn workspace_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("tool-context-tests");
        let dir = base.join(format!("cortex-tool-{tag}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create workspace dir");
        dir
    }

    #[test]
    fn permits_records_permission_decision_details() {
        let dir = temp_dir("permission");
        let audit = Arc::new(audit::AuditSink::new(dir.join("audit.jsonl"), "ses_perm"));
        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("write", "src/**", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ])));
        let ctx = ToolContext::new(dir.clone())
            .with_permissions(engine)
            .with_audit(audit.clone());

        assert!(ctx.permits("write", "src/lib.rs"));
        assert!(!ctx.permits("bash", "rm target"));

        let records = audit::read_jsonl(audit.path()).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "permission_decision"
                && record["tool"] == "write"
                && record["pattern"] == "src/**"
                && record["decision"] == "allow"
                && record["outcome"] == "allowed"
        }));
        assert!(records.iter().any(|record| {
            record["type"] == "permission_decision"
                && record["tool"] == "bash"
                && record["pattern"] == "rm *"
                && record["decision"] == "deny"
                && record["outcome"] == "denied"
        }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ensure_permitted_includes_deny_feedback_from_responder() {
        struct FeedbackResponder;

        impl Responder for FeedbackResponder {
            fn respond(&self, _request: &PermissionRequest) -> Decision {
                Decision::Deny
            }

            fn respond_scoped(&self, _request: &PermissionRequest) -> ScopedDecision {
                ScopedDecision::once(Decision::Deny).with_feedback("Use ls before deleting.")
            }
        }

        let dir = temp_dir("permission-feedback");
        let engine = Arc::new(
            PermissionEngine::new(Ruleset::new(vec![Rule::new("bash", "*", Action::Ask)]))
                .with_responder(Arc::new(FeedbackResponder)),
        );
        let ctx = ToolContext::new(dir.clone()).with_permissions(engine);

        let denied = ctx
            .ensure_permitted("bash", "rm target.txt")
            .expect_err("permission should be denied");
        assert!(denied.contains("bash: permission denied for rm target.txt"));
        assert!(denied.contains("Correction from user: Use ls before deleting."));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_and_read_ledger_use_canonical_identity_for_dot_paths() {
        let dir = temp_dir("canonical-dot");
        let file = dir.join("sample.txt");
        std::fs::write(&file, "content").expect("seed file");
        let ctx = ToolContext::new(dir.clone());

        let plain = ctx.resolve("sample.txt");
        let dotted = ctx.resolve("./sample.txt");
        assert_eq!(plain, dotted, "dot-path aliases resolve identically");

        ctx.record_read(dotted);
        assert!(
            ctx.was_read(&plain),
            "a read recorded via ./path is honored via path"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_and_read_ledger_canonicalize_symlink_aliases() {
        let dir = temp_dir("canonical-symlink");
        let target = dir.join("real.txt");
        let alias = dir.join("alias.txt");
        std::fs::write(&target, "content").expect("seed file");
        std::os::unix::fs::symlink(&target, &alias).expect("create symlink");
        let ctx = ToolContext::new(dir.clone());

        let direct = ctx.resolve("real.txt");
        let via_alias = ctx.resolve("alias.txt");
        assert_eq!(direct, via_alias, "symlink aliases resolve identically");

        ctx.record_read(via_alias);
        assert!(
            ctx.was_read(&direct),
            "a read recorded via symlink path is honored via target path"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn outside_workspace_paths_require_external_directory_permission() {
        let workspace = workspace_dir("external-gate");
        let outside = workspace
            .parent()
            .expect("workspace has parent")
            .join(format!(
                "outside-{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("secret.txt");
        if let Some(parent) = outside.parent() {
            std::fs::create_dir_all(parent).expect("create outside parent");
        }
        std::fs::write(&outside, "secret").expect("seed outside file");
        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Ask),
        ])));
        let ctx = ToolContext::new(workspace.clone()).with_permissions(engine);

        assert!(
            !ctx.permits("read", &outside.to_string_lossy()),
            "outside workspace path should be denied without an external_directory responder"
        );
        assert!(
            ctx.permits("read", "inside.txt"),
            "inside workspace path should stay allowed"
        );
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(
            outside
                .parent()
                .expect("outside file has parent")
                .to_path_buf(),
        );
    }

    #[test]
    fn temp_paths_are_allowlisted_from_external_directory_asks() {
        let workspace = workspace_dir("external-allow-temp");
        let temp_file = std::env::temp_dir()
            .join(format!(
                "cortex-tool-temp-{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("spill.txt");
        if let Some(parent) = temp_file.parent() {
            std::fs::create_dir_all(parent).expect("create temp parent");
        }
        std::fs::write(&temp_file, "spill").expect("seed temp file");
        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Ask),
        ])));
        let ctx = ToolContext::new(workspace.clone()).with_permissions(engine);

        assert!(
            ctx.permits("read", &temp_file.to_string_lossy()),
            "temp path should bypass external_directory ask via allowlist"
        );
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(
            temp_file
                .parent()
                .expect("temp file has parent")
                .to_path_buf(),
        );
    }

    #[test]
    fn resolve_expands_home_aliases() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let workspace = workspace_dir("resolve-home");
        let ctx = ToolContext::new(workspace.clone());
        let expected = canonicalize_identity_path(&PathBuf::from(home).join("cortex-home-path"));

        assert_eq!(ctx.resolve("~/cortex-home-path"), expected);
        assert_eq!(ctx.resolve("$HOME/cortex-home-path"), expected);
        assert_eq!(ctx.resolve("${HOME}/cortex-home-path"), expected);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[cfg(unix)]
    #[test]
    fn broken_symlink_targets_resolve_before_external_directory_matching() {
        let workspace = workspace_dir("external-broken-link");
        let outside_root = workspace
            .parent()
            .expect("workspace has parent")
            .join(format!(
                "outside-broken-{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&outside_root).expect("create outside root");
        let outside_target = outside_root.join("new-secret.txt");
        let alias = workspace.join("escape-link.txt");
        std::os::unix::fs::symlink(&outside_target, &alias).expect("create dangling symlink");

        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Ask),
        ])));
        let ctx = ToolContext::new(workspace.clone()).with_permissions(engine);
        let alias_arg = alias.to_string_lossy().to_string();

        assert_eq!(
            ctx.resolve(&alias_arg),
            outside_target,
            "dangling symlink should resolve to its outside target path"
        );
        assert!(
            !ctx.permits("write", &alias_arg),
            "write through dangling outside symlink should require external_directory permission"
        );

        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(outside_root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_parent_paths_resolve_before_external_directory_matching() {
        let workspace = workspace_dir("external-parent-symlink");
        let outside_root = workspace
            .parent()
            .expect("workspace has parent")
            .join(format!(
                "outside-parent-{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&outside_root).expect("create outside root");
        let alias_dir = workspace.join("outside-dir");
        std::os::unix::fs::symlink(&outside_root, &alias_dir).expect("create dir symlink");
        let alias_file = alias_dir.join("created-by-alias.txt");
        let outside_target = outside_root.join("created-by-alias.txt");

        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Ask),
        ])));
        let ctx = ToolContext::new(workspace.clone()).with_permissions(engine);
        let alias_arg = alias_file.to_string_lossy().to_string();

        assert_eq!(
            ctx.resolve(&alias_arg),
            outside_target,
            "paths under a symlinked parent should resolve to the outside target path"
        );
        assert!(
            !ctx.permits("apply_patch", &alias_arg),
            "apply_patch through symlinked outside parent should require external_directory permission"
        );

        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(outside_root);
    }

    #[cfg(unix)]
    #[test]
    fn inside_workspace_symlink_targets_stay_allowed() {
        let workspace = workspace_dir("inside-symlink");
        let target = workspace.join("real.txt");
        std::fs::write(&target, "inside").expect("seed inside target");
        let alias = workspace.join("alias.txt");
        std::os::unix::fs::symlink(&target, &alias).expect("create inside symlink");

        let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Ask),
        ])));
        let ctx = ToolContext::new(workspace.clone()).with_permissions(engine);
        let alias_arg = alias.to_string_lossy().to_string();

        assert!(
            ctx.permits("read", &alias_arg),
            "inside-workspace symlink target should not trip external_directory gating"
        );

        let _ = std::fs::remove_dir_all(workspace);
    }
}
