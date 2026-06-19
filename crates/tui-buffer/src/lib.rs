// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-buffer/src/lib.rs
// Target-Path: crates/tui-buffer/src/lib.rs
// Port-Mode: adapt
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)
// Upstream notices retained where present; product distributed under Apache-2.0; see NOTICE.
//! Double-buffered terminal rendering for Cortex TUI.
//!
//! This crate provides efficient buffer management for terminal UIs:
//!
//! - [`Cell`] - A single character cell with colors and attributes
//! - [`Buffer`] - A 2D grid of cells with clipping and rendering operations
//! - [`DoubleBuffer`] - Double-buffered rendering for flicker-free updates
//! - [`diff`] - Algorithms for computing minimal buffer updates
//!
//! # Architecture
//!
//! The rendering pipeline works as follows:
//!
//! 1. **Render to back buffer**: Use [`DoubleBuffer::back_mut()`] to get the
//!    back buffer and draw content using [`Buffer::draw_str`], [`Buffer::fill`], etc.
//!
//! 2. **Compute diff**: Call [`DoubleBuffer::diff`] to compare the back buffer
//!    with the front buffer and get a list of changes.
//!
//! 3. **Send to terminal**: Use the diff to emit ANSI escape sequences for
//!    only the changed cells (handled by `cortex-tui-terminal`).
//!
//! 4. **Swap buffers**: Call [`DoubleBuffer::swap`] to make the back buffer
//!    the new front buffer.
//!
//! # Examples
//!
//! Basic double-buffered rendering:
//!
//! ```
//! use tui_buffer::{Buffer, Cell, DoubleBuffer, diff::DiffOptions};
//! use tui_core::{Color, Style, TextAttributes};
//!
//! // Create a double buffer for an 80x24 terminal
//! let mut db = DoubleBuffer::new(80, 24);
//!
//! // Clear with a background color
//! db.clear_with_bg(Color::BLACK);
//!
//! // Draw some styled text
//! let title_style = Style::new()
//!     .fg(Color::CYAN)
//!     .bold();
//! db.back_mut().draw_str(10, 2, "Welcome to Cortex TUI!", title_style);
//!
//! // Draw a box
//! let border_style = Style::new().fg(Color::WHITE);
//! db.back_mut().draw_str(5, 4, "┌────────────────────┐", border_style);
//! db.back_mut().draw_str(5, 5, "│                    │", border_style);
//! db.back_mut().draw_str(5, 6, "└────────────────────┘", border_style);
//!
//! // Compute the changes
//! let diff = db.diff_default();
//!
//! // In real code: render diff to terminal via cortex-tui-terminal
//! // terminal.render_diff(&diff);
//!
//! // Swap buffers for next frame
//! db.swap();
//! ```
//!
//! Using scissor rectangles for clipping:
//!
//! ```
//! use tui_buffer::Buffer;
//! use tui_core::{Rect, Style};
//!
//! let mut buffer = Buffer::new(80, 24);
//!
//! // Push a scissor rectangle to clip rendering
//! buffer.push_scissor(Rect::new(10, 5, 30, 10));
//!
//! // This text will be clipped to the scissor rect
//! buffer.draw_str(0, 7, "This long text will be clipped at the scissor boundaries", Style::default());
//!
//! // Pop the scissor when done
//! buffer.pop_scissor();
//! ```

mod buffer;
mod cell;
pub mod diff;
mod double_buffer;
pub mod grapheme;

pub use buffer::Buffer;
pub use cell::Cell;
pub use diff::{BufferDiff, CellChange, ChangeRun, DiffOptions};
pub use double_buffer::DoubleBuffer;
pub use grapheme::{GraphemeId, GraphemePool};

// Re-export core types for convenience
pub use tui_core::{Color, Rect, Style, TextAttributes};
