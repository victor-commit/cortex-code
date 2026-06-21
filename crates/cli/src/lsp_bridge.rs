//! CLI bridge from the tool-layer mutation hook seam to the real LSP diagnostics manager.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lsp::{FeedbackManager, FeedbackOutcome, FileDiagnostic, LspConfigSet, ServerStatus};

/// Shared post-mutation LSP hook attached to headless `cortex run` tool contexts.
pub struct LspHooks {
    manager: Mutex<FeedbackManager>,
}

impl LspHooks {
    /// Build hooks rooted at the current run working directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            manager: Mutex::new(FeedbackManager::new(root, LspConfigSet::defaults())),
        }
    }

    /// Shut down every language server spawned by these hooks by its exact child handle/PID.
    pub fn shutdown(&self) {
        if let Ok(mut manager) = self.manager.lock() {
            manager.shutdown_all();
        }
    }
}

impl tools::MutationHooks for LspHooks {
    fn diagnostic_feedback(&self, path: &Path) -> tools::DiagnosticFeedback {
        let Ok(mut manager) = self.manager.lock() else {
            return tools::DiagnosticFeedback::default();
        };
        match manager.collect(path) {
            FeedbackOutcome::Diagnostics(diagnostics) => tools::DiagnosticFeedback::diagnostics(
                diagnostics.into_iter().map(to_tool_diagnostic).collect(),
            ),
            FeedbackOutcome::NoServer => tools::DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some(no_server_note(path)),
            },
            FeedbackOutcome::Unavailable => tools::DiagnosticFeedback {
                diagnostics: Vec::new(),
                note: Some(format!(
                    "LSP diagnostics unavailable for {}: language server stopped responding",
                    path.display()
                )),
            },
        }
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

/// Snapshot configured LSP server status without launching any server.
#[must_use]
pub fn current_status(root: impl Into<PathBuf>) -> Vec<ServerStatus> {
    let mut manager = FeedbackManager::new(root, LspConfigSet::defaults());
    manager.statuses()
}
