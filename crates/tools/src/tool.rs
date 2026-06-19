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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use permissions::{PermissionEngine, request_for};
use protocol::{TodoItem, ToolResultPart};

/// A diagnostic an LSP server reports for a file a mutating tool just wrote or edited.
///
/// The mutating tools re-inject these into their result so the model sees an error it introduced.
/// `line` is 1-based; `severity` is a short label (e.g. `error`/`warning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The severity label (e.g. `error`, `warning`).
    pub severity: String,
    /// The 1-based line the diagnostic points at.
    pub line: usize,
    /// The human-readable diagnostic message.
    pub message: String,
}

impl Diagnostic {
    /// Build a diagnostic at `line` with `severity` and `message`.
    #[must_use]
    pub fn new(severity: impl Into<String>, line: usize, message: impl Into<String>) -> Self {
        Self {
            severity: severity.into(),
            line,
            message: message.into(),
        }
    }

    /// Render the diagnostic as one stable, model-readable line (`severity at line N: message`).
    #[must_use]
    pub fn render(&self) -> String {
        format!("{} at line {}: {}", self.severity, self.line, self.message)
    }
}

/// The post-write/post-edit downstream seam: a formatter pass over the written file and the LSP
/// diagnostics re-injected into the tool result.
///
/// The mutating tools (`write`, later `edit`) call these after a successful mutation. The
/// implementation is owned outside `tools`: the `format` and `lsp` capability crates land in M4 and
/// are bridged in then. Until then no hooks are attached, so [`format`](MutationHooks::format) is a
/// no-op (the file is written verbatim) and [`diagnostics`](MutationHooks::diagnostics) returns
/// none (a write with no LSP available succeeds unchanged with no error). The default method bodies
/// make a partial implementation (formatter-only, or LSP-only) trivial.
pub trait MutationHooks: Send + Sync {
    /// Reformat the just-written `path` in place. The default leaves the file untouched.
    fn format(&self, _path: &Path) {}

    /// Diagnostics for the just-written `path` to re-inject into the tool result. The default
    /// returns none (no LSP available is not an error).
    fn diagnostics(&self, _path: &Path) -> Vec<Diagnostic> {
        Vec::new()
    }
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
    session_id: Option<String>,
    todos: Option<Arc<dyn TodoStore>>,
}

impl ToolContext {
    /// Build a context rooted at `cwd` with an empty read-tracking set, no permission gating, no
    /// post-mutation hooks, no session, and no todo store.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            reads: Arc::new(Mutex::new(HashSet::new())),
            permissions: None,
            hooks: None,
            session_id: None,
            todos: None,
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

    /// Attach the post-mutation downstream hooks (formatter + LSP diagnostics) used after a write or
    /// edit (builder style). With none attached the hooks are no-ops.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Arc<dyn MutationHooks>) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// Run the post-write formatter over `path` (a resolved absolute path) through the attached
    /// hooks. A no-op when no hooks are attached, so the file is left exactly as written.
    pub fn format_after_write(&self, path: &Path) {
        if let Some(hooks) = &self.hooks {
            hooks.format(path);
        }
    }

    /// Collect LSP diagnostics for `path` (a resolved absolute path) through the attached hooks.
    /// Returns none when no hooks are attached (no LSP available is not an error).
    #[must_use]
    pub fn diagnostics_after_write(&self, path: &Path) -> Vec<Diagnostic> {
        self.hooks
            .as_ref()
            .map(|hooks| hooks.diagnostics(path))
            .unwrap_or_default()
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
