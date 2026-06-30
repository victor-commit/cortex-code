// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-tui-components/src/spinner.rs
// Target-Path: crates/tui-components/src/spinner.rs
// Port-Mode: adapt
// Adaptation: renamed frame-engine crate references (upstream cortex core) to frame / frame per workspace crate-name map
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)
//! Loading spinner component.

use frame::style::CYAN_PRIMARY;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

/// Spinner animation styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpinnerStyle {
    /// Breathing pattern used in streaming (default in Cortex TUI)
    #[default]
    Breathing,
    /// Half-circle rotation for tool execution
    HalfCircle,
    /// Simple line rotation
    Line,
    /// Block pattern
    Blocks,
}

impl SpinnerStyle {
    /// Get the frames for this spinner style.
    pub fn frames(&self) -> &[&'static str] {
        match self {
            // Breathing pattern: · → ✢ → ✻ → ✽ (ping-pong for fluid effect)
            SpinnerStyle::Breathing => &["·", "✢", "✻", "✽", "✻", "✢"],
            // Half-circle rotation for tools
            SpinnerStyle::HalfCircle => &["◐", "◑", "◒", "◓"],
            SpinnerStyle::Line => &["|", "/", "-", "\\"],
            SpinnerStyle::Blocks => &["▖", "▘", "▝", "▗"],
        }
    }
}

/// A loading spinner widget.
pub struct LoadingSpinner {
    /// Current frame index
    pub frame: usize,
    /// Spinner style
    pub style: SpinnerStyle,
    /// Optional label text
    pub label: Option<String>,
}

impl LoadingSpinner {
    /// Create a new spinner.
    pub fn new() -> Self {
        Self {
            frame: 0,
            style: SpinnerStyle::default(),
            label: None,
        }
    }

    /// Set the spinner style.
    pub fn with_style(mut self, style: SpinnerStyle) -> Self {
        self.style = style;
        self
    }

    /// Set a label.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Advance to the next frame.
    pub fn tick(&mut self) {
        let frames = self.style.frames();
        self.frame = (self.frame + 1) % frames.len();
    }

    /// Get the current frame character.
    pub fn current_frame(&self) -> &'static str {
        let frames = self.style.frames();
        frames[self.frame % frames.len()]
    }
}

impl Default for LoadingSpinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for &LoadingSpinner {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 2 {
            return;
        }

        let spinner_style = Style::default().fg(CYAN_PRIMARY);
        buf.set_string(area.x, area.y, self.current_frame(), spinner_style);

        if let Some(label) = &self.label
            && area.width > 3
        {
            buf.set_string(area.x + 2, area.y, label, Style::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_tick() {
        let mut spinner = LoadingSpinner::new();
        let first_frame = spinner.current_frame();

        spinner.tick();
        let second_frame = spinner.current_frame();

        assert_ne!(first_frame, second_frame);
    }

    #[test]
    fn test_spinner_styles() {
        assert!(!SpinnerStyle::Breathing.frames().is_empty());
        assert!(!SpinnerStyle::HalfCircle.frames().is_empty());
        assert!(!SpinnerStyle::Line.frames().is_empty());
        assert!(!SpinnerStyle::Blocks.frames().is_empty());
    }

    #[test]
    fn no_spinner_style_uses_braille() {
        for style in [
            SpinnerStyle::Breathing,
            SpinnerStyle::HalfCircle,
            SpinnerStyle::Line,
            SpinnerStyle::Blocks,
        ] {
            for frame in style.frames() {
                assert!(
                    !frame
                        .chars()
                        .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
                    "braille glyph in spinner frames: {frame:?}"
                );
            }
        }
    }
}
