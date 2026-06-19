// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/renderer/mod.rs
// Target-Path: crates/frame/src/markdown/renderer/mod.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame; the optional CodeBlockRenderer seam
//   (tree-sitter syntax highlighting via tui-syntax) is removed -- fenced code blocks render
//   via the built-in simple bordered container. Syntax highlighting is the syntax-highlight
//   feature's responsibility.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Markdown Renderer - Main entry point for markdown rendering.
//!
//! This module provides the main `MarkdownRenderer` and `IncrementalMarkdownRenderer`
//! for converting markdown text to styled ratatui `Line`s.
//!
//! # Architecture
//!
//! The renderer uses pulldown-cmark for parsing and delegates to specialized
//! renderers for different block types:
//! - `CodeBlockRenderer` for fenced code blocks with syntax highlighting
//! - `TableBuilder` for tables with ASCII borders
//! - `ListContext` for ordered, unordered, and task lists
//!
//! # Example
//!
//! ```rust,ignore
//! use frame::markdown::{MarkdownRenderer, IncrementalMarkdownRenderer};
//!
//! // One-shot rendering
//! let renderer = MarkdownRenderer::new().with_width(80);
//! let lines = renderer.render("# Hello **World**");
//!
//! // Streaming/incremental rendering
//! let mut incremental = IncrementalMarkdownRenderer::new(renderer);
//! incremental.append("Some ");
//! incremental.append("content...");
//! let lines = incremental.get_lines();
//! ```

mod handlers;
mod helpers;
mod incremental;
mod state;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use pulldown_cmark::{Options, Parser};
use ratatui::text::Line;

use crate::markdown::theme::MarkdownTheme;

pub use self::incremental::IncrementalMarkdownRenderer;
use self::state::RenderState;

// ============================================================
// Main MarkdownRenderer
// ============================================================

/// Main markdown renderer.
///
/// Converts markdown text to styled ratatui `Line`s using pulldown-cmark
/// for parsing and specialized renderers for different block types.
#[derive(Clone)]
pub struct MarkdownRenderer {
    /// Theme for styling markdown elements.
    pub(crate) theme: Arc<MarkdownTheme>,
    /// Maximum width for rendering (used for word wrapping).
    pub(crate) width: u16,
}

impl std::fmt::Debug for MarkdownRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownRenderer")
            .field("theme", &"<MarkdownTheme>")
            .field("width", &self.width)
            .finish()
    }
}

impl MarkdownRenderer {
    /// Creates a new markdown renderer with default theme.
    pub fn new() -> Self {
        Self {
            theme: Arc::new(MarkdownTheme::default()),
            width: 80,
        }
    }

    /// Creates a renderer with a custom theme.
    pub fn with_theme(theme: MarkdownTheme) -> Self {
        Self {
            theme: Arc::new(theme),
            width: 80,
        }
    }

    /// Sets the rendering width.
    #[must_use]
    pub fn with_width(mut self, width: u16) -> Self {
        self.width = width;
        self
    }

    /// Renders markdown to lines.
    ///
    /// This is the main entry point for rendering markdown content.
    ///
    /// # Arguments
    /// * `markdown` - The markdown source text
    ///
    /// # Returns
    /// A vector of styled `Line`s ready for display.
    pub fn render(&self, markdown: &str) -> Vec<Line<'static>> {
        let mut state = RenderState::new(self);
        let parser = Parser::new_ext(markdown, Self::options());

        for event in parser {
            state.handle_event(event);
        }

        state.finish()
    }

    /// Get pulldown-cmark options for parsing.
    fn options() -> Options {
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_HEADING_ATTRIBUTES
    }

    /// Returns the theme.
    pub fn theme(&self) -> &MarkdownTheme {
        &self.theme
    }

    /// Returns the width.
    pub fn width(&self) -> u16 {
        self.width
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}
