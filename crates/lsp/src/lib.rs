//! Real Language Server Protocol support for Cortex Code.
//!
//! This crate resolves a server by file extension, starts the real language server over stdio
//! JSON-RPC, completes the initialize/initialized handshake, and collects publishDiagnostics after
//! file mutations. Auto-install is guarded by `CORTEX_NO_NET` before any package manager command can
//! open a socket; pre-installed local servers remain usable offline.

mod client;
mod config;
mod diagnostics;
mod error;
mod feedback;
mod navigation;
mod protocol;
mod transport;

pub use client::{LspClient, LspServerCommand};
pub use config::{InstallPlan, LspConfigSet, LspServerConfig};
pub use diagnostics::{
    Diagnostic, DiagnosticSeverity, DiagnosticStore, Position, PublishDiagnosticsParams, Range,
};
pub use error::LspError;
pub use feedback::{
    FeedbackManager, FeedbackOutcome, FileDiagnostic, NavigationOutcome, NavigationRequest,
    ServerRunState, ServerStatus, render_feedback_block,
};
pub use navigation::{
    DocumentSymbol, Hover, Location, SymbolInformation, SymbolKind, document_symbols_context_block,
    hover_context_block, locations_context_block, workspace_symbols_context_block,
};
pub use protocol::{InitializeParams, InitializeResult, ServerCapabilities, ServerInfo};
