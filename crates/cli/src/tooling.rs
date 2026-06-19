//! Bridge the `tools` registry to the engine's tool-dispatch seam for headless `cortex run`.
//!
//! The durable runner advertises and executes tools through the [`engine::ToolDispatcher`] seam
//! without depending on concrete tools. This adapter holds a [`ToolRegistry`] and the per-turn
//! [`ToolContext`] and implements that seam by projecting the registry's catalogue and routing each
//! assembled tool call to the registry, converting the structured [`tools::ToolOutput`] into the
//! runner's [`ToolDispatchResult`].

use std::path::PathBuf;

use engine::{ToolDispatchResult, ToolDispatcher};
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use tools::{ToolContext, ToolRegistry};

/// A [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`].
pub struct RegistryDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
}

impl RegistryDispatcher {
    /// Build a dispatcher over the built-in tool set, resolving relative paths against `cwd`.
    #[must_use]
    pub fn builtin(cwd: PathBuf) -> Self {
        Self {
            registry: ToolRegistry::builtin(),
            context: ToolContext::new(cwd),
        }
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
