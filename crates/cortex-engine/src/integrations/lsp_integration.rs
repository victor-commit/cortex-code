//! LSP integration for cortex-core.
//!
//! Connects the cortex-lsp crate to provide diagnostics in tool results.

use cortex_lsp::{Diagnostic, DiagnosticSeverity, LspManager};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

// Type aliases for LSP results (using Value for flexibility)
pub type Location = Value;
pub type DocumentSymbol = Value;
pub type WorkspaceSymbol = Value;
pub type CallHierarchyItem = Value;
pub type CallHierarchyIncomingCall = Value;
pub type CallHierarchyOutgoingCall = Value;
pub type CompletionItem = Value;
pub type SignatureHelp = Value;
pub type WorkspaceEdit = Value;
pub type CodeAction = Value;

/// LSP integration for the agent.
pub struct LspIntegration {
    manager: Arc<RwLock<Option<LspManager>>>,
    enabled: bool,
}

impl LspIntegration {
    /// Create a new LSP integration.
    pub fn new(enabled: bool) -> Self {
        Self {
            manager: Arc::new(RwLock::new(None)),
            enabled,
        }
    }

    /// Initialize LSP for a workspace.
    pub async fn init(&self, workspace_root: &Path) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let manager = LspManager::new(workspace_root.to_path_buf());
        debug!("Initialized LSP manager for {:?}", workspace_root);

        *self.manager.write().await = Some(manager);
        Ok(())
    }

    /// Get diagnostics for a file.
    pub async fn get_diagnostics(&self, file_path: &Path) -> Vec<Diagnostic> {
        let guard = self.manager.read().await;
        if let Some(ref manager) = *guard {
            manager.get_diagnostics(file_path).await
        } else {
            Vec::new()
        }
    }

    /// Get all diagnostics for the workspace.
    pub async fn get_all_diagnostics(&self) -> HashMap<PathBuf, Vec<Diagnostic>> {
        let guard = self.manager.read().await;
        if let Some(ref manager) = *guard {
            manager.all_diagnostics().await
        } else {
            HashMap::new()
        }
    }

    /// Get diagnostics for multiple files.
    pub async fn get_diagnostics_batch(
        &self,
        files: &[PathBuf],
    ) -> Vec<(PathBuf, Vec<Diagnostic>)> {
        let mut results = Vec::new();

        for file in files {
            let diagnostics = self.get_diagnostics(file).await;
            if !diagnostics.is_empty() {
                results.push((file.clone(), diagnostics));
            }
        }

        results
    }

    /// Get hover information for a position.
    pub async fn hover(
        &self,
        path: &str,
        line: u32,
        column: u32,
    ) -> anyhow::Result<Option<String>> {
        let guard = self.manager.read().await;
        if let Some(ref manager) = *guard {
            manager
                .hover(Path::new(path), line, column)
                .await
                .map_err(anyhow::Error::from)
        } else {
            Ok(None)
        }
    }

    fn unsupported_operation(operation: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "LSP operation '{operation}' is unsupported: cortex-lsp currently exposes diagnostics and hover only"
        )
    }

    /// Go to definition.
    pub async fn go_to_definition(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<Location>> {
        Err(Self::unsupported_operation("goToDefinition"))
    }

    /// Find references.
    pub async fn find_references(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<Location>> {
        Err(Self::unsupported_operation("findReferences"))
    }

    /// Get document symbols.
    pub async fn document_symbols(&self, _path: &str) -> anyhow::Result<Vec<DocumentSymbol>> {
        Err(Self::unsupported_operation("documentSymbol"))
    }

    /// Search workspace symbols.
    pub async fn workspace_symbols(&self, _query: &str) -> anyhow::Result<Vec<WorkspaceSymbol>> {
        Err(Self::unsupported_operation("workspaceSymbol"))
    }

    /// Go to implementation.
    pub async fn go_to_implementation(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<Location>> {
        Err(Self::unsupported_operation("goToImplementation"))
    }

    /// Prepare call hierarchy.
    pub async fn prepare_call_hierarchy(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<CallHierarchyItem>> {
        Err(Self::unsupported_operation("prepareCallHierarchy"))
    }

    /// Get incoming calls.
    pub async fn incoming_calls(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<CallHierarchyIncomingCall>> {
        Err(Self::unsupported_operation("incomingCalls"))
    }

    /// Get outgoing calls.
    pub async fn outgoing_calls(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<CallHierarchyOutgoingCall>> {
        Err(Self::unsupported_operation("outgoingCalls"))
    }

    /// Get completions.
    pub async fn completions(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<CompletionItem>> {
        Err(Self::unsupported_operation("completions"))
    }

    /// Get signature help.
    pub async fn signature_help(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Option<SignatureHelp>> {
        Err(Self::unsupported_operation("signatureHelp"))
    }

    /// Rename symbol.
    pub async fn rename(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
        _new_name: &str,
    ) -> anyhow::Result<WorkspaceEdit> {
        Err(Self::unsupported_operation("rename"))
    }

    /// Get code actions.
    pub async fn code_actions(
        &self,
        _path: &str,
        _line: u32,
        _column: u32,
    ) -> anyhow::Result<Vec<CodeAction>> {
        Err(Self::unsupported_operation("codeActions"))
    }

    /// Format diagnostics for display in tool results.
    pub fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
        if diagnostics.is_empty() {
            return String::new();
        }

        let mut output = String::from("\n\n--- LSP Diagnostics ---\n");

        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect();

        let warnings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .collect();

        if !errors.is_empty() {
            output.push_str(&format!("\nErrors ({}):\n", errors.len()));
            for diag in errors.iter().take(5) {
                output.push_str(&format!("  Line {}: {}\n", diag.line + 1, diag.message));
            }
            if errors.len() > 5 {
                output.push_str(&format!("  ... and {} more errors\n", errors.len() - 5));
            }
        }

        if !warnings.is_empty() {
            output.push_str(&format!("\nWarnings ({}):\n", warnings.len()));
            for diag in warnings.iter().take(3) {
                output.push_str(&format!("  Line {}: {}\n", diag.line + 1, diag.message));
            }
            if warnings.len() > 3 {
                output.push_str(&format!("  ... and {} more warnings\n", warnings.len() - 3));
            }
        }

        output
    }

    /// Shutdown LSP servers.
    pub async fn shutdown(&self) {
        *self.manager.write().await = None;
    }

    /// Check if LSP is enabled and running.
    pub async fn is_running(&self) -> bool {
        self.enabled && self.manager.read().await.is_some()
    }
}

impl Default for LspIntegration {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unimplemented_lsp_operations_return_unsupported_errors() {
        let lsp = LspIntegration::new(false);

        let cases = [
            lsp.go_to_definition("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.find_references("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.document_symbols("src/lib.rs").await.map(|_| ()),
            lsp.workspace_symbols("query").await.map(|_| ()),
            lsp.go_to_implementation("src/lib.rs", 1, 1)
                .await
                .map(|_| ()),
            lsp.prepare_call_hierarchy("src/lib.rs", 1, 1)
                .await
                .map(|_| ()),
            lsp.incoming_calls("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.outgoing_calls("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.completions("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.signature_help("src/lib.rs", 1, 1).await.map(|_| ()),
            lsp.rename("src/lib.rs", 1, 1, "new_name").await.map(|_| ()),
            lsp.code_actions("src/lib.rs", 1, 1).await.map(|_| ()),
        ];

        for result in cases {
            let message = result
                .expect_err("operation should be unsupported")
                .to_string();
            assert!(message.contains("LSP operation"));
            assert!(message.contains("is unsupported"));
            assert!(message.contains("diagnostics and hover only"));
        }
    }
}
