//! TUI bridge from tool-layer LSP seams to the real language-server diagnostics manager.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lsp::{
    FeedbackManager, FeedbackOutcome, FileDiagnostic, LspConfigSet, NavigationOutcome,
    NavigationRequest,
};

/// Shared LSP hook attached to TUI tool contexts for diagnostics and the read-only `lsp` tool.
pub(crate) struct LspHooks {
    manager: Mutex<FeedbackManager>,
}

impl LspHooks {
    /// Build hooks rooted at the TUI working directory.
    #[must_use]
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            manager: Mutex::new(FeedbackManager::new(root, LspConfigSet::defaults())),
        }
    }

    /// Shut down every language server spawned by these hooks by its exact child handle/PID.
    pub(crate) fn shutdown(&self) {
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

impl tools::LspService for LspHooks {
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
