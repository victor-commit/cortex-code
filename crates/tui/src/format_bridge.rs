//! TUI bridge from the tool-layer mutation hook seam to the real post-mutation formatter.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tools::{DiagnosticFeedback, MutationHooks};

/// TUI post-mutation hooks: run the real formatter first, then delegate diagnostics to the LSP hook.
pub(crate) struct PostMutationHooks {
    formatter: format::FormatterRunner,
    diagnostics: Arc<dyn MutationHooks>,
}

impl PostMutationHooks {
    /// Build hooks rooted at the current TUI working directory with config-driven formatter
    /// overrides.
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
