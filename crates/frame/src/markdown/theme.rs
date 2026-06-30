// SPDX-License-Identifier: MIT
// Provenance: direct-port (clean-room exception: direct-port-allowed)
// Upstream: CortexLM/cortex-code@7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Repo: https://github.com/CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/markdown/theme.rs
// Target-Path: crates/frame/src/markdown/theme.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame (cortex_frame -> frame import paths); deps re-pointed to this workspace.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Markdown Theme System for Cortex TUI
//!
//! Provides a comprehensive theming system for markdown rendering with
//! customizable styles for all markdown elements. Integrates seamlessly
//! with the cortex-frame color palette.
//!
//! # Example
//!
//! ```rust,ignore
//! use frame::markdown::MarkdownTheme;
//!
//! // Use default theme
//! let theme = MarkdownTheme::default();
//!
//! // Or customize with builder pattern
//! let custom_theme = MarkdownTheme::new()
//!     .with_h1(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
//!     .with_bold(Style::default().fg(Color::Yellow));
//! ```

use ratatui::style::{Color, Modifier, Style};

use crate::style::{
    AMBER, BORDER_BRAND, BRIGHT, CODEBG, DIM, FAINT, INLBG, OKG, ORANGE, ORANGE_D, PANEL2, TXT,
};

/// Comprehensive theme configuration for markdown rendering.
///
/// Contains styles for all markdown elements including headers, text formatting,
/// code blocks, blockquotes, lists, tables, links, and more.
#[derive(Debug, Clone)]
pub struct MarkdownTheme {
    // ============================================================
    // Headers (H1-H6)
    // ============================================================
    /// Style for H1 headers (largest)
    pub h1: Style,
    /// Style for H2 headers
    pub h2: Style,
    /// Style for H3 headers
    pub h3: Style,
    /// Style for H4 headers
    pub h4: Style,
    /// Style for H5 headers
    pub h5: Style,
    /// Style for H6 headers (smallest)
    pub h6: Style,

    // ============================================================
    // Text Styles
    // ============================================================
    /// Style for bold text
    pub bold: Style,
    /// Style for italic text
    pub italic: Style,
    /// Style for strikethrough text
    pub strikethrough: Style,
    /// Style for inline code spans
    pub code_inline: Style,

    // ============================================================
    // Code Blocks
    // ============================================================
    /// Background color for code blocks
    pub code_block_bg: Color,
    /// Border color for code blocks
    pub code_block_border: Color,
    /// Style for code block text (when no syntax highlighting)
    pub code_block_text: Style,
    /// Style for the language tag in code blocks
    pub code_lang_tag: Style,

    // ============================================================
    // Blockquotes
    // ============================================================
    /// Border color for blockquote left border
    pub blockquote_border: Color,
    /// Style for blockquote text
    pub blockquote_text: Style,

    // ============================================================
    // Lists
    // ============================================================
    /// Style for unordered list bullets
    pub list_bullet: Style,
    /// Style for ordered list numbers
    pub list_number: Style,
    /// Style for checked task list items
    pub task_checked: Style,
    /// Style for unchecked task list items
    pub task_unchecked: Style,

    // ============================================================
    // Tables
    // ============================================================
    /// Border color for table borders
    pub table_border: Color,
    /// Background color for table headers
    pub table_header_bg: Color,
    /// Style for table header text
    pub table_header_text: Style,
    /// Style for table cell text
    pub table_cell_text: Style,

    // ============================================================
    // Links
    // ============================================================
    /// Style for link text
    pub link_text: Style,
    /// Style for link URLs
    pub link_url: Style,

    // ============================================================
    // Other Elements
    // ============================================================
    /// Style for horizontal rules
    pub hr: Style,
    /// Style for normal/default text
    pub text: Style,

    // ============================================================
    // Syntax binding
    // ============================================================
    /// The chrome theme name this markdown theme is bound to. Code blocks resolve their
    /// tree-sitter syntax palette from this (via `tui_syntax::Theme::from_chrome_name`) so the
    /// syntax colors switch together with the chrome theme.
    pub syntax_theme: String,
}

impl MarkdownTheme {
    /// Creates a new MarkdownTheme with default values.
    ///
    /// The default theme uses the cortex-frame color palette for a
    /// cohesive visual experience.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // ============================================================
    // Header Builder Methods
    // ============================================================

    /// Sets the H1 header style.
    #[must_use]
    pub fn with_h1(mut self, style: Style) -> Self {
        self.h1 = style;
        self
    }

    /// Sets the H2 header style.
    #[must_use]
    pub fn with_h2(mut self, style: Style) -> Self {
        self.h2 = style;
        self
    }

    /// Sets the H3 header style.
    #[must_use]
    pub fn with_h3(mut self, style: Style) -> Self {
        self.h3 = style;
        self
    }

    /// Sets the H4 header style.
    #[must_use]
    pub fn with_h4(mut self, style: Style) -> Self {
        self.h4 = style;
        self
    }

    /// Sets the H5 header style.
    #[must_use]
    pub fn with_h5(mut self, style: Style) -> Self {
        self.h5 = style;
        self
    }

    /// Sets the H6 header style.
    #[must_use]
    pub fn with_h6(mut self, style: Style) -> Self {
        self.h6 = style;
        self
    }

    // ============================================================
    // Text Style Builder Methods
    // ============================================================

    /// Sets the bold text style.
    #[must_use]
    pub fn with_bold(mut self, style: Style) -> Self {
        self.bold = style;
        self
    }

    /// Sets the italic text style.
    #[must_use]
    pub fn with_italic(mut self, style: Style) -> Self {
        self.italic = style;
        self
    }

    /// Sets the strikethrough text style.
    #[must_use]
    pub fn with_strikethrough(mut self, style: Style) -> Self {
        self.strikethrough = style;
        self
    }

    /// Sets the inline code style.
    #[must_use]
    pub fn with_code_inline(mut self, style: Style) -> Self {
        self.code_inline = style;
        self
    }

    // ============================================================
    // Code Block Builder Methods
    // ============================================================

    /// Sets the code block background color.
    #[must_use]
    pub fn with_code_block_bg(mut self, color: Color) -> Self {
        self.code_block_bg = color;
        self
    }

    /// Sets the code block border color.
    #[must_use]
    pub fn with_code_block_border(mut self, color: Color) -> Self {
        self.code_block_border = color;
        self
    }

    /// Sets the code block text style.
    #[must_use]
    pub fn with_code_block_text(mut self, style: Style) -> Self {
        self.code_block_text = style;
        self
    }

    /// Sets the code language tag style.
    #[must_use]
    pub fn with_code_lang_tag(mut self, style: Style) -> Self {
        self.code_lang_tag = style;
        self
    }

    // ============================================================
    // Blockquote Builder Methods
    // ============================================================

    /// Sets the blockquote border color.
    #[must_use]
    pub fn with_blockquote_border(mut self, color: Color) -> Self {
        self.blockquote_border = color;
        self
    }

    /// Sets the blockquote text style.
    #[must_use]
    pub fn with_blockquote_text(mut self, style: Style) -> Self {
        self.blockquote_text = style;
        self
    }

    // ============================================================
    // List Builder Methods
    // ============================================================

    /// Sets the list bullet style.
    #[must_use]
    pub fn with_list_bullet(mut self, style: Style) -> Self {
        self.list_bullet = style;
        self
    }

    /// Sets the list number style.
    #[must_use]
    pub fn with_list_number(mut self, style: Style) -> Self {
        self.list_number = style;
        self
    }

    /// Sets the checked task style.
    #[must_use]
    pub fn with_task_checked(mut self, style: Style) -> Self {
        self.task_checked = style;
        self
    }

    /// Sets the unchecked task style.
    #[must_use]
    pub fn with_task_unchecked(mut self, style: Style) -> Self {
        self.task_unchecked = style;
        self
    }

    // ============================================================
    // Table Builder Methods
    // ============================================================

    /// Sets the table border color.
    #[must_use]
    pub fn with_table_border(mut self, color: Color) -> Self {
        self.table_border = color;
        self
    }

    /// Sets the table header background color.
    #[must_use]
    pub fn with_table_header_bg(mut self, color: Color) -> Self {
        self.table_header_bg = color;
        self
    }

    /// Sets the table header text style.
    #[must_use]
    pub fn with_table_header_text(mut self, style: Style) -> Self {
        self.table_header_text = style;
        self
    }

    /// Sets the table cell text style.
    #[must_use]
    pub fn with_table_cell_text(mut self, style: Style) -> Self {
        self.table_cell_text = style;
        self
    }

    // ============================================================
    // Link Builder Methods
    // ============================================================

    /// Sets the link text style.
    #[must_use]
    pub fn with_link_text(mut self, style: Style) -> Self {
        self.link_text = style;
        self
    }

    /// Sets the link URL style.
    #[must_use]
    pub fn with_link_url(mut self, style: Style) -> Self {
        self.link_url = style;
        self
    }

    // ============================================================
    // Other Builder Methods
    // ============================================================

    /// Sets the horizontal rule style.
    #[must_use]
    pub fn with_hr(mut self, style: Style) -> Self {
        self.hr = style;
        self
    }

    /// Sets the default text style.
    #[must_use]
    pub fn with_text(mut self, style: Style) -> Self {
        self.text = style;
        self
    }

    // ============================================================
    // Utility Methods
    // ============================================================

    /// Returns the appropriate header style for the given level (1-6).
    ///
    /// Levels outside the valid range are clamped.
    #[must_use]
    pub fn header_style(&self, level: u8) -> Style {
        match level {
            1 => self.h1,
            2 => self.h2,
            3 => self.h3,
            4 => self.h4,
            5 => self.h5,
            6 => self.h6,
            _ if level < 1 => self.h1,
            _ => self.h6,
        }
    }
}

impl MarkdownTheme {
    /// Create a markdown theme for the "dark" theme (same as default)
    pub fn dark() -> Self {
        Self::default()
    }

    /// Create a markdown theme for the "light" theme
    pub fn light() -> Self {
        Self {
            h1: Style::default()
                .fg(Color::Rgb(0, 100, 70))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default()
                .fg(Color::Rgb(0, 100, 70))
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(Color::Rgb(0, 80, 60))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default()
                .fg(Color::Rgb(0, 80, 60))
                .add_modifier(Modifier::ITALIC),
            h5: Style::default()
                .fg(Color::Rgb(80, 80, 80))
                .add_modifier(Modifier::ITALIC),
            h6: Style::default()
                .fg(Color::Rgb(120, 120, 120))
                .add_modifier(Modifier::ITALIC),
            bold: Style::default()
                .fg(Color::Rgb(0, 100, 70))
                .add_modifier(Modifier::BOLD),
            italic: Style::default()
                .fg(Color::Rgb(30, 30, 30))
                .add_modifier(Modifier::ITALIC),
            strikethrough: Style::default()
                .fg(Color::Rgb(100, 100, 100))
                .add_modifier(Modifier::CROSSED_OUT),
            code_inline: Style::default()
                .fg(Color::Rgb(0, 80, 60))
                .bg(Color::Rgb(235, 235, 235)),
            code_block_bg: Color::Rgb(245, 245, 245),
            code_block_border: Color::Rgb(200, 200, 200),
            code_block_text: Style::default().fg(Color::Rgb(30, 30, 30)),
            code_lang_tag: Style::default()
                .fg(Color::Rgb(50, 100, 200))
                .add_modifier(Modifier::ITALIC),
            blockquote_border: Color::Rgb(0, 100, 70),
            blockquote_text: Style::default()
                .fg(Color::Rgb(80, 80, 80))
                .add_modifier(Modifier::ITALIC),
            list_bullet: Style::default().fg(Color::Rgb(0, 100, 70)),
            list_number: Style::default().fg(Color::Rgb(0, 100, 70)),
            task_checked: Style::default().fg(Color::Rgb(0, 150, 0)),
            task_unchecked: Style::default().fg(Color::Rgb(120, 120, 120)),
            table_border: Color::Rgb(0, 100, 70),
            table_header_bg: Color::Rgb(235, 235, 235),
            table_header_text: Style::default()
                .fg(Color::Rgb(30, 30, 30))
                .add_modifier(Modifier::BOLD),
            table_cell_text: Style::default().fg(Color::Rgb(30, 30, 30)),
            link_text: Style::default()
                .fg(Color::Rgb(50, 100, 200))
                .add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(Color::Rgb(120, 120, 120)),
            hr: Style::default().fg(Color::Rgb(200, 200, 200)),
            text: Style::default().fg(Color::Rgb(30, 30, 30)),
            syntax_theme: "light".to_string(),
        }
    }

    /// Create a markdown theme for the "ocean_dark" theme
    pub fn ocean_dark() -> Self {
        Self {
            h1: Style::default()
                .fg(Color::Rgb(0, 200, 200))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default()
                .fg(Color::Rgb(0, 200, 200))
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(Color::Rgb(100, 200, 220))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default()
                .fg(Color::Rgb(100, 200, 220))
                .add_modifier(Modifier::ITALIC),
            h5: Style::default()
                .fg(Color::Rgb(140, 170, 200))
                .add_modifier(Modifier::ITALIC),
            h6: Style::default()
                .fg(Color::Rgb(80, 110, 140))
                .add_modifier(Modifier::ITALIC),
            bold: Style::default()
                .fg(Color::Rgb(0, 200, 200))
                .add_modifier(Modifier::BOLD),
            italic: Style::default()
                .fg(Color::Rgb(230, 240, 250))
                .add_modifier(Modifier::ITALIC),
            strikethrough: Style::default()
                .fg(Color::Rgb(140, 170, 200))
                .add_modifier(Modifier::CROSSED_OUT),
            code_inline: Style::default()
                .fg(Color::Rgb(100, 180, 255))
                .bg(Color::Rgb(25, 50, 80)),
            code_block_bg: Color::Rgb(15, 35, 60),
            code_block_border: Color::Rgb(40, 80, 120),
            code_block_text: Style::default().fg(Color::Rgb(230, 240, 250)),
            code_lang_tag: Style::default()
                .fg(Color::Rgb(100, 180, 255))
                .add_modifier(Modifier::ITALIC),
            blockquote_border: Color::Rgb(0, 180, 180),
            blockquote_text: Style::default()
                .fg(Color::Rgb(140, 170, 200))
                .add_modifier(Modifier::ITALIC),
            list_bullet: Style::default().fg(Color::Rgb(0, 200, 200)),
            list_number: Style::default().fg(Color::Rgb(0, 200, 200)),
            task_checked: Style::default().fg(Color::Rgb(0, 220, 180)),
            task_unchecked: Style::default().fg(Color::Rgb(80, 110, 140)),
            table_border: Color::Rgb(0, 200, 200),
            table_header_bg: Color::Rgb(25, 50, 80),
            table_header_text: Style::default()
                .fg(Color::Rgb(230, 240, 250))
                .add_modifier(Modifier::BOLD),
            table_cell_text: Style::default().fg(Color::Rgb(230, 240, 250)),
            link_text: Style::default()
                .fg(Color::Rgb(100, 180, 255))
                .add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(Color::Rgb(80, 110, 140)),
            hr: Style::default().fg(Color::Rgb(40, 80, 120)),
            text: Style::default().fg(Color::Rgb(230, 240, 250)),
            syntax_theme: "ocean".to_string(),
        }
    }

    /// Create a markdown theme for the "monokai" theme
    pub fn monokai() -> Self {
        Self {
            h1: Style::default()
                .fg(Color::Rgb(166, 226, 46)) // Monokai green
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default()
                .fg(Color::Rgb(166, 226, 46))
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(Color::Rgb(102, 217, 239)) // Monokai cyan
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default()
                .fg(Color::Rgb(102, 217, 239))
                .add_modifier(Modifier::ITALIC),
            h5: Style::default()
                .fg(Color::Rgb(180, 180, 170))
                .add_modifier(Modifier::ITALIC),
            h6: Style::default()
                .fg(Color::Rgb(117, 113, 94)) // Monokai comment
                .add_modifier(Modifier::ITALIC),
            bold: Style::default()
                .fg(Color::Rgb(249, 38, 114)) // Monokai pink
                .add_modifier(Modifier::BOLD),
            italic: Style::default()
                .fg(Color::Rgb(248, 248, 242)) // Monokai white
                .add_modifier(Modifier::ITALIC),
            strikethrough: Style::default()
                .fg(Color::Rgb(117, 113, 94))
                .add_modifier(Modifier::CROSSED_OUT),
            code_inline: Style::default()
                .fg(Color::Rgb(230, 219, 116)) // Monokai yellow
                .bg(Color::Rgb(55, 56, 50)),
            code_block_bg: Color::Rgb(45, 46, 40),
            code_block_border: Color::Rgb(70, 71, 65),
            code_block_text: Style::default().fg(Color::Rgb(248, 248, 242)),
            code_lang_tag: Style::default()
                .fg(Color::Rgb(102, 217, 239))
                .add_modifier(Modifier::ITALIC),
            blockquote_border: Color::Rgb(166, 226, 46),
            blockquote_text: Style::default()
                .fg(Color::Rgb(117, 113, 94))
                .add_modifier(Modifier::ITALIC),
            list_bullet: Style::default().fg(Color::Rgb(249, 38, 114)),
            list_number: Style::default().fg(Color::Rgb(249, 38, 114)),
            task_checked: Style::default().fg(Color::Rgb(166, 226, 46)),
            task_unchecked: Style::default().fg(Color::Rgb(117, 113, 94)),
            table_border: Color::Rgb(166, 226, 46),
            table_header_bg: Color::Rgb(55, 56, 50),
            table_header_text: Style::default()
                .fg(Color::Rgb(248, 248, 242))
                .add_modifier(Modifier::BOLD),
            table_cell_text: Style::default().fg(Color::Rgb(248, 248, 242)),
            link_text: Style::default()
                .fg(Color::Rgb(102, 217, 239))
                .add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(Color::Rgb(117, 113, 94)),
            hr: Style::default().fg(Color::Rgb(70, 71, 65)),
            text: Style::default().fg(Color::Rgb(248, 248, 242)),
            syntax_theme: "monokai".to_string(),
        }
    }

    /// Create a markdown theme for the "vscode" (VS Code Dark+) chrome theme.
    pub fn vscode() -> Self {
        Self {
            h1: Style::default()
                .fg(Color::Rgb(86, 156, 214))
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default()
                .fg(Color::Rgb(86, 156, 214))
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(Color::Rgb(78, 201, 176))
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default()
                .fg(Color::Rgb(78, 201, 176))
                .add_modifier(Modifier::ITALIC),
            h5: Style::default()
                .fg(Color::Rgb(150, 150, 150))
                .add_modifier(Modifier::ITALIC),
            h6: Style::default()
                .fg(Color::Rgb(110, 110, 110))
                .add_modifier(Modifier::ITALIC),
            bold: Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
            italic: Style::default()
                .fg(Color::Rgb(212, 212, 212))
                .add_modifier(Modifier::ITALIC),
            strikethrough: Style::default()
                .fg(Color::Rgb(150, 150, 150))
                .add_modifier(Modifier::CROSSED_OUT),
            code_inline: Style::default()
                .fg(Color::Rgb(206, 145, 120))
                .bg(Color::Rgb(45, 45, 46)),
            code_block_bg: Color::Rgb(30, 30, 30),
            code_block_border: Color::Rgb(62, 62, 66),
            code_block_text: Style::default().fg(Color::Rgb(212, 212, 212)),
            code_lang_tag: Style::default()
                .fg(Color::Rgb(86, 156, 214))
                .add_modifier(Modifier::ITALIC),
            blockquote_border: Color::Rgb(0, 122, 204),
            blockquote_text: Style::default()
                .fg(Color::Rgb(150, 150, 150))
                .add_modifier(Modifier::ITALIC),
            list_bullet: Style::default().fg(Color::Rgb(0, 122, 204)),
            list_number: Style::default().fg(Color::Rgb(0, 122, 204)),
            task_checked: Style::default().fg(Color::Rgb(137, 209, 133)),
            task_unchecked: Style::default().fg(Color::Rgb(110, 110, 110)),
            table_border: Color::Rgb(0, 122, 204),
            table_header_bg: Color::Rgb(45, 45, 46),
            table_header_text: Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD),
            table_cell_text: Style::default().fg(Color::Rgb(212, 212, 212)),
            link_text: Style::default()
                .fg(Color::Rgb(86, 156, 214))
                .add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(Color::Rgb(110, 110, 110)),
            hr: Style::default().fg(Color::Rgb(62, 62, 66)),
            text: Style::default().fg(Color::Rgb(212, 212, 212)),
            syntax_theme: "vscode".to_string(),
        }
    }

    /// Create a markdown theme from a theme name
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            "ocean" | "ocean_dark" => Self::ocean_dark(),
            "monokai" => Self::monokai(),
            "vscode" | "vscode_dark" | "vscode-dark" => Self::vscode(),
            _ => Self::dark(),
        }
    }
}

impl Default for MarkdownTheme {
    /// The default markdown theme is the mono-orange brand `dark` theme.
    fn default() -> Self {
        Self {
            // Headers - decreasing prominence from H1 to H6
            h1: Style::default()
                .fg(ORANGE)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(AMBER)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default().fg(AMBER).add_modifier(Modifier::ITALIC),
            h5: Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
            h6: Style::default().fg(FAINT).add_modifier(Modifier::ITALIC),

            // Text styles
            bold: Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            italic: Style::default().fg(TXT).add_modifier(Modifier::ITALIC),
            strikethrough: Style::default()
                .fg(DIM)
                .add_modifier(Modifier::CROSSED_OUT),
            code_inline: Style::default().fg(AMBER).bg(INLBG),

            // Code blocks
            code_block_bg: CODEBG,
            code_block_border: BORDER_BRAND,
            code_block_text: Style::default().fg(TXT),
            code_lang_tag: Style::default().fg(FAINT).add_modifier(Modifier::ITALIC),

            // Blockquotes
            blockquote_border: ORANGE_D,
            blockquote_text: Style::default().fg(DIM).add_modifier(Modifier::ITALIC),

            // Lists
            list_bullet: Style::default().fg(ORANGE_D),
            list_number: Style::default().fg(ORANGE_D),
            task_checked: Style::default().fg(OKG),
            task_unchecked: Style::default().fg(FAINT),

            // Tables - use the brand accent for borders
            table_border: ORANGE_D,
            table_header_bg: PANEL2,
            table_header_text: Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            table_cell_text: Style::default().fg(TXT),

            // Links
            link_text: Style::default().fg(AMBER).add_modifier(Modifier::UNDERLINED),
            link_url: Style::default().fg(FAINT),

            // Other elements
            hr: Style::default().fg(BORDER_BRAND),
            text: Style::default().fg(TXT),

            // Syntax binding: the default theme is the brand `dark` chrome.
            syntax_theme: "dark".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_theme_creation() {
        let theme = MarkdownTheme::default();

        // Verify header styles use the mono-orange brand palette.
        assert_eq!(theme.h1.fg, Some(ORANGE));
        assert_eq!(theme.h2.fg, Some(ORANGE));
        assert_eq!(theme.h3.fg, Some(AMBER));
        assert_eq!(theme.h4.fg, Some(AMBER));
        assert_eq!(theme.h5.fg, Some(DIM));
        assert_eq!(theme.h6.fg, Some(FAINT));
    }

    #[test]
    fn test_new_equals_default() {
        let theme_new = MarkdownTheme::new();
        let theme_default = MarkdownTheme::default();

        // Verify new() returns the same as default()
        assert_eq!(theme_new.h1.fg, theme_default.h1.fg);
        assert_eq!(theme_new.bold.fg, theme_default.bold.fg);
        assert_eq!(theme_new.code_block_bg, theme_default.code_block_bg);
    }

    #[test]
    fn test_header_modifiers() {
        let theme = MarkdownTheme::default();

        // H1 should have BOLD and UNDERLINED
        assert!(theme.h1.add_modifier.contains(Modifier::BOLD));
        assert!(theme.h1.add_modifier.contains(Modifier::UNDERLINED));

        // H2 should have BOLD only
        assert!(theme.h2.add_modifier.contains(Modifier::BOLD));
        assert!(!theme.h2.add_modifier.contains(Modifier::UNDERLINED));

        // H3 should have BOLD and ITALIC
        assert!(theme.h3.add_modifier.contains(Modifier::BOLD));
        assert!(theme.h3.add_modifier.contains(Modifier::ITALIC));

        // H4, H5, H6 should have ITALIC
        assert!(theme.h4.add_modifier.contains(Modifier::ITALIC));
        assert!(theme.h5.add_modifier.contains(Modifier::ITALIC));
        assert!(theme.h6.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_text_style_modifiers() {
        let theme = MarkdownTheme::default();

        assert!(theme.bold.add_modifier.contains(Modifier::BOLD));
        assert!(theme.italic.add_modifier.contains(Modifier::ITALIC));
        assert!(
            theme
                .strikethrough
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
    }

    #[test]
    fn test_code_inline_has_background() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.code_inline.fg, Some(AMBER));
        assert_eq!(theme.code_inline.bg, Some(INLBG));
    }

    #[test]
    fn test_code_block_colors() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.code_block_bg, CODEBG);
        assert_eq!(theme.code_block_border, BORDER_BRAND);
        assert_eq!(theme.code_block_text.fg, Some(TXT));
        assert!(theme.code_lang_tag.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_blockquote_styles() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.blockquote_border, ORANGE_D);
        assert_eq!(theme.blockquote_text.fg, Some(DIM));
        assert!(
            theme
                .blockquote_text
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn test_list_styles() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.list_bullet.fg, Some(ORANGE_D));
        assert_eq!(theme.list_number.fg, Some(ORANGE_D));
        assert_eq!(theme.task_checked.fg, Some(OKG));
        assert_eq!(theme.task_unchecked.fg, Some(FAINT));
    }

    #[test]
    fn test_table_styles() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.table_border, ORANGE_D); // Uses the brand accent for consistency
        assert_eq!(theme.table_header_bg, PANEL2);
        assert_eq!(theme.table_header_text.fg, Some(BRIGHT));
        assert!(
            theme
                .table_header_text
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(theme.table_cell_text.fg, Some(TXT));
    }

    #[test]
    fn test_link_styles() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.link_text.fg, Some(AMBER));
        assert!(theme.link_text.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(theme.link_url.fg, Some(FAINT));
    }

    #[test]
    fn test_other_styles() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.hr.fg, Some(BORDER_BRAND));
        assert_eq!(theme.text.fg, Some(TXT));
    }

    #[test]
    fn test_builder_with_h1() {
        let custom_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let theme = MarkdownTheme::new().with_h1(custom_style);

        assert_eq!(theme.h1.fg, Some(Color::Red));
        assert!(theme.h1.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_builder_with_h2() {
        let custom_style = Style::default().fg(Color::Green);
        let theme = MarkdownTheme::new().with_h2(custom_style);

        assert_eq!(theme.h2.fg, Some(Color::Green));
    }

    #[test]
    fn test_builder_with_h3() {
        let custom_style = Style::default().fg(Color::Blue);
        let theme = MarkdownTheme::new().with_h3(custom_style);

        assert_eq!(theme.h3.fg, Some(Color::Blue));
    }

    #[test]
    fn test_builder_with_h4() {
        let custom_style = Style::default().fg(Color::Yellow);
        let theme = MarkdownTheme::new().with_h4(custom_style);

        assert_eq!(theme.h4.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_builder_with_h5() {
        let custom_style = Style::default().fg(Color::Magenta);
        let theme = MarkdownTheme::new().with_h5(custom_style);

        assert_eq!(theme.h5.fg, Some(Color::Magenta));
    }

    #[test]
    fn test_builder_with_h6() {
        let custom_style = Style::default().fg(Color::Cyan);
        let theme = MarkdownTheme::new().with_h6(custom_style);

        assert_eq!(theme.h6.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_builder_with_bold() {
        let custom_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let theme = MarkdownTheme::new().with_bold(custom_style);

        assert_eq!(theme.bold.fg, Some(Color::White));
    }

    #[test]
    fn test_builder_with_italic() {
        let custom_style = Style::default().fg(Color::LightBlue);
        let theme = MarkdownTheme::new().with_italic(custom_style);

        assert_eq!(theme.italic.fg, Some(Color::LightBlue));
    }

    #[test]
    fn test_builder_with_strikethrough() {
        let custom_style = Style::default().fg(Color::DarkGray);
        let theme = MarkdownTheme::new().with_strikethrough(custom_style);

        assert_eq!(theme.strikethrough.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_builder_with_code_inline() {
        let custom_style = Style::default().fg(Color::LightGreen).bg(Color::Black);
        let theme = MarkdownTheme::new().with_code_inline(custom_style);

        assert_eq!(theme.code_inline.fg, Some(Color::LightGreen));
        assert_eq!(theme.code_inline.bg, Some(Color::Black));
    }

    #[test]
    fn test_builder_with_code_block_bg() {
        let theme = MarkdownTheme::new().with_code_block_bg(Color::Black);

        assert_eq!(theme.code_block_bg, Color::Black);
    }

    #[test]
    fn test_builder_with_code_block_border() {
        let theme = MarkdownTheme::new().with_code_block_border(Color::White);

        assert_eq!(theme.code_block_border, Color::White);
    }

    #[test]
    fn test_builder_with_code_block_text() {
        let custom_style = Style::default().fg(Color::LightYellow);
        let theme = MarkdownTheme::new().with_code_block_text(custom_style);

        assert_eq!(theme.code_block_text.fg, Some(Color::LightYellow));
    }

    #[test]
    fn test_builder_with_code_lang_tag() {
        let custom_style = Style::default().fg(Color::LightCyan);
        let theme = MarkdownTheme::new().with_code_lang_tag(custom_style);

        assert_eq!(theme.code_lang_tag.fg, Some(Color::LightCyan));
    }

    #[test]
    fn test_builder_with_blockquote_border() {
        let theme = MarkdownTheme::new().with_blockquote_border(Color::Magenta);

        assert_eq!(theme.blockquote_border, Color::Magenta);
    }

    #[test]
    fn test_builder_with_blockquote_text() {
        let custom_style = Style::default().fg(Color::Gray);
        let theme = MarkdownTheme::new().with_blockquote_text(custom_style);

        assert_eq!(theme.blockquote_text.fg, Some(Color::Gray));
    }

    #[test]
    fn test_builder_with_list_bullet() {
        let custom_style = Style::default().fg(Color::LightRed);
        let theme = MarkdownTheme::new().with_list_bullet(custom_style);

        assert_eq!(theme.list_bullet.fg, Some(Color::LightRed));
    }

    #[test]
    fn test_builder_with_list_number() {
        let custom_style = Style::default().fg(Color::LightMagenta);
        let theme = MarkdownTheme::new().with_list_number(custom_style);

        assert_eq!(theme.list_number.fg, Some(Color::LightMagenta));
    }

    #[test]
    fn test_builder_with_task_checked() {
        let custom_style = Style::default().fg(Color::Green);
        let theme = MarkdownTheme::new().with_task_checked(custom_style);

        assert_eq!(theme.task_checked.fg, Some(Color::Green));
    }

    #[test]
    fn test_builder_with_task_unchecked() {
        let custom_style = Style::default().fg(Color::Red);
        let theme = MarkdownTheme::new().with_task_unchecked(custom_style);

        assert_eq!(theme.task_unchecked.fg, Some(Color::Red));
    }

    #[test]
    fn test_builder_with_table_border() {
        let theme = MarkdownTheme::new().with_table_border(Color::Blue);

        assert_eq!(theme.table_border, Color::Blue);
    }

    #[test]
    fn test_builder_with_table_header_bg() {
        let theme = MarkdownTheme::new().with_table_header_bg(Color::DarkGray);

        assert_eq!(theme.table_header_bg, Color::DarkGray);
    }

    #[test]
    fn test_builder_with_table_header_text() {
        let custom_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let theme = MarkdownTheme::new().with_table_header_text(custom_style);

        assert_eq!(theme.table_header_text.fg, Some(Color::White));
    }

    #[test]
    fn test_builder_with_table_cell_text() {
        let custom_style = Style::default().fg(Color::LightGreen);
        let theme = MarkdownTheme::new().with_table_cell_text(custom_style);

        assert_eq!(theme.table_cell_text.fg, Some(Color::LightGreen));
    }

    #[test]
    fn test_builder_with_link_text() {
        let custom_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED);
        let theme = MarkdownTheme::new().with_link_text(custom_style);

        assert_eq!(theme.link_text.fg, Some(Color::Cyan));
    }

    #[test]
    fn test_builder_with_link_url() {
        let custom_style = Style::default().fg(Color::DarkGray);
        let theme = MarkdownTheme::new().with_link_url(custom_style);

        assert_eq!(theme.link_url.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_builder_with_hr() {
        let custom_style = Style::default().fg(Color::Gray);
        let theme = MarkdownTheme::new().with_hr(custom_style);

        assert_eq!(theme.hr.fg, Some(Color::Gray));
    }

    #[test]
    fn test_builder_with_text() {
        let custom_style = Style::default().fg(Color::White);
        let theme = MarkdownTheme::new().with_text(custom_style);

        assert_eq!(theme.text.fg, Some(Color::White));
    }

    #[test]
    fn test_builder_chaining() {
        let theme = MarkdownTheme::new()
            .with_h1(Style::default().fg(Color::Red))
            .with_h2(Style::default().fg(Color::Green))
            .with_bold(Style::default().fg(Color::Yellow))
            .with_code_block_bg(Color::Black)
            .with_table_border(Color::White);

        assert_eq!(theme.h1.fg, Some(Color::Red));
        assert_eq!(theme.h2.fg, Some(Color::Green));
        assert_eq!(theme.bold.fg, Some(Color::Yellow));
        assert_eq!(theme.code_block_bg, Color::Black);
        assert_eq!(theme.table_border, Color::White);
    }

    #[test]
    fn test_header_style_utility() {
        let theme = MarkdownTheme::default();

        assert_eq!(theme.header_style(1).fg, theme.h1.fg);
        assert_eq!(theme.header_style(2).fg, theme.h2.fg);
        assert_eq!(theme.header_style(3).fg, theme.h3.fg);
        assert_eq!(theme.header_style(4).fg, theme.h4.fg);
        assert_eq!(theme.header_style(5).fg, theme.h5.fg);
        assert_eq!(theme.header_style(6).fg, theme.h6.fg);
    }

    #[test]
    fn test_header_style_clamping() {
        let theme = MarkdownTheme::default();

        // Levels below 1 should return h1
        assert_eq!(theme.header_style(0).fg, theme.h1.fg);

        // Levels above 6 should return h6
        assert_eq!(theme.header_style(7).fg, theme.h6.fg);
        assert_eq!(theme.header_style(255).fg, theme.h6.fg);
    }

    #[test]
    fn test_theme_clone() {
        let original = MarkdownTheme::new()
            .with_h1(Style::default().fg(Color::Red))
            .with_bold(Style::default().fg(Color::Blue));

        let cloned = original.clone();

        assert_eq!(cloned.h1.fg, Some(Color::Red));
        assert_eq!(cloned.bold.fg, Some(Color::Blue));
    }

    #[test]
    fn test_theme_debug() {
        let theme = MarkdownTheme::default();
        let debug_str = format!("{:?}", theme);

        // Verify debug output contains expected fields
        assert!(debug_str.contains("h1"));
        assert!(debug_str.contains("bold"));
        assert!(debug_str.contains("code_block_bg"));
    }

    #[test]
    fn test_all_default_colors_are_set() {
        let theme = MarkdownTheme::default();

        // Verify all Style fields have foreground colors set
        assert!(theme.h1.fg.is_some());
        assert!(theme.h2.fg.is_some());
        assert!(theme.h3.fg.is_some());
        assert!(theme.h4.fg.is_some());
        assert!(theme.h5.fg.is_some());
        assert!(theme.h6.fg.is_some());
        assert!(theme.bold.fg.is_some());
        assert!(theme.italic.fg.is_some());
        assert!(theme.strikethrough.fg.is_some());
        assert!(theme.code_inline.fg.is_some());
        assert!(theme.code_block_text.fg.is_some());
        assert!(theme.code_lang_tag.fg.is_some());
        assert!(theme.blockquote_text.fg.is_some());
        assert!(theme.list_bullet.fg.is_some());
        assert!(theme.list_number.fg.is_some());
        assert!(theme.task_checked.fg.is_some());
        assert!(theme.task_unchecked.fg.is_some());
        assert!(theme.table_header_text.fg.is_some());
        assert!(theme.table_cell_text.fg.is_some());
        assert!(theme.link_text.fg.is_some());
        assert!(theme.link_url.fg.is_some());
        assert!(theme.hr.fg.is_some());
        assert!(theme.text.fg.is_some());
    }

    #[test]
    fn test_dark_theme() {
        let theme = MarkdownTheme::dark();
        let default_theme = MarkdownTheme::default();

        // dark() should return the same as default()
        assert_eq!(theme.h1.fg, default_theme.h1.fg);
        assert_eq!(theme.bold.fg, default_theme.bold.fg);
        assert_eq!(theme.code_block_bg, default_theme.code_block_bg);
    }

    #[test]
    fn test_light_theme() {
        let theme = MarkdownTheme::light();

        // Light theme has different colors than dark
        assert_eq!(theme.h1.fg, Some(Color::Rgb(0, 100, 70)));
        assert_eq!(theme.text.fg, Some(Color::Rgb(30, 30, 30)));
        assert_eq!(theme.code_block_bg, Color::Rgb(245, 245, 245));
    }

    #[test]
    fn test_ocean_dark_theme() {
        let theme = MarkdownTheme::ocean_dark();

        // Ocean dark theme has cyan accent
        assert_eq!(theme.h1.fg, Some(Color::Rgb(0, 200, 200)));
        assert_eq!(theme.text.fg, Some(Color::Rgb(230, 240, 250)));
        assert_eq!(theme.code_block_bg, Color::Rgb(15, 35, 60));
    }

    #[test]
    fn test_monokai_theme() {
        let theme = MarkdownTheme::monokai();

        // Monokai theme has distinctive green headers
        assert_eq!(theme.h1.fg, Some(Color::Rgb(166, 226, 46)));
        assert_eq!(theme.bold.fg, Some(Color::Rgb(249, 38, 114))); // Monokai pink
        assert_eq!(theme.code_block_bg, Color::Rgb(45, 46, 40));
    }

    #[test]
    fn test_from_name() {
        // Test all valid theme names
        let dark = MarkdownTheme::from_name("dark");
        assert_eq!(dark.h1.fg, MarkdownTheme::dark().h1.fg);

        let light = MarkdownTheme::from_name("light");
        assert_eq!(light.h1.fg, MarkdownTheme::light().h1.fg);

        let ocean = MarkdownTheme::from_name("ocean_dark");
        assert_eq!(ocean.h1.fg, MarkdownTheme::ocean_dark().h1.fg);

        let ocean_short = MarkdownTheme::from_name("ocean");
        assert_eq!(ocean_short.h1.fg, MarkdownTheme::ocean_dark().h1.fg);

        let monokai = MarkdownTheme::from_name("monokai");
        assert_eq!(monokai.h1.fg, MarkdownTheme::monokai().h1.fg);

        let vscode = MarkdownTheme::from_name("vscode");
        assert_eq!(vscode.h1.fg, MarkdownTheme::vscode().h1.fg);
    }

    #[test]
    fn from_name_binds_syntax_theme_to_chrome() {
        // Each chrome theme carries the matching syntax-theme name so code blocks switch palettes
        // together with the chrome theme.
        assert_eq!(MarkdownTheme::from_name("dark").syntax_theme, "dark");
        assert_eq!(MarkdownTheme::from_name("light").syntax_theme, "light");
        assert_eq!(MarkdownTheme::from_name("monokai").syntax_theme, "monokai");
        assert_eq!(MarkdownTheme::from_name("ocean").syntax_theme, "ocean");
        assert_eq!(MarkdownTheme::from_name("vscode").syntax_theme, "vscode");
        // The default theme is the brand dark chrome.
        assert_eq!(MarkdownTheme::default().syntax_theme, "dark");
    }

    #[test]
    fn test_from_name_case_insensitive() {
        // Should be case insensitive
        let dark_upper = MarkdownTheme::from_name("DARK");
        let dark_lower = MarkdownTheme::from_name("dark");
        let dark_mixed = MarkdownTheme::from_name("DaRk");

        assert_eq!(dark_upper.h1.fg, dark_lower.h1.fg);
        assert_eq!(dark_mixed.h1.fg, dark_lower.h1.fg);
    }

    #[test]
    fn test_from_name_fallback() {
        // Unknown names should fall back to dark
        let unknown = MarkdownTheme::from_name("unknown");
        let dark = MarkdownTheme::dark();

        assert_eq!(unknown.h1.fg, dark.h1.fg);
    }

    #[test]
    fn test_all_theme_colors_are_set() {
        // Verify all theme variants have colors set
        for theme in [
            MarkdownTheme::dark(),
            MarkdownTheme::light(),
            MarkdownTheme::ocean_dark(),
            MarkdownTheme::monokai(),
            MarkdownTheme::vscode(),
        ] {
            assert!(theme.h1.fg.is_some());
            assert!(theme.h2.fg.is_some());
            assert!(theme.h3.fg.is_some());
            assert!(theme.h4.fg.is_some());
            assert!(theme.h5.fg.is_some());
            assert!(theme.h6.fg.is_some());
            assert!(theme.bold.fg.is_some());
            assert!(theme.italic.fg.is_some());
            assert!(theme.text.fg.is_some());
        }
    }
}
