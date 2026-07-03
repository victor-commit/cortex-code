//! CLI adapter for the shared subagent `task` runner.
//!
//! The durable subagent child-turn runner (session creation, permission narrowing, child tool
//! materializing, PID cleanup) lives in the [`subagent_runner`] crate, shared with the TUI so both
//! surfaces drive identical behavior without a `cli <-> tui` dependency cycle. The CLI contributes
//! one surface-specific piece: wrapping a child turn's provider with the plugin request-shaping
//! middleware, attributed to the child session's audit sink.

use std::sync::Arc;

use llm::Provider;
use subagent_runner::ChildProviderWrapper;

pub(crate) use subagent_runner::{
    CliTaskRunner, CliTaskRunnerConfig, ProviderAuditConfig, permitted_catalog,
};

use crate::plugin_bridge::PluginState;

/// A [`ChildProviderWrapper`] that runs a child turn's provider requests through the CLI's plugin
/// request-shaping middleware, attributing plugin-initiated records to the child session's audit
/// sink. This mirrors the parent run's provider composition so a subagent turn shapes requests
/// exactly like the parent.
pub(crate) struct PluginProviderWrapper {
    plugin_state: PluginState,
}

impl PluginProviderWrapper {
    pub(crate) fn new(plugin_state: PluginState) -> Self {
        Self { plugin_state }
    }
}

impl ChildProviderWrapper for PluginProviderWrapper {
    fn wrap(
        &self,
        base: Arc<dyn Provider>,
        child_audit: &Arc<audit::AuditSink>,
    ) -> Arc<dyn Provider> {
        self.plugin_state
            .clone_with_audit(Some(child_audit.clone()))
            .wrap_provider(base)
    }
}
