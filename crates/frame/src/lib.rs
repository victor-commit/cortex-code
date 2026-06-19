//! # frame
//!
//! Markdown rendering for the Cortex TUI, ported from the authorized upstream TUI framework
//! (`CortexLM/cortex-code@7954d02`, `src/cortex-core`). This crate converts assistant markdown
//! into styled `ratatui` lines: headings, bold/italic/strikethrough emphasis, inline code,
//! bulleted/numbered/task lists, fenced code blocks (in a distinct bordered container), tables,
//! and blockquotes, with incremental rendering for streaming content.
//!
//! Fenced code blocks are syntax-highlighted via `tui-syntax` (tree-sitter) when the fence info
//! string maps to a registered grammar (rust/bash); unknown languages and still-streaming partial
//! blocks fall back to a plain bordered container that preserves line breaks and indentation.
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
