//! Bridge the `tools` registry to the engine's tool-dispatch seam for headless `cortex run`.
//!
//! The durable runner advertises and executes tools through the [`engine::ToolDispatcher`] seam
//! without depending on concrete tools. This adapter holds a [`ToolRegistry`] and the per-turn
//! [`ToolContext`] and implements that seam by projecting the registry's catalogue and routing each
//! assembled tool call to the registry, converting the structured [`tools::ToolOutput`] into the
//! runner's [`ToolDispatchResult`].

use std::path::PathBuf;
use std::sync::Arc;

use engine::{ModelFamily, ToolDispatchResult, ToolDispatcher};
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use mcp::McpManager;
use permissions::PermissionEngine;
use tools::{
    BackgroundJobs, LspService, MutationHooks, TaskRunner, TaskTarget, TodoStore, ToolContext,
    ToolRegistry,
};

/// A [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`].
///
/// When an [`McpManager`] is attached, the dispatcher also advertises every connected MCP server's
/// tools under their server-qualified ids and routes a call to one of those ids through the manager
/// (a real `tools/call`), so MCP tools materialize alongside the built-in tools for the turn.
pub struct RegistryDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
    mcp: Option<Arc<McpManager>>,
}

impl RegistryDispatcher {
    /// Build a dispatcher whose tool set is materialized for `model_id`'s family (architecture §4):
    /// a GPT-family model receives `apply_patch` (and loses `edit`/`write`), while every other model
    /// keeps the default `edit`/`write` set and has no `apply_patch`. The context is scoped to
    /// `session_id` and carries `todos`, so the per-session `todo` tool persists to (and reads from)
    /// the durable store for this session.
    #[must_use]
    pub fn for_model(
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
            mcp: None,
        }
    }

    /// Attach a connected [`McpManager`] so its server-qualified MCP tools are advertised and
    /// callable this turn (builder style). Without it the dispatcher exposes only built-in tools.
    #[must_use]
    pub fn with_mcp(mut self, manager: Arc<McpManager>) -> Self {
        self.mcp = Some(manager);
        self
    }

    /// Gate every tool effect through `engine` (builder style).
    ///
    /// Without this the headless context runs ungated (every action proceeds, as before this wiring
    /// landed). With it attached, each tool consults the engine via `ctx.permits` before producing
    /// an effect, so a denied `write`/`edit`/`apply_patch`/`bash`/`read` produces no side effect.
    /// MCP tool calls are gated the same way, by their namespaced qualified id (`<server>_<tool>`):
    /// a denied MCP tool is refused before the manager is consulted, so the server receives zero
    /// `tools/call`.
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<PermissionEngine>) -> Self {
        self.context = self.context.with_permissions(engine);
        self
    }

    /// Attach the pre-mutation checkpoint recorder so each mutating tool snapshots a file's prior
    /// state before it changes it (builder style). Without this the headless context records no
    /// checkpoints, so `/undo` has nothing to restore.
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

    /// Attach the `task` tool and the surface-owned durable subagent runner.
    ///
    /// `targets` must already be filtered to exactly the subagent types this parent is allowed to
    /// spawn. When the filtered set is empty the task tool is omitted entirely.
    #[must_use]
    pub fn with_task_runner(
        mut self,
        targets: Vec<TaskTarget>,
        runner: Arc<dyn TaskRunner>,
    ) -> Self {
        if !targets.is_empty() {
            self.registry.register_task(targets);
            self.context = self.context.with_task_runner(runner);
        }
        self
    }

    /// The names of the tools materialized for this turn, in advertised order: the built-in tools
    /// first, then the connected MCP servers' server-qualified tool ids. Surfaced in machine output
    /// so a consumer can observe the per-turn (model-aware) tool list.
    #[cfg(test)]
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> {
        self.definitions().into_iter().map(|def| def.name).collect()
    }

    /// The MCP definitions advertised this turn: server-qualified tools plus synthetic
    /// resources/read and prompts/get wrappers for advertised MCP resources/prompts.
    fn mcp_definitions(&self) -> Vec<ToolDef> {
        self.mcp
            .as_ref()
            .map(|manager| {
                let mut definitions = manager
                    .tools()
                    .iter()
                    .map(|tool| ToolDef {
                        name: tool.id.clone(),
                        description: tool.description.clone().unwrap_or_default(),
                        parameters: tool.input_schema.clone(),
                    })
                    .collect::<Vec<_>>();
                definitions.extend(manager.resources().iter().map(|resource| ToolDef {
                    name: resource.id.clone(),
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
                definitions.extend(manager.prompts().iter().map(|prompt| {
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
                    ToolDef {
                        name: prompt.id.clone(),
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
                definitions
            })
            .unwrap_or_default()
    }

    /// The background-job registry tracking long-running `bash` jobs spawned during the run, so the
    /// caller can reap them (terminate + await by PID) when the run ends — no orphan survives.
    #[must_use]
    pub fn background_jobs(&self) -> Arc<BackgroundJobs> {
        self.context.background_jobs().clone()
    }
}

impl ToolDispatcher for RegistryDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        let mut defs = self.registry.definitions();
        defs.extend(self.mcp_definitions());
        defs
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            // Route a call to a connected MCP tool (matched by its server-qualified id) through the
            // manager as a real `tools/call`; everything else goes to the built-in registry. The
            // MCP call is permission-gated by its namespaced qualified id BEFORE the manager is
            // consulted, so a denied tool is refused here and the server receives zero `tools/call`.
            if let Some(manager) = &self.mcp {
                if manager.has_tool(&call.name)
                    || manager.has_resource(&call.name)
                    || manager.has_prompt(&call.name)
                {
                    if !self.context.permits(&call.name, &call.name) {
                        let message = format!("{}: permission denied", call.name);
                        return ToolDispatchResult {
                            content: message.clone(),
                            parts: vec![protocol::ToolResultPart::text(message)],
                            is_error: true,
                        };
                    }
                    return dispatch_mcp(manager, call).await;
                }
            }
            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;
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
async fn dispatch_mcp(manager: &McpManager, call: &ToolCall) -> ToolDispatchResult {
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

    let result = if manager.has_resource(&call.name) {
        manager.read_resource(&call.name).await
    } else if manager.has_prompt(&call.name) {
        manager.get_prompt(&call.name, arguments).await
    } else {
        manager.call(&call.name, arguments).await
    };

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use permissions::{Action, PermissionEngine, Rule, Ruleset};
    use protocol::TodoItem;

    use super::*;

    /// A todo store stub: `RegistryDispatcher::for_model` requires one, but these tests exercise the
    /// file tools rather than `todo`.
    struct NoopTodos;

    impl TodoStore for NoopTodos {
        fn list(&self, _session_id: &str) -> Result<Vec<TodoItem>, String> {
            Ok(Vec::new())
        }

        fn replace(&self, _session_id: &str, _items: &[TodoItem]) -> Result<(), String> {
            Ok(())
        }
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-perm-wire-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

    #[test]
    fn with_permissions_denies_a_gated_write_with_no_disk_effect() {
        let dir = scratch_dir("write");
        std::fs::write(dir.join("guard.txt"), "KEEP").unwrap();
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "**", Action::Allow),
            Rule::new("write", "guard.txt", Action::Deny),
        ]));
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "google/gemini-2.5-flash",
            "ses_w".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_permissions(Arc::new(engine));

        let result = block_on(dispatcher.dispatch(&call(
            "write",
            r#"{"path":"guard.txt","content":"OVERWRITTEN"}"#,
        )));

        assert!(
            result.is_error,
            "a denied write is an error: {}",
            result.content
        );
        assert!(
            result.content.contains("permission denied"),
            "got: {}",
            result.content
        );
        // The deny produced zero side effects: the file is byte-for-byte unchanged.
        assert_eq!(
            std::fs::read_to_string(dir.join("guard.txt")).unwrap(),
            "KEEP"
        );
    }

    #[test]
    fn an_ungated_dispatcher_still_writes() {
        let dir = scratch_dir("write-ok");
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "google/gemini-2.5-flash",
            "ses_ok".to_owned(),
            Arc::new(NoopTodos),
        );

        let result =
            block_on(dispatcher.dispatch(&call("write", r#"{"path":"ok.txt","content":"hello"}"#)));

        assert!(!result.is_error, "got: {}", result.content);
        assert_eq!(
            std::fs::read_to_string(dir.join("ok.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn with_permissions_denies_a_gated_bash_command_with_no_side_effect() {
        let dir = scratch_dir("bash");
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "bash",
            "touch *",
            Action::Deny,
        )]));
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "google/gemini-2.5-flash",
            "ses_b".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_permissions(Arc::new(engine));

        let result =
            block_on(dispatcher.dispatch(&call("bash", r#"{"command":"touch sentinel.txt"}"#)));

        assert!(result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("permission denied"),
            "got: {}",
            result.content
        );
        assert!(
            !dir.join("sentinel.txt").exists(),
            "a denied bash command must not run"
        );
    }

    #[test]
    fn a_compound_bash_command_with_a_denied_segment_is_refused_atomically() {
        let dir = scratch_dir("compound");
        std::fs::write(dir.join("victim.txt"), "alive").unwrap();
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "**", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]));
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "google/gemini-2.5-flash",
            "ses_c".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_permissions(Arc::new(engine));

        let result = block_on(dispatcher.dispatch(&call(
            "bash",
            r#"{"command":"echo SENTINEL > a.txt && rm victim.txt"}"#,
        )));

        assert!(result.is_error, "got: {}", result.content);
        // Fail-closed atomicity: the allowed `echo` segment did not run either.
        assert!(
            !dir.join("a.txt").exists(),
            "the allowed segment must not run"
        );
        assert!(
            dir.join("victim.txt").exists(),
            "the denied rm must not run"
        );
    }

    #[test]
    fn with_permissions_gates_apply_patch_by_path_for_gpt_models() {
        let dir = scratch_dir("apply-patch");
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "**", Action::Allow),
            Rule::new("apply_patch", "secrets/**", Action::Deny),
        ]));
        // A GPT-family model materializes `apply_patch` (architecture §4).
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "openai/gpt-4o",
            "ses_ap".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_permissions(Arc::new(engine));
        assert!(
            dispatcher.tool_names().iter().any(|n| n == "apply_patch"),
            "a GPT model must materialize apply_patch: {:?}",
            dispatcher.tool_names()
        );

        let denied = block_on(dispatcher.dispatch(&call(
            "apply_patch",
            r#"{"patch":"*** Begin Patch\n*** Add File: secrets/k.txt\n+top secret\n*** End Patch"}"#,
        )));
        assert!(denied.is_error, "got: {}", denied.content);
        assert!(
            !dir.join("secrets/k.txt").exists(),
            "a denied-path apply_patch must create no file"
        );

        let allowed = block_on(dispatcher.dispatch(&call(
            "apply_patch",
            r#"{"patch":"*** Begin Patch\n*** Add File: src/k.txt\n+public\n*** End Patch"}"#,
        )));
        assert!(!allowed.is_error, "got: {}", allowed.content);
        assert_eq!(
            std::fs::read_to_string(dir.join("src/k.txt")).unwrap(),
            "public\n"
        );
    }

    #[test]
    fn with_permissions_gates_read_so_protected_bytes_do_not_leak() {
        let dir = scratch_dir("read");
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        std::fs::write(dir.join("secrets/key.txt"), "SUPERSECRETVALUE").unwrap();
        std::fs::write(dir.join("ok.txt"), "public data").unwrap();
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("*", "**", Action::Allow),
            Rule::new("read", "secrets/**", Action::Deny),
        ]));
        let dispatcher = RegistryDispatcher::for_model(
            dir.clone(),
            "google/gemini-2.5-flash",
            "ses_r".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_permissions(Arc::new(engine));

        let denied = block_on(dispatcher.dispatch(&call("read", r#"{"path":"secrets/key.txt"}"#)));
        assert!(denied.is_error, "got: {}", denied.content);
        assert!(
            !denied.content.contains("SUPERSECRETVALUE"),
            "protected bytes must not appear in the tool result: {}",
            denied.content
        );

        let allowed = block_on(dispatcher.dispatch(&call("read", r#"{"path":"ok.txt"}"#)));
        assert!(!allowed.is_error, "got: {}", allowed.content);
        assert!(
            allowed.content.contains("public data"),
            "an allowed read returns its content: {}",
            allowed.content
        );
    }

    /// Resolve the compiled echo-server fixture next to the test runner's target dir, or `None`
    /// when it has not been built (so the gating test skips cleanly rather than failing).
    fn echo_server_bin() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        // .../target/debug/deps/<test-bin>  ->  .../target/debug/cortex-mcp-echo-server
        let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
        bin.exists().then_some(bin)
    }

    // VAL-MCP-017: an MCP tool call is gated by the permission engine on its namespaced qualified
    // id — a deny rule blocks it before any `tools/call` reaches the server, and an allow rule lets
    // the real call through. Proven against the real stdio echo-server fixture (no LLM needed): the
    // server log records exactly the allowed call and never the denied one.
    #[test]
    fn mcp_tool_calls_are_permission_gated() {
        let Some(bin) = echo_server_bin() else {
            eprintln!("skipping: cortex-mcp-echo-server fixture not built");
            return;
        };
        let log = std::env::temp_dir().join(format!(
            "cortex-mcp-gate-log-{}-{:?}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&log);

        let servers = mcp::servers_from_value(&serde_json::json!({
            "demo": {
                "type": "local",
                "command": [bin.to_string_lossy()],
                "environment": { "CORTEX_MCP_ECHO_LOG": log.to_string_lossy() }
            }
        }))
        .expect("valid mcp config");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let manager = Arc::new(McpManager::connect(servers).await);
            assert!(
                manager.has_tool("demo_echo"),
                "the echo server exposes demo_echo"
            );

            // Deny: the call is refused here; the server must receive zero `tools/call`.
            let deny = PermissionEngine::new(Ruleset::new(vec![Rule::new(
                "demo_echo",
                "*",
                Action::Deny,
            )]));
            let denied_dispatcher = RegistryDispatcher::for_model(
                std::env::temp_dir(),
                "google/gemini-2.5-flash",
                "ses_deny".to_owned(),
                Arc::new(NoopTodos),
            )
            .with_permissions(Arc::new(deny))
            .with_mcp(manager.clone());
            let denied = denied_dispatcher
                .dispatch(&call("demo_echo", r#"{"message":"hi"}"#))
                .await;
            assert!(
                denied.is_error,
                "a denied MCP tool returns an error: {}",
                denied.content
            );
            assert!(
                denied.content.contains("permission denied"),
                "got: {}",
                denied.content
            );

            // Allow: the same call now reaches the server and returns its real result.
            let allow = PermissionEngine::new(Ruleset::new(vec![Rule::new(
                "demo_echo",
                "*",
                Action::Allow,
            )]));
            let allowed_dispatcher = RegistryDispatcher::for_model(
                std::env::temp_dir(),
                "google/gemini-2.5-flash",
                "ses_allow".to_owned(),
                Arc::new(NoopTodos),
            )
            .with_permissions(Arc::new(allow))
            .with_mcp(manager.clone());
            let allowed = allowed_dispatcher
                .dispatch(&call("demo_echo", r#"{"message":"hi"}"#))
                .await;
            assert!(
                !allowed.is_error,
                "an allowed MCP tool succeeds: {}",
                allowed.content
            );
            assert!(allowed.content.contains("hi"), "got: {}", allowed.content);

            manager.shutdown().await;

            // The log proves exactly one `tools/call` reached the server: the allowed one, never
            // the denied one.
            let contents = std::fs::read_to_string(&log).unwrap_or_default();
            let calls = contents.matches("tools/call name=echo").count();
            assert_eq!(
                calls, 1,
                "only the allowed call reaches the server; log:\n{contents}"
            );
        });
        let _ = std::fs::remove_file(&log);
    }
}
