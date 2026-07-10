//! Tool dispatch for a TUI turn, with file-change diff capture.
//!
//! The durable runner advertises and executes tools through the [`engine::ToolDispatcher`] seam.
//! This adapter holds a [`ToolRegistry`] and the per-turn [`ToolContext`] (mirroring the headless
//! `cortex run` bridge) so a real turn driven from the TUI can read, edit, and create files. Around
//! each mutating tool call it captures the target file's before/after content and emits a
//! [`crate::diff::FileDiff`] over the turn channel, so an edit/creation renders as a colored unified
//! diff in the transcript (VAL-TUI-021..023).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use engine::{ModelFamily, ToolDispatchResult, ToolDispatcher};
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use mcp::{CapabilityRoute, McpManager};
use protocol::TodoItem;
use tools::{
    BackgroundJobs, LspService, MutationHooks, TodoStore, ToolContext, ToolRegistry,
    WebSearchConfig,
};

use crate::diff::compute_file_diff;
use crate::turn::TurnMessage;

/// A [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`], emitting a
/// [`crate::diff::FileDiff`] for each file a mutating tool changes.
pub struct TurnDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
    events: Sender<TurnMessage>,
    mcp: Option<Arc<McpManager>>,
}

impl TurnDispatcher {
    /// Build a dispatcher whose tool set is materialized for `model_id`'s family (a GPT-family model
    /// gets `apply_patch`; every other model keeps `edit`/`write`), scoped to `cwd` and `session_id`
    /// with an in-memory todo store. File changes are reported as diffs over `events`.
    #[must_use]
    pub fn new(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        events: Sender<TurnMessage>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        let todos: Arc<dyn TodoStore> = Arc::new(EphemeralTodos::default());
        let context = ToolContext::new(cwd)
            .with_session(session_id)
            .with_todo_store(todos)
            .with_interactive_questions(true);
        Self {
            registry,
            context,
            events,
            mcp: None,
        }
    }

    /// Attach a connected [`McpManager`] so its server-qualified MCP tools are advertised and
    /// callable from this TUI turn (builder style). Without it the dispatcher exposes only built-in
    /// tools.
    #[must_use]
    pub fn with_mcp(mut self, manager: Arc<McpManager>) -> Self {
        self.mcp = Some(manager);
        self
    }

    /// Gate every tool effect through `engine` (builder style). The TUI attaches an engine whose
    /// interactive responder surfaces an `ask` to the UI thread, so an ask-gated mutation prompts
    /// for approval and a denied action produces no side effect.
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<permissions::PermissionEngine>) -> Self {
        self.context = self.context.with_permissions(engine);
        self
    }

    /// Attach the durable audit sink used to record real TUI tool effects.
    #[must_use]
    pub fn with_audit(mut self, audit: Arc<audit::AuditSink>) -> Self {
        self.context = self.context.with_audit(audit);
        self
    }

    /// Attach the pre-mutation checkpoint recorder so each mutating tool snapshots a file's prior
    /// state before it changes it (builder style). Without this the TUI turn records no checkpoints,
    /// so `/undo` would have nothing to restore.
    #[must_use]
    pub fn with_checkpoints(mut self, recorder: Arc<dyn tools::CheckpointRecorder>) -> Self {
        self.context = self.context.with_checkpoints(recorder);
        self
    }

    /// Attach post-mutation hooks (formatter + LSP diagnostics) to the tool context.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Arc<dyn MutationHooks>) -> Self {
        self.context = self.context.with_hooks(hooks);
        self
    }

    /// Attach read-only LSP navigation/status query support to the `lsp` tool.
    #[must_use]
    pub fn with_lsp_service(mut self, service: Arc<dyn LspService>) -> Self {
        self.context = self.context.with_lsp_service(service);
        self
    }

    /// Attach the per-session todo store the `todo` tool reads and writes through.
    #[must_use]
    pub fn with_todo_store(mut self, store: Arc<dyn TodoStore>) -> Self {
        self.context = self.context.with_todo_store(store);
        self
    }

    /// Attach discovered skills to the per-turn registry so the model can engage one through the
    /// `skill` tool. Skill guidance remains inactive until that tool succeeds, matching headless.
    #[must_use]
    pub fn with_skills(mut self, skills: Vec<skills::Skill>) -> Self {
        self.registry.register_skills(skills);
        self
    }

    /// Attach the real `websearch` tool after the turn config has explicitly enabled a provider.
    ///
    /// Mirrors the headless bridge: with no call to this method `websearch` is omitted from the
    /// advertised tool set, so the default-off policy holds for a live TUI turn too. Once
    /// `[websearch] enabled = true` resolves a config, the model can search the web this turn.
    #[must_use]
    pub fn with_websearch(mut self, config: WebSearchConfig) -> Self {
        self.registry.register_websearch(config);
        self
    }

    /// Attach the `task` tool and the surface-owned durable subagent runner.
    ///
    /// Mirrors the headless bridge (`RegistryDispatcher::with_task_runner`): `targets` must already
    /// be permission-filtered to exactly the subagent types this parent may spawn; when the set is
    /// empty the task tool is omitted entirely (the model is not offered a delegation that could
    /// never spawn anything). With it attached, a live TUI turn can delegate to a permitted subagent
    /// via the shared [`subagent_runner`] runner.
    #[must_use]
    pub fn with_task_runner(
        mut self,
        targets: Vec<tools::TaskTarget>,
        runner: Arc<dyn tools::TaskRunner>,
    ) -> Self {
        if !targets.is_empty() {
            self.registry.register_task(targets);
            self.context = self.context.with_task_runner(runner);
        }
        self
    }

    /// The MCP definitions advertised this turn, materialized by the manager's one global
    /// allocator alongside their typed dispatch routes.
    fn mcp_definitions(&self) -> Vec<ToolDef> {
        self.mcp
            .as_ref()
            .map(|manager| {
                manager
                    .capabilities()
                    .into_iter()
                    .map(|capability| ToolDef {
                        name: capability.id,
                        description: capability.description,
                        parameters: capability.parameters,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The background-job registry tracking long-running `bash` jobs spawned during the turn, so the
    /// caller can reap them (terminate + await by PID) when the turn ends — no orphan survives.
    #[must_use]
    pub fn background_jobs(&self) -> Arc<BackgroundJobs> {
        self.context.background_jobs().clone()
    }

    /// Capture a diff for each file `call` changed, if any, and publish it over the turn channel.
    fn emit_diffs(&self, call: &ToolCall, before: &[(String, PathBuf, Option<String>)]) {
        for (rel, resolved, before_content) in before {
            let after = std::fs::read_to_string(resolved).ok();
            if before_content.as_deref() == after.as_deref() {
                continue;
            }
            if let Some(diff) = compute_file_diff(rel, before_content.as_deref(), after.as_deref())
            {
                let _ = self.events.send(TurnMessage::FileDiff(Box::new(diff)));
            }
        }
        let _ = call;
    }
}

impl ToolDispatcher for TurnDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        let mut defs = self.registry.definitions();
        defs.extend(self.mcp_definitions());
        defs
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            if let Some(manager) = &self.mcp {
                if let Some(route) = manager
                    .route_for_id(&call.name)
                    .filter(CapabilityRoute::is_mcp)
                {
                    if let Err(message) = self.context.ensure_permitted(&call.name, &call.name) {
                        return ToolDispatchResult {
                            content: message.clone(),
                            parts: vec![protocol::ToolResultPart::text(message)],
                            is_error: true,
                        };
                    }
                    return dispatch_mcp(manager, &route, call, &self.context).await;
                }
            }

            // Snapshot each target file's prior content BEFORE the tool mutates it, so the diff
            // reflects exactly this call's change (a missing file reads as `None` -> a new-file diff).
            let before: Vec<(String, PathBuf, Option<String>)> = mutated_targets(call)
                .into_iter()
                .map(|rel| {
                    let resolved = self.context.resolve(&rel);
                    let content = std::fs::read_to_string(&resolved).ok();
                    (rel, resolved, content)
                })
                .collect();

            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;

            if !output.is_error {
                self.emit_diffs(call, &before);
            }

            ToolDispatchResult {
                content: output.model_content(),
                parts: output.parts,
                is_error: output.is_error,
            }
        })
    }
}

/// Execute one MCP tool call through the manager and convert the relayed result into the runner's
/// [`ToolDispatchResult`]. A non-object/unparseable arguments string or a transport error resolves
/// to an error result so the turn continues and the model can recover.
async fn dispatch_mcp(
    manager: &McpManager,
    route: &CapabilityRoute,
    call: &ToolCall,
    ctx: &ToolContext,
) -> ToolDispatchResult {
    let audit_tool = manager.tool_info(&call.name);
    let arguments = if call.arguments.trim().is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_str(&call.arguments) {
            Ok(value) => value,
            Err(err) => {
                return ToolDispatchResult {
                    content: format!("{}: invalid arguments JSON: {err}", call.name),
                    parts: vec![protocol::ToolResultPart::text(format!(
                        "{}: invalid arguments JSON: {err}",
                        call.name
                    ))],
                    is_error: true,
                };
            }
        }
    };

    let result = manager.invoke(route, arguments).await;

    if let Some(tool) = audit_tool
        && let Some(audit) = ctx.audit_sink()
    {
        let outcome = match &result {
            Ok(relayed) if !relayed.is_error => "success",
            Ok(_) | Err(_) => "error",
        };
        audit.record_mcp_tool_call(&tool.server, &tool.tool, &tool.id, &call.arguments, outcome);
    }

    match result {
        Ok(relayed) => ToolDispatchResult {
            content: relayed.content,
            parts: relayed.parts,
            is_error: relayed.is_error,
        },
        Err(err) => {
            let message = err.to_string();
            ToolDispatchResult {
                content: message.clone(),
                parts: vec![protocol::ToolResultPart::text(message)],
                is_error: true,
            }
        }
    }
}

/// The workspace-relative path(s) a tool call mutates, for before/after diff capture. A non-mutating
/// tool (read/glob/grep/bash/todo) returns an empty list, so no diff is produced for it.
fn mutated_targets(call: &ToolCall) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments).unwrap_or_default();
    match call.name.as_str() {
        "write" | "edit" => value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.to_owned()])
            .unwrap_or_default(),
        "apply_patch" => value
            .get("patch")
            .and_then(serde_json::Value::as_str)
            .map(parse_patch_paths)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Extract the file paths an `apply_patch` envelope adds or updates (the sections that change file
/// content), so each changed file gets its own diff.
fn parse_patch_paths(patch: &str) -> Vec<String> {
    const ADD: &str = "*** Add File: ";
    const UPDATE: &str = "*** Update File: ";
    let mut paths = Vec::new();
    for line in patch.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix(ADD) {
            paths.push(rest.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix(UPDATE) {
            paths.push(rest.trim().to_owned());
        }
    }
    paths
}

/// An in-memory, per-session todo store: the TUI turn does not persist todos to the durable DB, but
/// the `todo` tool still requires a store, so this keeps a session's list alive for the turn.
#[derive(Default)]
struct EphemeralTodos {
    lists: Mutex<HashMap<String, Vec<TodoItem>>>,
}

impl TodoStore for EphemeralTodos {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        Ok(self
            .lists
            .lock()
            .map_err(|_| "todo store poisoned".to_owned())?
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.lists
            .lock()
            .map_err(|_| "todo store poisoned".to_owned())?
            .insert(session_id.to_owned(), items.to_vec());
        Ok(())
    }
}

/// A durable todo store backed by the shared SQLite database the TUI uses for sessions.
pub struct DurableTodos {
    db_path: PathBuf,
}

impl DurableTodos {
    /// Create a todo store over a resolved SQLite database path.
    #[must_use]
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    fn open(&self) -> Result<db::Database, String> {
        db::Database::open(&self.db_path).map_err(|err| err.to_string())
    }
}

impl TodoStore for DurableTodos {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        self.open()?
            .todos()
            .list_for_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.open()?
            .todos()
            .replace_for_session(session_id, items)
            .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-tui-diff-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn block_on(
        future: impl std::future::Future<Output = ToolDispatchResult>,
    ) -> ToolDispatchResult {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    fn drain_diffs(rx: &std::sync::mpsc::Receiver<TurnMessage>) -> Vec<crate::diff::FileDiff> {
        let mut diffs = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let TurnMessage::FileDiff(diff) = message {
                diffs.push(*diff);
            }
        }
        diffs
    }

    fn echo_server_bin() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
        bin.exists().then_some(bin)
    }

    #[test]
    fn mutated_targets_reads_write_and_edit_paths() {
        assert_eq!(
            mutated_targets(&call("write", r#"{"path":"a.txt","content":"x"}"#)),
            vec!["a.txt".to_owned()]
        );
        assert_eq!(
            mutated_targets(&call(
                "edit",
                r#"{"path":"b.rs","oldString":"x","newString":"y"}"#
            )),
            vec!["b.rs".to_owned()]
        );
        assert!(mutated_targets(&call("read", r#"{"path":"c.txt"}"#)).is_empty());
        assert!(mutated_targets(&call("bash", r#"{"command":"ls"}"#)).is_empty());
    }

    #[test]
    fn mutated_targets_parses_apply_patch_sections() {
        let patch = "*** Begin Patch\n*** Add File: new.py\n+line\n*** Update File: old.py\n@@\n-a\n+b\n*** End Patch";
        let args = serde_json::json!({ "patch": patch }).to_string();
        assert_eq!(
            mutated_targets(&call("apply_patch", &args)),
            vec!["new.py".to_owned(), "old.py".to_owned()]
        );
    }

    #[test]
    fn writing_a_new_file_emits_an_all_added_diff() {
        let dir = scratch_dir("write-new");
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        let result = block_on(dispatcher.dispatch(&call(
            "write",
            r#"{"path":"created.txt","content":"hello\nworld\n"}"#,
        )));
        assert!(!result.is_error, "write should succeed: {}", result.content);

        let diffs = drain_diffs(&rx);
        assert_eq!(diffs.len(), 1, "exactly one file changed");
        let diff = &diffs[0];
        assert_eq!(diff.path, "created.txt");
        assert!(diff.is_new, "a created file is a new-file diff");
        assert_eq!(diff.removals, 0);
        assert!(diff.additions >= 2);
    }

    #[test]
    fn editing_an_existing_file_emits_a_replacement_diff() {
        let dir = scratch_dir("edit");
        std::fs::write(
            dir.join("greet.rs"),
            "fn main() {\n    println!(\"Hello\");\n    let x = 1;\n}\n",
        )
        .unwrap();
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        // The edit tool requires a prior read in the session; the shared context records it.
        let read = block_on(dispatcher.dispatch(&call("read", r#"{"path":"greet.rs"}"#)));
        assert!(!read.is_error, "read should succeed: {}", read.content);

        let result = block_on(dispatcher.dispatch(&call(
            "edit",
            r#"{"path":"greet.rs","oldString":"println!(\"Hello\");","newString":"println!(\"Cortex\");"}"#,
        )));
        assert!(!result.is_error, "edit should succeed: {}", result.content);

        let diffs = drain_diffs(&rx);
        assert_eq!(diffs.len(), 1, "only the edit emits a diff, not the read");
        let diff = &diffs[0];
        assert_eq!(diff.path, "greet.rs");
        assert!(!diff.is_new);
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.removals, 1);
    }

    #[test]
    fn a_failed_tool_call_emits_no_diff() {
        let dir = scratch_dir("fail");
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        // Editing a file that was never read (and does not exist) fails; no diff is emitted.
        let result = block_on(dispatcher.dispatch(&call(
            "edit",
            r#"{"path":"missing.txt","oldString":"a","newString":"b"}"#,
        )));
        assert!(result.is_error);
        assert!(drain_diffs(&rx).is_empty());
    }

    struct FormatAndDiagnose;

    impl MutationHooks for FormatAndDiagnose {
        fn format(&self, path: &std::path::Path) {
            let _ = std::fs::write(path, "fn main() {\n    let value: i32 = \"bad\";\n}\n");
        }

        fn diagnostics(&self, _path: &std::path::Path) -> Vec<tools::Diagnostic> {
            vec![tools::Diagnostic::new("error", 2, "mismatched types")]
        }
    }

    #[test]
    fn tui_dispatcher_runs_mutation_hooks_and_surfaces_diagnostics() {
        let dir = scratch_dir("hooks");
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx)
                .with_hooks(Arc::new(FormatAndDiagnose));

        let result = block_on(dispatcher.dispatch(&call(
            "write",
            r#"{"path":"bad.rs","content":"fn  main( ){let value:i32=\"bad\";}"}"#,
        )));

        assert!(!result.is_error, "write should succeed: {}", result.content);
        let written = std::fs::read_to_string(dir.join("bad.rs")).expect("formatted file");
        assert_eq!(
            written, "fn main() {\n    let value: i32 = \"bad\";\n}\n",
            "formatter hook governs the persisted bytes"
        );
        assert!(
            result.content.contains("Diagnostics:") && result.content.contains("mismatched types"),
            "diagnostics are re-injected into the tool result: {}",
            result.content
        );
        let diffs = drain_diffs(&rx);
        assert_eq!(diffs.len(), 1, "the formatted write still emits one diff");
    }

    #[test]
    fn with_skills_registers_the_skill_tool_without_leaking_guidance() {
        let dir = scratch_dir("skills");
        let (tx, _rx) = channel();
        let dispatcher = TurnDispatcher::new(dir, "google/gemini-2.5-flash", "ses".to_owned(), tx)
            .with_skills(vec![skills::Skill {
                name: "cross-skill".to_owned(),
                description: "Use for cross-surface skill checks".to_owned(),
                guidance: "Prefix every answer with TUI-SKILL-SECRET.".to_owned(),
                source: PathBuf::from("SKILL.md"),
            }]);

        let skill = dispatcher
            .definitions()
            .into_iter()
            .find(|definition| definition.name == "skill")
            .expect("skill tool is advertised");

        let name_description = skill.parameters["properties"]["name"]["description"]
            .as_str()
            .expect("name description");
        assert!(name_description.contains("cross-skill"));
        assert!(skill.parameters["properties"]["name"]["enum"].is_null());
        assert_eq!(
            skill.parameters["anyOf"][0]["required"][0],
            serde_json::Value::String("name".to_owned())
        );
        assert_eq!(
            skill.parameters["anyOf"][1]["required"][0],
            serde_json::Value::String("skill".to_owned())
        );
        assert!(
            skill
                .description
                .contains("Use for cross-surface skill checks"),
            "description is visible: {}",
            skill.description
        );
        assert!(
            !skill.description.contains("TUI-SKILL-SECRET"),
            "guidance remains isolated until the skill tool is called"
        );
    }

    #[test]
    fn with_websearch_advertises_the_tool_and_default_omits_it() {
        let dir = scratch_dir("websearch");
        let (tx, _rx) = channel();
        let plain =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);
        assert!(
            !plain
                .definitions()
                .iter()
                .any(|def| def.name == "websearch"),
            "websearch must be omitted from a default TUI turn (default-off policy)"
        );

        let (tx, _rx) = channel();
        let wired = TurnDispatcher::new(dir, "google/gemini-2.5-flash", "ses".to_owned(), tx)
            .with_websearch(tools::WebSearchConfig::duckduckgo());
        assert!(
            wired
                .definitions()
                .iter()
                .any(|def| def.name == "websearch"),
            "with_websearch must advertise the websearch tool so the model can search this turn"
        );
    }

    #[test]
    fn tui_dispatcher_uses_the_same_unique_mcp_routes_as_headless() {
        let Some(bin) = echo_server_bin() else {
            panic!("the cortex-mcp-echo-server fixture must be built for global routing coverage");
        };
        let dir = scratch_dir("mcp-global-routes");
        let log = dir.join("mcp.log");
        let servers = mcp::servers_from_value(&serde_json::json!({
            "apply": {
                "type": "local",
                "command": [bin.to_string_lossy()],
                "environment": {
                    "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                    "CORTEX_MCP_ECHO_LABEL": "apply",
                    "CORTEX_MCP_ECHO_EXTRA_TOOL": "patch"
                }
            },
            "demo": {
                "type": "local",
                "command": [bin.to_string_lossy()],
                "environment": {
                    "CORTEX_MCP_ECHO_LOG": log.to_string_lossy(),
                    "CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS": "1",
                    "CORTEX_MCP_ECHO_ENABLE_COLLISION_TOOLS": "1"
                }
            }
        }))
        .expect("valid fixture config");

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(async {
                let manager = Arc::new(
                    mcp::McpManager::connect_with(
                        servers,
                        mcp::McpConnectOptions {
                            bus: None,
                            credentials: None,
                            roots: Vec::new(),
                            reserved_tool_ids: ToolRegistry::reserved_builtin_ids(),
                        },
                    )
                    .await,
                );
                let (tx, _rx) = channel();
                let dispatcher = TurnDispatcher::new(
                    dir.clone(),
                    "google/gemini-2.5-flash",
                    "ses_mcp_tui".to_owned(),
                    tx,
                )
                .with_mcp(manager.clone());
                let names = dispatcher
                    .definitions()
                    .into_iter()
                    .map(|definition| definition.name)
                    .collect::<Vec<_>>();
                let mut unique = names.clone();
                unique.sort();
                unique.dedup();
                assert_eq!(
                    unique.len(),
                    names.len(),
                    "the TUI must advertise one collision-free model namespace: {names:?}"
                );

                let resource_tool = manager
                    .tools()
                    .into_iter()
                    .find(|tool| tool.tool == "resource_notes://demo")
                    .expect("collision tool");
                let resource = manager
                    .resources()
                    .into_iter()
                    .find(|resource| resource.uri == "notes://demo")
                    .expect("resource");
                let prompt_tool = manager
                    .tools()
                    .into_iter()
                    .find(|tool| tool.tool == "prompt_greet")
                    .expect("prompt collision tool");
                let prompt = manager
                    .prompts()
                    .into_iter()
                    .find(|prompt| prompt.name == "greet")
                    .expect("prompt");

                for (name, arguments, expected) in [
                    (
                        resource_tool.id,
                        "{}".to_owned(),
                        "collision tool resource_notes://demo",
                    ),
                    (resource.id, "{}".to_owned(), "RESOURCE-CONTENT-default"),
                    (
                        prompt_tool.id,
                        "{}".to_owned(),
                        "collision tool prompt_greet",
                    ),
                    (
                        prompt.id,
                        r#"{"name":"tui"}"#.to_owned(),
                        "Hello tui from MCP",
                    ),
                ] {
                    let result = dispatcher.dispatch(&call(&name, &arguments)).await;
                    assert!(
                        !result.is_error && result.content.contains(expected),
                        "TUI route `{name}` must reach its exact capability: {}",
                        result.content
                    );
                }

                manager.shutdown().await;
            });

        let log_contents = std::fs::read_to_string(&log).expect("fixture log");
        for expected in [
            "tools/call name=resource_notes://demo",
            "resources/read uri=notes://demo",
            "tools/call name=prompt_greet",
            "prompts/get name=greet",
        ] {
            assert!(
                log_contents.contains(expected),
                "fixture log is missing `{expected}`:\n{log_contents}"
            );
        }
    }

    struct StubTaskRunner;

    impl tools::TaskRunner for StubTaskRunner {
        fn run_task<'a>(
            &'a self,
            _request: tools::TaskRequest,
        ) -> BoxFuture<'a, tools::ToolOutput> {
            Box::pin(async { tools::ToolOutput::text("stub subagent result") })
        }
    }

    #[test]
    fn with_task_runner_advertises_the_tool_only_when_targets_are_non_empty() {
        let dir = scratch_dir("task-runner");
        let (tx, _rx) = channel();
        let plain =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);
        assert!(
            !plain.definitions().iter().any(|def| def.name == "task"),
            "task must be omitted from a default TUI turn (no runner attached)"
        );

        let (tx, _rx) = channel();
        let empty_targets =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx)
                .with_task_runner(Vec::new(), Arc::new(StubTaskRunner));
        assert!(
            !empty_targets
                .definitions()
                .iter()
                .any(|def| def.name == "task"),
            "task must stay omitted when the permission-filtered target list is empty"
        );

        let (tx, _rx) = channel();
        let wired = TurnDispatcher::new(dir, "google/gemini-2.5-flash", "ses".to_owned(), tx)
            .with_task_runner(
                vec![tools::TaskTarget::new("explore", "read-only investigation")],
                Arc::new(StubTaskRunner),
            );
        assert!(
            wired.definitions().iter().any(|def| def.name == "task"),
            "with_task_runner must advertise the task tool once a permitted target is attached"
        );
    }

    #[test]
    fn durable_todos_write_to_the_shared_database() {
        let dir = scratch_dir("durable-todos");
        let db_path = dir.join("cortex.db");
        let session_id = "ses_tui_todos";
        {
            let db = db::Database::open(&db_path).expect("open");
            db.sessions().ensure(session_id, None).expect("session");
        }
        let (tx, _rx) = channel();
        let dispatcher = TurnDispatcher::new(
            dir.clone(),
            "google/gemini-2.5-flash",
            session_id.to_owned(),
            tx,
        )
        .with_todo_store(Arc::new(DurableTodos::new(db_path.clone())));

        let result = block_on(dispatcher.dispatch(&call(
            "todo",
            r#"{"todos":[{"content":"visible in TUI","status":"completed"}]}"#,
        )));
        assert!(!result.is_error, "todo should succeed: {}", result.content);

        let db = db::Database::open(&db_path).expect("reopen");
        let todos = db.todos().list_for_session(session_id).expect("todos");
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].content, "visible in TUI");
        assert_eq!(todos[0].status, protocol::TodoStatus::Completed);
    }

    #[test]
    fn tui_dispatcher_audits_shell_execution_with_surface() {
        let dir = scratch_dir("audit");
        let audit = Arc::new(
            audit::AuditSink::new(dir.join("audit.jsonl"), "ses_tui_audit").with_surface("tui"),
        );
        let (tx, _rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx)
                .with_audit(audit.clone());

        let result =
            block_on(dispatcher.dispatch(&call("bash", r#"{"command":"echo tui-audit-marker"}"#)));
        assert!(!result.is_error, "bash should succeed: {}", result.content);

        let records = audit::read_jsonl(audit.path()).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "shell_execution"
                && record["surface"] == "tui"
                && record["source"] == "tui"
                && record["session"] == "ses_tui_audit"
                && record["command"] == "echo tui-audit-marker"
        }));
    }
}
