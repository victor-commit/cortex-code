// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/table/mod.rs
// Target-Path: crates/frame/src/markdown/table/mod.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame (cortex_frame -> frame import paths); deps re-pointed to this workspace.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! ASCII Table Renderer
//!
//! Provides complete ASCII box-drawing table rendering with:
//! - Full box-drawing border characters
//! - Column alignment (left, center, right)
//! - Unicode-aware width calculation
//! - Automatic column width distribution
//! - Content truncation with ellipsis
//!
//! ## Example Output
//!
//! ```text
//! ┌──────────┬───────────┬─────────┐
//! │ Header 1 │ Header 2  │ Header 3│
//! ├──────────┼───────────┼─────────┤
//! │ Cell 1   │ Cell 2    │ Cell 3  │
//! │ Cell 4   │ Cell 5    │ Cell 6  │
//! └──────────┴───────────┴─────────┘
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use frame::markdown::table::{TableBuilder, render_table, Alignment};
//! use ratatui::style::{Style, Color};
//!
//! let mut builder = TableBuilder::new();
//! builder.start_header();
//! builder.add_cell("Name".to_string());
//! builder.add_cell("Value".to_string());
//! builder.end_header();
//!
//! builder.start_row();
//! builder.add_cell("foo".to_string());
//! builder.add_cell("bar".to_string());
//! builder.end_row();
//!
//! builder.set_alignments(vec![Alignment::Left, Alignment::Right]);
//! let table = builder.build();
//!
//! let lines = render_table(
//!     &table,
//!     Color::Gray,
//!     Style::default().fg(Color::White),
//!     Style::default(),
//!     80,
//! );
//! ```

// Sub-modules
pub mod border;
mod builder;
mod render;
#[cfg(test)]
mod tests;
mod types;
pub mod utils;

// Re-exports for backwards compatibility
pub use builder::TableBuilder;
pub use render::{render_table, render_table_simple};
pub use types::{Alignment, Table, TableCell};
