//! # frame
//!
//! Markdown rendering for the Cortex TUI, ported from the authorized upstream TUI framework
//! (`CortexLM/cortex-code@7954d02`, `src/cortex-core`). This crate converts assistant markdown
//! into styled `ratatui` lines: headings, bold/italic/strikethrough emphasis, inline code,
//! bulleted/numbered/task lists, fenced code blocks (in a distinct bordered container), tables,
//! and blockquotes, with incremental rendering for streaming content.
//!
//! Tree-sitter syntax highlighting of code blocks is intentionally NOT part of this crate; it is
//! layered on by the syntax-highlight feature (via `tui-syntax`). Fenced code blocks here render
//! in a plain bordered container that preserves line breaks and indentation.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use frame::markdown::{render_markdown, MarkdownRenderer};
//!
//! // One-shot
//! let lines = render_markdown("# Hello **world**", 80);
//!
//! // Reusable renderer
//! let renderer = MarkdownRenderer::new().with_width(80);
//! let lines = renderer.render("## Overview\n\nSome text.");
//! ```

pub mod markdown;
pub mod style;

pub use markdown::{MarkdownRenderer, MarkdownTheme, render_markdown};
