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
use permissions::PermissionEngine;
use tools::{BackgroundJobs, TodoStore, ToolContext, ToolRegistry};

/// A [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`].
pub struct RegistryDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
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
        }
    }

    /// Gate every tool effect through `engine` (builder style).
    ///
    /// Without this the headless context runs ungated (every action proceeds, as before this wiring
    /// landed). With it attached, each tool consults the engine via `ctx.permits` before producing
    /// an effect, so a denied `write`/`edit`/`apply_patch`/`bash`/`read` produces no side effect.
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

    /// The names of the tools materialized for this turn, in advertised order. Surfaced in machine
    /// output so a consumer can observe the per-turn (model-aware) tool list.
    #[must_use]
    pub fn tool_names(&self) -> Vec<String> {
        self.registry
            .definitions()
            .into_iter()
            .map(|def| def.name)
            .collect()
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
                parts: output.parts,
                is_error: output.is_error,
            }
        })
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
}
