//! The tool abstraction: the [`Tool`] trait, the per-session [`ToolContext`], and the structured
//! [`ToolOutput`] every tool produces.
//!
//! A [`Tool`] advertises a name, a model-facing description, and a JSON-Schema parameter object,
//! and executes an assembled arguments object against a [`ToolContext`]. The context carries the
//! session working directory, the per-session read-tracking set (the freshness ledger that the
//! mutating tools consult before overwriting a file), and the optional permission engine the tool
//! consults before producing an effect. A tool never aborts the turn: a failure is returned as a
//! [`ToolOutput`] with [`is_error`](ToolOutput::is_error) set so the model can recover.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use permissions::{PermissionEngine, request_for};
use protocol::ToolResultPart;

/// The structured result of running a tool.
///
/// The parts are surfaced verbatim to machine consumers; [`model_content`](ToolOutput::model_content)
/// renders the text the model reads back. A non-text attachment (an image/PDF) is carried as a typed
/// part so binary bytes never enter the text channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// The structured result parts (text and/or attachment references).
    pub parts: Vec<ToolResultPart>,
    /// Whether the tool reported an error (the turn still continues).
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful result carrying a single text part.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![ToolResultPart::text(text)],
            is_error: false,
        }
    }

    /// An error result carrying a single text part describing the failure.
    #[must_use]
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            parts: vec![ToolResultPart::text(text)],
            is_error: true,
        }
    }

    /// A successful result built from explicit parts.
    #[must_use]
    pub fn parts(parts: Vec<ToolResultPart>) -> Self {
        Self {
            parts,
            is_error: false,
        }
    }

    /// The text the model reads back: every text part joined by newlines, with any attachment
    /// described by its media type so a text-only transcript still records what was returned.
    #[must_use]
    pub fn model_content(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for part in &self.parts {
            match part {
                ToolResultPart::Text { text } => lines.push(text.clone()),
                ToolResultPart::Image { media_type, path } => {
                    lines.push(format!("[image attachment: {path} ({media_type})]"));
                }
                ToolResultPart::Document { media_type, path } => {
                    lines.push(format!("[document attachment: {path} ({media_type})]"));
                }
            }
        }
        lines.join("\n")
    }
}

/// The per-session execution context shared across every tool call in a turn.
///
/// It resolves relative paths against the session working directory, tracks which files have been
/// read (so a later mutating tool can require a prior read), and optionally gates a tool's effect
/// through the permission engine.
#[derive(Clone)]
pub struct ToolContext {
    cwd: PathBuf,
    reads: Arc<Mutex<HashSet<PathBuf>>>,
    permissions: Option<Arc<PermissionEngine>>,
}

impl ToolContext {
    /// Build a context rooted at `cwd` with an empty read-tracking set and no permission gating.
    #[must_use]
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            reads: Arc::new(Mutex::new(HashSet::new())),
            permissions: None,
        }
    }

    /// Attach a permission engine that gates each tool's effect (builder style).
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<PermissionEngine>) -> Self {
        self.permissions = Some(engine);
        self
    }

    /// The session working directory relative paths resolve against.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Resolve `path` against the working directory: an absolute path is returned as-is; a relative
    /// path is joined onto the working directory so the same relative path denotes the same file
    /// across every tool in the session.
    #[must_use]
    pub fn resolve(&self, path: &str) -> PathBuf {
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.cwd.join(candidate)
        }
    }

    /// Record that `path` (a resolved absolute path) has been read this session.
    pub fn record_read(&self, path: impl Into<PathBuf>) {
        if let Ok(mut reads) = self.reads.lock() {
            reads.insert(path.into());
        }
    }

    /// Whether `path` (a resolved absolute path) has been read this session.
    #[must_use]
    pub fn was_read(&self, path: &Path) -> bool {
        self.reads
            .lock()
            .map(|reads| reads.contains(path))
            .unwrap_or(false)
    }

    /// Whether the permission engine (when attached) allows `tool` to act on `target`.
    ///
    /// With no engine attached every action is allowed, so a tool can run unconditionally until the
    /// enforcement layer wires a real engine in. With an engine attached the action's subject is
    /// scoped by the tool (a file tool scopes by path) and resolved to allow/deny.
    #[must_use]
    pub fn permits(&self, tool: &str, target: &str) -> bool {
        match &self.permissions {
            Some(engine) => engine.evaluate(&request_for(tool, target)).is_allowed(),
            None => true,
        }
    }
}

/// A callable agent tool.
pub trait Tool: Send + Sync {
    /// The tool name the model calls (e.g. `read`).
    fn name(&self) -> &str;

    /// The model-facing description of when and how to use the tool.
    fn description(&self) -> &str;

    /// The JSON Schema describing the tool's arguments object.
    fn parameters(&self) -> serde_json::Value;

    /// Execute the tool with an assembled `arguments` object against `ctx`.
    fn execute<'a>(
        &'a self,
        arguments: serde_json::Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, ToolOutput>;
}
