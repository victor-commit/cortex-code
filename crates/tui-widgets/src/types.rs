// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-widgets/src/types.rs
// Target-Path: crates/tui-widgets/src/types.rs
// Port-Mode: direct-copy
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)
// Upstream notices retained where present; product distributed under Apache-2.0; see NOTICE.

//! Core types for the widgets crate.

pub use tui_buffer::Cell;
pub use tui_core::color::Color;
pub use tui_core::geometry::{Point, Rect, Size};
pub use tui_core::style::{Style, TextAttributes};

/// RGBA color with 8-bit components.
/// Re-exported as Color from tui_core for compatibility.
pub use tui_core::Color as RGBA;

/// Edge values for padding, margin, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

impl Edges {
    /// Creates new edge values.
    pub const fn new(top: u16, right: u16, bottom: u16, left: u16) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Creates uniform edge values.
    pub const fn uniform(value: u16) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// Creates symmetric edge values (vertical, horizontal).
    pub const fn symmetric(vertical: u16, horizontal: u16) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }

    /// Zero edges.
    pub const ZERO: Self = Self {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };

    /// Returns the total horizontal size.
    pub const fn horizontal(&self) -> u16 {
        self.left.saturating_add(self.right)
    }

    /// Returns the total vertical size.
    pub const fn vertical(&self) -> u16 {
        self.top.saturating_add(self.bottom)
    }
}
