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
