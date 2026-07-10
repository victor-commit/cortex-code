//! Surface-agnostic bridges the subagent child turn wires into its tool context: a minimal tool
//! dispatcher, a durable todo store, the pre-mutation checkpoint recorder, the post-mutation
//! formatter + LSP diagnostics hooks, the read-only `lsp` navigation service, and the built-in
//! per-agent permission rules.
//!
//! Each of these mirrors the identical adapter the `cli` and `tui` crates keep for their own
//! primary turns; they live here so the promoted child-turn runner can build a child dispatcher
//! without depending on either surface crate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use checkpoints::{CheckpointError, RecordedTurn, ShadowStore, TurnRecorder};
use engine::{Agent, ModelFamily, ToolDispatchResult, ToolDispatcher};
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use lsp::{
    FeedbackManager, FeedbackOutcome, FileDiagnostic, LspConfigSet, NavigationOutcome,
    NavigationRequest,
};
use memory::DynamicMemoryTracker;
use permissions::{Action, Rule};
use protocol::TodoItem;
use tools::{
    BackgroundJobs, DiagnosticFeedback, LspService, MutationHooks, TodoStore, ToolContext,
    ToolRegistry,
};

/// A minimal [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`], used for
/// subagent child turns. It advertises and routes the built-in tool set (model-family aware) but
/// deliberately omits MCP, skills, websearch, and nested `task` delegation, so a child subagent
/// cannot recursively spawn further subagents.
pub(crate) struct ChildDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
    dynamic_memory: Option<Arc<Mutex<DynamicMemoryTracker>>>,
}

impl ChildDispatcher {
    pub(crate) fn for_model(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        todos: Arc<dyn TodoStore>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        Self {
            registry,
            context: ToolContext::new(cwd)
                .with_session(session_id)
                .with_todo_store(todos),
            dynamic_memory: None,
        }
    }

    #[must_use]
    pub(crate) fn with_dynamic_memory(mut self, tracker: DynamicMemoryTracker) -> Self {
        self.dynamic_memory = Some(Arc::new(Mutex::new(tracker)));
        self
    }

    #[must_use]
    pub(crate) fn with_audit(mut self, audit: Arc<audit::AuditSink>) -> Self {
        self.context = self.context.with_audit(audit);
        self
    }

    #[must_use]
    pub(crate) fn with_permissions(mut self, engine: Arc<permissions::PermissionEngine>) -> Self {
        self.context = self.context.with_permissions(engine);
        self
    }

    #[must_use]
    pub(crate) fn with_checkpoints(mut self, recorder: Arc<dyn tools::CheckpointRecorder>) -> Self {
        self.context = self.context.with_checkpoints(recorder);
        self
    }

    #[must_use]
    pub(crate) fn with_hooks(mut self, hooks: Arc<dyn MutationHooks>) -> Self {
        self.context = self.context.with_hooks(hooks);
        self
    }

    #[must_use]
    pub(crate) fn with_lsp_service(mut self, service: Arc<dyn LspService>) -> Self {
        self.context = self.context.with_lsp_service(service);
        self
    }

    pub(crate) fn background_jobs(&self) -> Arc<BackgroundJobs> {
        self.context.background_jobs().clone()
    }
}

impl ToolDispatcher for ChildDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        self.registry.definitions()
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;
            ToolDispatchResult {
                content: output.model_content(),
                trusted_skill_guidance: output.trusted_skill_guidance().cloned(),
                parts: output.parts,
                mcp_result: None,
                is_error: output.is_error,
            }
        })
    }

    fn dynamic_system_instructions(&self) -> Result<Option<String>, errors::Diagnostic> {
        let Some(tracker) = self.dynamic_memory.as_ref() else {
            return Ok(None);
        };
        let Ok(mut tracker) = tracker.lock() else {
            return Ok(None);
        };
        let update = tracker.update_for_reads(&self.context.memory_scopes());
        for warning in update.warnings {
            eprintln!("cortex run: {warning}");
        }
        if update.net_blocked {
            return Err(errors::Diagnostic::net_disabled(
                "dynamic instruction URL fetch triggered by read-path memory discovery",
            ));
        }
        Ok(update.rendered)
    }
}

/// A [`TodoStore`] backed by the durable SQLite database at a resolved path, reusing one connection
/// for every todo read/write in the child session.
pub(crate) struct DbTodoStore {
    db: Mutex<Result<db::Database, String>>,
}

impl DbTodoStore {
    #[must_use]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            db: Mutex::new(db::Database::open(&path).map_err(|err| err.to_string())),
        }
    }

    fn with_db<T>(&self, f: impl FnOnce(&db::Database) -> Result<T, String>) -> Result<T, String> {
        let guard = self
            .db
            .lock()
            .map_err(|_| "todo store connection lock poisoned".to_owned())?;
        match guard.as_ref() {
            Ok(db) => f(db),
            Err(err) => Err(err.clone()),
        }
    }
}

impl TodoStore for DbTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        self.with_db(|db| {
            db.todos()
                .list_for_session(session_id)
                .map_err(|err| err.to_string())
        })
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.with_db(|db| {
            db.todos()
                .replace_for_session(session_id, items)
                .map_err(|err| err.to_string())
        })
    }
}

/// Bridges the `tools` pre-mutation recorder seam to a [`checkpoints::TurnRecorder`], so a child
/// turn's mutating tools snapshot a file's prior state into the shared shadow store.
pub(crate) struct RecorderBridge {
    inner: Arc<TurnRecorder>,
}

impl RecorderBridge {
    #[must_use]
    pub(crate) fn new(inner: Arc<TurnRecorder>) -> Self {
        Self { inner }
    }
}

impl tools::CheckpointRecorder for RecorderBridge {
    fn record_pre_image(&self, path: &Path) {
        self.inner.record_pre_image(path);
    }
}

/// Open (creating on first use) the shared shadow checkpoint store for `cwd`, or `None` when no
/// data location can be resolved, so a child turn degrades to no checkpointing rather than failing.
pub(crate) fn open_store(cwd: &Path) -> Result<Option<ShadowStore>, CheckpointError> {
    ShadowStore::open_default(cwd)
}

/// Finalize a turn recorder at the end of a child turn, logging (but not failing on) any snapshot
/// failure.
pub(crate) fn finalize_turn(recorder: &TurnRecorder) -> Option<RecordedTurn> {
    match recorder.finalize() {
        Ok(turn) => turn,
        Err(err) => {
            eprintln!("cortex run: subagent checkpoint snapshot unavailable: {err}");
            None
        }
    }
}

/// Post-mutation hooks: run the real formatter first, then delegate diagnostics to the LSP hook.
pub(crate) struct PostMutationHooks {
    formatter: format::FormatterRunner,
    diagnostics: Arc<dyn MutationHooks>,
}

impl PostMutationHooks {
    #[must_use]
    pub(crate) fn with_config(
        root: impl Into<PathBuf>,
        config: format::FormatterConfig,
        diagnostics: Arc<dyn MutationHooks>,
    ) -> Self {
        Self {
            formatter: format::FormatterRunner::with_config(root, config),
            diagnostics,
        }
    }
}

impl MutationHooks for PostMutationHooks {
    fn format(&self, path: &Path) {
        let _ = self.formatter.format_path(path);
    }

    fn format_feedback(&self, path: &Path) -> Option<String> {
        match self.formatter.format_path(path) {
            format::FormatOutcome::Failed(failure) => Some(format::format_failure_note(&failure)),
            format::FormatOutcome::Formatted(_) | format::FormatOutcome::Skipped(_) => None,
        }
    }

    fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
        self.diagnostics.diagnostic_feedback(path)
    }
}

/// Post-mutation LSP diagnostics hook and read-only `lsp` navigation service for a child turn.
pub(crate) struct LspHooks {
    manager: Mutex<FeedbackManager>,
}

impl LspHooks {
    #[must_use]
    pub(crate) fn new(root: impl Into<PathBuf>, config: LspConfigSet) -> Self {
        Self {
            manager: Mutex::new(FeedbackManager::new(root, config)),
        }
    }

    pub(crate) fn shutdown(&self) {
        if let Ok(mut manager) = self.manager.lock() {
            manager.shutdown_all();
        }
    }
}

impl MutationHooks for LspHooks {
    fn diagnostic_feedback(&self, path: &Path) -> DiagnosticFeedback {
        let Ok(mut manager) = self.manager.lock() else {
            return DiagnosticFeedback::default();
        };
        match manager.collect(path) {
            FeedbackOutcome::Diagnostics(diagnostics) => DiagnosticFeedback::diagnostics(
                diagnostics.into_iter().map(to_tool_diagnostic).collect(),
            ),
            FeedbackOutcome::NoServer => DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some(no_server_note(path)),
            },
            FeedbackOutcome::Unavailable => DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some(format!(
                    "LSP diagnostics unavailable for {}: language server stopped responding",
                    path.display()
                )),
            },
        }
    }
}

impl LspService for LspHooks {
    fn query(&self, request: tools::LspRequest) -> tools::ToolOutput {
        let Ok(mut manager) = self.manager.lock() else {
            return tools::ToolOutput::text(
                "lsp: language server unavailable because the LSP manager lock is poisoned",
            );
        };
        let label = request_label(&request);
        let navigation = match to_navigation_request(request) {
            Ok(request) => request,
            Err(message) => return tools::ToolOutput::error(message),
        };
        render_navigation_result(&label, manager.navigate(navigation))
    }
}

fn to_navigation_request(request: tools::LspRequest) -> Result<NavigationRequest, String> {
    match request.operation {
        tools::LspOperation::Hover => Ok(NavigationRequest::Hover {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::Definition => Ok(NavigationRequest::Definition {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::References => Ok(NavigationRequest::References {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
            include_declaration: request.include_declaration,
        }),
        tools::LspOperation::Implementation => Ok(NavigationRequest::Implementation {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::PrepareCallHierarchy => Ok(NavigationRequest::PrepareCallHierarchy {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::IncomingCalls => Ok(NavigationRequest::IncomingCalls {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::OutgoingCalls => Ok(NavigationRequest::OutgoingCalls {
            file_path: path_arg(request.file_path)?,
            line: required_position(request.line, "line")?,
            character: required_position(request.character, "character")?,
        }),
        tools::LspOperation::DocumentSymbols => Ok(NavigationRequest::DocumentSymbols {
            file_path: path_arg(request.file_path)?,
        }),
        tools::LspOperation::WorkspaceSymbols => Ok(NavigationRequest::WorkspaceSymbols {
            query: request
                .query
                .filter(|query| !query.trim().is_empty())
                .ok_or_else(|| "lsp: workspaceSymbols requires a non-empty query".to_owned())?,
            file_path: request.file_path.map(Into::into),
        }),
    }
}

fn path_arg(path: Option<String>) -> Result<PathBuf, String> {
    path.filter(|path| !path.trim().is_empty())
        .map(Into::into)
        .ok_or_else(|| "lsp: filePath is required for this action".to_owned())
}

fn required_position(value: Option<u32>, name: &str) -> Result<u32, String> {
    value
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("lsp: {name} must be a positive 1-based integer"))
}

fn request_label(request: &tools::LspRequest) -> String {
    let action = match request.operation {
        tools::LspOperation::Hover => "hover",
        tools::LspOperation::Definition => "definition",
        tools::LspOperation::References => "references",
        tools::LspOperation::Implementation => "implementation",
        tools::LspOperation::PrepareCallHierarchy => "prepareCallHierarchy",
        tools::LspOperation::IncomingCalls => "incomingCalls",
        tools::LspOperation::OutgoingCalls => "outgoingCalls",
        tools::LspOperation::DocumentSymbols => "documentSymbols",
        tools::LspOperation::WorkspaceSymbols => "workspaceSymbols",
    };
    let target = request
        .file_path
        .as_deref()
        .or(request.query.as_deref())
        .unwrap_or("workspace");
    format!("{action} for {target}")
}

fn render_navigation_result(label: &str, outcome: NavigationOutcome) -> tools::ToolOutput {
    match outcome {
        NavigationOutcome::NoServer => tools::ToolOutput::text(format!(
            "lsp: no server available for {label}; no diagnostics or navigation results were claimed"
        )),
        NavigationOutcome::Unavailable => tools::ToolOutput::text(format!(
            "lsp: language server unavailable for {label}; the server stopped responding or crashed"
        )),
        NavigationOutcome::Hover(hover) => tools::ToolOutput::text(format!(
            "lsp hover {label}:\n{}",
            lsp::hover_context_block(hover.as_ref())
        )),
        NavigationOutcome::Locations(locations) => tools::ToolOutput::text(format!(
            "lsp locations {label}:\n{}",
            lsp::locations_context_block(&locations)
        )),
        NavigationOutcome::CallHierarchyItems(items) => tools::ToolOutput::text(format!(
            "lsp call hierarchy {label}:\n{}",
            lsp::call_hierarchy_items_context_block(&items)
        )),
        NavigationOutcome::IncomingCalls(calls) => tools::ToolOutput::text(format!(
            "lsp incoming calls {label}:\n{}",
            lsp::incoming_calls_context_block(&calls)
        )),
        NavigationOutcome::OutgoingCalls(calls) => tools::ToolOutput::text(format!(
            "lsp outgoing calls {label}:\n{}",
            lsp::outgoing_calls_context_block(&calls)
        )),
        NavigationOutcome::DocumentSymbols(symbols) => tools::ToolOutput::text(format!(
            "lsp document symbols {label}:\n{}",
            lsp::document_symbols_context_block(&symbols)
        )),
        NavigationOutcome::WorkspaceSymbols(symbols) => tools::ToolOutput::text(format!(
            "lsp workspace symbols {label}:\n{}",
            lsp::workspace_symbols_context_block(&symbols)
        )),
    }
}

fn no_server_note(path: &Path) -> String {
    if redaction::network_disabled() {
        format!(
            "LSP diagnostics unavailable for {}: CORTEX_NO_NET=1 prevents auto-installing a missing language server; write/edit still persisted",
            path.display()
        )
    } else {
        format!(
            "LSP diagnostics unavailable for {}: no language server available or auto-install could not provide one",
            path.display()
        )
    }
}

fn to_tool_diagnostic(diagnostic: FileDiagnostic) -> tools::Diagnostic {
    tools::Diagnostic::with_range(
        diagnostic.severity,
        usize::try_from(diagnostic.line).unwrap_or(usize::MAX),
        usize::try_from(diagnostic.character).unwrap_or(usize::MAX),
        usize::try_from(diagnostic.end_line).unwrap_or(usize::MAX),
        usize::try_from(diagnostic.end_character).unwrap_or(usize::MAX),
        diagnostic.message,
    )
    .with_metadata(diagnostic.source, diagnostic.code)
}

/// Low and high precedence built-in permission rules contributed by an agent mode.
///
/// `base_rules` are prepended before project/user configuration; `restriction_rules` are appended
/// after config/save rules AND installed as hard denies, so a read-only mode's mutation blocks
/// cannot be widened by a project allow or a saved allow-always decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BuiltinAgentPermissionRules {
    pub base_rules: Vec<Rule>,
    pub restriction_rules: Vec<Rule>,
}

/// The built-in permission layer for a selected agent persona (mirrors the headless CLI's rules).
#[must_use]
pub(crate) fn builtin_agent_permission_rules(agent: Agent) -> BuiltinAgentPermissionRules {
    match agent {
        Agent::Plan | Agent::Explore => BuiltinAgentPermissionRules {
            base_rules: {
                let mut rules = read_only_allow_rules();
                rules.extend(permissions::default_external_and_env_rules());
                rules
            },
            restriction_rules: mutation_deny_rules(),
        },
        Agent::Build | Agent::General => BuiltinAgentPermissionRules {
            base_rules: permissions::permissive_default_rules(),
            restriction_rules: Vec::new(),
        },
    }
}

fn read_only_allow_rules() -> Vec<Rule> {
    ["read", "glob", "grep", "lsp", "webfetch", "websearch"]
        .into_iter()
        .map(|tool| Rule::new(tool, "*", Action::Allow))
        .collect()
}

fn mutation_deny_rules() -> Vec<Rule> {
    ["write", "edit", "apply_patch", "format", "bash"]
        .into_iter()
        .map(|tool| Rule::new(tool, "*", Action::Deny))
        .collect()
}
