// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: cortex-tui-framework/crates/cortex-tui-syntax/src/theme.rs
// Target-Path: crates/tui-syntax/src/theme.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-tui-syntax -> tui-syntax (cortex_tui_text -> tui_text imports).
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Syntax highlighting theme system.
//!
//! Provides a theme definition that maps tree-sitter capture names to visual styles.

#![allow(clippy::unreadable_literal)] // Hex color values represent colors, not numbers
#![allow(clippy::return_self_not_must_use)] // Builder pattern methods don't require must_use

use ahash::AHashMap;
use tui_text::{Color, Style};

/// Helper to create a color from hex value.
#[inline]
fn hex_color(hex: u32) -> Color {
    Color::from_rgb_u8(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

/// A syntax highlighting theme.
///
/// Maps capture group names (e.g., "keyword", "string", "comment") to visual styles.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Named styles for capture groups.
    styles: AHashMap<String, Style>,
    /// Default style for unhighlighted text.
    default_style: Style,
    /// Cached merged styles for compound capture names.
    merge_cache: AHashMap<String, Style>,
}

impl Default for Theme {
    fn default() -> Self {
        Self::vscode_dark()
    }
}

impl Theme {
    /// Creates a new empty theme.
    pub fn new() -> Self {
        Self {
            styles: AHashMap::new(),
            default_style: Style::new(),
            merge_cache: AHashMap::new(),
        }
    }

    /// Creates a theme with a default style.
    pub fn with_default(default_style: Style) -> Self {
        Self {
            styles: AHashMap::new(),
            default_style,
            merge_cache: AHashMap::new(),
        }
    }

    /// Creates a VS Code Dark+ inspired theme.
    ///
    /// This provides sensible defaults similar to VS Code's dark theme.
    #[allow(clippy::too_many_lines)]
    pub fn vscode_dark() -> Self {
        let mut theme = Self::new();

        // Keywords
        theme.set("keyword", Style::new().fg(hex_color(0x569CD6)));
        theme.set("keyword.control", Style::new().fg(hex_color(0xC586C0)));
        theme.set("keyword.control.flow", Style::new().fg(hex_color(0xC586C0)));
        theme.set(
            "keyword.control.return",
            Style::new().fg(hex_color(0xC586C0)),
        );
        theme.set(
            "keyword.control.conditional",
            Style::new().fg(hex_color(0xC586C0)),
        );
        theme.set(
            "keyword.control.repeat",
            Style::new().fg(hex_color(0xC586C0)),
        );
        theme.set("keyword.function", Style::new().fg(hex_color(0x569CD6)));
        theme.set("keyword.operator", Style::new().fg(hex_color(0x569CD6)));
        theme.set("keyword.import", Style::new().fg(hex_color(0xC586C0)));
        theme.set("keyword.type", Style::new().fg(hex_color(0x569CD6)));
        theme.set("keyword.modifier", Style::new().fg(hex_color(0x569CD6)));
        theme.set("keyword.storage", Style::new().fg(hex_color(0x569CD6)));

        // Types
        theme.set("type", Style::new().fg(hex_color(0x4EC9B0)));
        theme.set("type.builtin", Style::new().fg(hex_color(0x4EC9B0)));
        theme.set("type.definition", Style::new().fg(hex_color(0x4EC9B0)));
        theme.set("type.qualifier", Style::new().fg(hex_color(0x569CD6)));

        // Functions
        theme.set("function", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("function.builtin", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("function.call", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("function.macro", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("function.method", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("function.method.call", Style::new().fg(hex_color(0xDCDCAA)));

        // Variables
        theme.set("variable", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("variable.builtin", Style::new().fg(hex_color(0x569CD6)));
        theme.set("variable.parameter", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("variable.member", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set(
            "variable.other.member",
            Style::new().fg(hex_color(0x9CDCFE)),
        );

        // Constants
        theme.set("constant", Style::new().fg(hex_color(0x4FC1FF)));
        theme.set("constant.builtin", Style::new().fg(hex_color(0x569CD6)));
        theme.set("constant.character", Style::new().fg(hex_color(0xCE9178)));
        theme.set("constant.numeric", Style::new().fg(hex_color(0xB5CEA8)));
        theme.set(
            "constant.character.escape",
            Style::new().fg(hex_color(0xD7BA7D)),
        );

        // Strings
        theme.set("string", Style::new().fg(hex_color(0xCE9178)));
        theme.set("string.regexp", Style::new().fg(hex_color(0xD16969)));
        theme.set("string.escape", Style::new().fg(hex_color(0xD7BA7D)));
        theme.set("string.special", Style::new().fg(hex_color(0xD7BA7D)));
        theme.set(
            "string.special.symbol",
            Style::new().fg(hex_color(0xCE9178)),
        );

        // Comments
        theme.set("comment", Style::new().fg(hex_color(0x6A9955)).italic());
        theme.set(
            "comment.line",
            Style::new().fg(hex_color(0x6A9955)).italic(),
        );
        theme.set(
            "comment.block",
            Style::new().fg(hex_color(0x6A9955)).italic(),
        );
        theme.set(
            "comment.documentation",
            Style::new().fg(hex_color(0x608B4E)).italic(),
        );

        // Operators and punctuation
        theme.set("operator", Style::new().fg(hex_color(0xD4D4D4)));
        theme.set("punctuation", Style::new().fg(hex_color(0xD4D4D4)));
        theme.set("punctuation.bracket", Style::new().fg(hex_color(0xD4D4D4)));
        theme.set(
            "punctuation.delimiter",
            Style::new().fg(hex_color(0xD4D4D4)),
        );
        theme.set("punctuation.special", Style::new().fg(hex_color(0x569CD6)));

        // Properties and attributes
        theme.set("property", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("attribute", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("label", Style::new().fg(hex_color(0xDCDCAA)));
        theme.set("namespace", Style::new().fg(hex_color(0x4EC9B0)));
        theme.set("module", Style::new().fg(hex_color(0x4EC9B0)));

        // Tags (HTML/XML)
        theme.set("tag", Style::new().fg(hex_color(0x569CD6)));
        theme.set("tag.attribute", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("tag.delimiter", Style::new().fg(hex_color(0x808080)));

        // Markup (Markdown)
        theme.set(
            "markup.heading",
            Style::new().fg(hex_color(0x569CD6)).bold(),
        );
        theme.set("markup.bold", Style::new().bold());
        theme.set("markup.italic", Style::new().italic());
        theme.set("markup.strikethrough", Style::new().strikethrough());
        theme.set("markup.link", Style::new().fg(hex_color(0x569CD6)));
        theme.set(
            "markup.link.url",
            Style::new().fg(hex_color(0xCE9178)).underline(),
        );
        theme.set("markup.raw", Style::new().fg(hex_color(0xCE9178)));
        theme.set(
            "markup.quote",
            Style::new().fg(hex_color(0x6A9955)).italic(),
        );
        theme.set("markup.list", Style::new().fg(hex_color(0x569CD6)));

        // Special
        theme.set("text", Style::new().fg(hex_color(0xD4D4D4)));
        theme.set("text.literal", Style::new().fg(hex_color(0xCE9178)));
        theme.set("text.reference", Style::new().fg(hex_color(0x9CDCFE)));
        theme.set("text.title", Style::new().fg(hex_color(0x569CD6)));
        theme.set("text.uri", Style::new().fg(hex_color(0xCE9178)));
        theme.set("text.underline", Style::new().underline());

        // Errors and diagnostics
        theme.set("error", Style::new().fg(hex_color(0xF44747)));
        theme.set("warning", Style::new().fg(hex_color(0xCCA700)));
        theme.set("info", Style::new().fg(hex_color(0x3794FF)));
        theme.set("hint", Style::new().fg(hex_color(0x9CDCFE)));

        // Diff
        theme.set("diff.plus", Style::new().fg(hex_color(0x6A9955)));
        theme.set("diff.minus", Style::new().fg(hex_color(0xF44747)));
        theme.set("diff.delta", Style::new().fg(hex_color(0x569CD6)));

        theme
    }

    /// Builds a compact theme from a core set of token colors (keyword, function, type, string,
    /// number, comment, operator, constant, variable). Used to keep the chrome-bound syntax
    /// palettes (`cortex_dark`/`light`/`ocean`) consistent and rich enough that every fenced block
    /// shows multiple distinct colors.
    #[allow(clippy::too_many_arguments)]
    fn from_core(
        kw: u32,
        func: u32,
        typ: u32,
        string: u32,
        num: u32,
        comment: u32,
        op: u32,
        konst: u32,
        var: u32,
    ) -> Self {
        let mut t = Self::with_default(Style::new().fg(hex_color(op)));

        t.set("keyword", Style::new().fg(hex_color(kw)));
        t.set("keyword.control", Style::new().fg(hex_color(kw)));
        t.set("keyword.function", Style::new().fg(hex_color(kw)));
        t.set("keyword.operator", Style::new().fg(hex_color(kw)));
        t.set("keyword.import", Style::new().fg(hex_color(kw)));
        t.set("keyword.return", Style::new().fg(hex_color(kw)));

        t.set("type", Style::new().fg(hex_color(typ)));
        t.set("type.builtin", Style::new().fg(hex_color(typ)));
        t.set("namespace", Style::new().fg(hex_color(typ)));
        t.set("module", Style::new().fg(hex_color(typ)));

        t.set("function", Style::new().fg(hex_color(func)));
        t.set("function.call", Style::new().fg(hex_color(func)));
        t.set("function.method", Style::new().fg(hex_color(func)));
        t.set("function.macro", Style::new().fg(hex_color(func)));

        t.set("variable", Style::new().fg(hex_color(var)));
        t.set("variable.parameter", Style::new().fg(hex_color(var)));
        t.set("property", Style::new().fg(hex_color(var)));
        t.set("attribute", Style::new().fg(hex_color(var)));

        t.set("constant", Style::new().fg(hex_color(konst)));
        t.set("constant.builtin", Style::new().fg(hex_color(konst)));
        t.set("constant.numeric", Style::new().fg(hex_color(num)));
        t.set("number", Style::new().fg(hex_color(num)));

        t.set("string", Style::new().fg(hex_color(string)));
        t.set("string.special", Style::new().fg(hex_color(string)));
        t.set("constant.character", Style::new().fg(hex_color(string)));

        t.set("comment", Style::new().fg(hex_color(comment)).italic());
        t.set("comment.line", Style::new().fg(hex_color(comment)).italic());
        t.set("comment.block", Style::new().fg(hex_color(comment)).italic());

        t.set("operator", Style::new().fg(hex_color(op)));
        t.set("punctuation", Style::new().fg(hex_color(op)));
        t.set("punctuation.bracket", Style::new().fg(hex_color(op)));
        t.set("punctuation.delimiter", Style::new().fg(hex_color(op)));

        t
    }

    /// Cortex brand "dark" syntax palette (orange-tinted keywords on the mono-orange theme).
    /// RGB mirror `library/design-system.md` (Wave 7 `dark` syntax row).
    pub fn cortex_dark() -> Self {
        Self::from_core(
            0xE89256, // keyword  (232,146,86) ORANGE
            0xF2B778, // function (242,183,120) AMBER
            0xF2C896, // type     (242,200,150)
            0xCE9678, // string   (206,150,120)
            0x60CC6E, // number   (96,204,110)
            0x78786E, // comment  (120,120,110)
            0xAAAAAA, // operator (170,170,170)
            0xF2B778, // constant (242,183,120)
            0xD0D0CE, // variable (208,208,206) TXT
        )
    }

    /// Light syntax palette (dark inks on a light canvas). Wave 7 `light` syntax row.
    pub fn light() -> Self {
        Self::from_core(
            0x0000C8, // keyword  (0,0,200)
            0x966400, // function (150,100,0)
            0x008080, // type     (0,128,128)
            0xA02020, // string   (160,32,32)
            0x006496, // number   (0,100,150)
            0x008000, // comment  (0,128,0)
            0x3C3C3C, // operator (60,60,60)
            0x006496, // constant (0,100,150)
            0x1E1E1E, // variable (30,30,30)
        )
    }

    /// Ocean syntax palette (cool blues/teals). Wave 7 `ocean` syntax row.
    pub fn ocean() -> Self {
        Self::from_core(
            0x00C8C8, // keyword  (0,200,200)
            0x64C8DC, // function (100,200,220)
            0x78D2E6, // type     (120,210,230)
            0x82DCAA, // string   (130,220,170)
            0xB4C878, // number   (180,200,120)
            0x506E8C, // comment  (80,110,140)
            0xC8DCEB, // operator (200,220,235)
            0x78D2E6, // constant (120,210,230)
            0xE6F0FA, // variable (230,240,250)
        )
    }

    /// Resolves the syntax theme bound to a chrome theme name, so code blocks and the theme gallery
    /// switch syntax palettes together with the chrome theme.
    pub fn from_chrome_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            "monokai" => Self::monokai(),
            "ocean" | "ocean_dark" => Self::ocean(),
            // The brand `dark` chrome (and the `vscode` chrome, and any unknown name) wraps a real
            // VS Code Dark+ token set for code blocks (the documented syntax exception): keywords
            // blue, strings orange-brown, comments green, functions yellow.
            _ => Self::vscode_dark(),
        }
    }

    /// Resolves the syntax palette for the Wave-7 themes gallery's canonical code block.
    ///
    /// This differs from [`Self::from_chrome_name`] on the `dark` theme only: the gallery's `dark`
    /// snippet uses the mono-orange brand syntax ([`Self::cortex_dark`] — orange keywords, amber
    /// identifiers) per the gallery design (VAL-THEME-014), whereas the ordinary in-chat code block
    /// binds `dark` to VS Code Dark+ (blue keywords). The `vscode` theme uses VS Code Dark+
    /// (VAL-THEME-022); light/monokai/ocean match their chrome palettes.
    pub fn gallery(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            "monokai" => Self::monokai(),
            "ocean" | "ocean_dark" => Self::ocean(),
            "vscode" | "vscode_dark" | "vscode-dark" => Self::vscode_dark(),
            // The brand `dark` gallery snippet uses the mono-orange brand syntax palette.
            _ => Self::cortex_dark(),
        }
    }

    /// Creates a Monokai-inspired theme.
    pub fn monokai() -> Self {
        let mut theme = Self::new();

        // Keywords
        theme.set("keyword", Style::new().fg(hex_color(0xF92672)));
        theme.set("keyword.control", Style::new().fg(hex_color(0xF92672)));
        theme.set("keyword.function", Style::new().fg(hex_color(0x66D9EF)));
        theme.set("keyword.operator", Style::new().fg(hex_color(0xF92672)));
        theme.set("keyword.type", Style::new().fg(hex_color(0x66D9EF)));

        // Types
        theme.set("type", Style::new().fg(hex_color(0x66D9EF)));
        theme.set(
            "type.builtin",
            Style::new().fg(hex_color(0x66D9EF)).italic(),
        );

        // Functions
        theme.set("function", Style::new().fg(hex_color(0xA6E22E)));
        theme.set("function.call", Style::new().fg(hex_color(0xA6E22E)));
        theme.set("function.method", Style::new().fg(hex_color(0xA6E22E)));

        // Variables
        theme.set("variable", Style::new().fg(hex_color(0xF8F8F2)));
        theme.set(
            "variable.parameter",
            Style::new().fg(hex_color(0xFD971F)).italic(),
        );

        // Constants
        theme.set("constant", Style::new().fg(hex_color(0xAE81FF)));
        theme.set("constant.numeric", Style::new().fg(hex_color(0xAE81FF)));
        theme.set("constant.character", Style::new().fg(hex_color(0xE6DB74)));

        // Strings
        theme.set("string", Style::new().fg(hex_color(0xE6DB74)));

        // Comments
        theme.set("comment", Style::new().fg(hex_color(0x75715E)));

        // Operators
        theme.set("operator", Style::new().fg(hex_color(0xF92672)));
        theme.set("punctuation", Style::new().fg(hex_color(0xF8F8F2)));

        theme
    }

    /// Sets a style for a capture name.
    #[inline]
    pub fn set(&mut self, capture: impl Into<String>, style: Style) {
        let capture = capture.into();
        self.styles.insert(capture, style);
        // Invalidate merge cache when styles change
        self.merge_cache.clear();
    }

    /// Gets the style for a capture name, with fallback to parent scopes.
    ///
    /// For example, if "keyword.control.flow" is requested but not found,
    /// it will try "keyword.control", then "keyword".
    pub fn get(&self, capture: &str) -> Style {
        // Try exact match first
        if let Some(style) = self.styles.get(capture) {
            return *style;
        }

        // Try progressively shorter prefixes
        let mut current = capture;
        while let Some(dot_pos) = current.rfind('.') {
            current = &current[..dot_pos];
            if let Some(style) = self.styles.get(current) {
                return *style;
            }
        }

        // Return default style if no match
        self.default_style
    }

    /// Gets the style for a capture name without fallback.
    #[inline]
    pub fn get_exact(&self, capture: &str) -> Option<Style> {
        self.styles.get(capture).copied()
    }

    /// Returns the default style.
    #[inline]
    pub fn default_style(&self) -> Style {
        self.default_style
    }

    /// Sets the default style for unhighlighted text.
    #[inline]
    pub fn set_default_style(&mut self, style: Style) {
        self.default_style = style;
    }

    /// Merges styles for multiple capture names.
    ///
    /// Later captures in the list take precedence.
    pub fn merge(&mut self, captures: &[&str]) -> Style {
        if captures.is_empty() {
            return self.default_style;
        }

        if captures.len() == 1 {
            return self.get(captures[0]);
        }

        // Check cache
        let cache_key = captures.join(":");
        if let Some(cached) = self.merge_cache.get(&cache_key) {
            return *cached;
        }

        // Merge styles
        let mut merged = self.default_style;
        for capture in captures {
            let style = self.get(capture);
            merged = merged.merge(&style);
        }

        // Cache the result
        self.merge_cache.insert(cache_key, merged);
        merged
    }

    /// Returns all registered capture names.
    pub fn captures(&self) -> impl Iterator<Item = &str> {
        self.styles.keys().map(|s| s.as_str())
    }

    /// Returns the number of registered styles.
    #[inline]
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    /// Returns true if no styles are registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

/// Builder for creating custom themes.
#[derive(Debug, Clone, Default)]
pub struct ThemeBuilder {
    theme: Theme,
}

impl ThemeBuilder {
    /// Creates a new theme builder.
    pub fn new() -> Self {
        Self {
            theme: Theme::new(),
        }
    }

    /// Starts from an existing theme.
    pub fn from_theme(theme: Theme) -> Self {
        Self { theme }
    }

    /// Sets the default style.
    pub fn default_style(mut self, style: Style) -> Self {
        self.theme.set_default_style(style);
        self
    }

    /// Adds a style for a capture name.
    pub fn style(mut self, capture: impl Into<String>, style: Style) -> Self {
        self.theme.set(capture, style);
        self
    }

    /// Adds a foreground color for a capture name.
    pub fn fg(self, capture: impl Into<String>, color: Color) -> Self {
        self.style(capture, Style::new().fg(color))
    }

    /// Adds a bold style for a capture name.
    pub fn bold(self, capture: impl Into<String>, color: Color) -> Self {
        self.style(capture, Style::new().fg(color).bold())
    }

    /// Adds an italic style for a capture name.
    pub fn italic(self, capture: impl Into<String>, color: Color) -> Self {
        self.style(capture, Style::new().fg(color).italic())
    }

    /// Builds the theme.
    pub fn build(self) -> Theme {
        self.theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_get_exact() {
        let theme = Theme::vscode_dark();
        assert!(theme.get_exact("keyword").is_some());
        assert!(theme.get_exact("nonexistent").is_none());
    }

    #[test]
    fn test_theme_fallback() {
        let mut theme = Theme::new();
        theme.set("keyword", Style::new().fg(Color::RED));

        // Should fall back to "keyword"
        let style = theme.get("keyword.control.flow");
        assert_eq!(style.fg, Some(Color::RED));
    }

    #[test]
    fn test_theme_merge() {
        let mut theme = Theme::new();
        theme.set("keyword", Style::new().fg(Color::BLUE));
        theme.set("emphasis", Style::new().bold());

        let merged = theme.merge(&["keyword", "emphasis"]);
        assert_eq!(merged.fg, Some(Color::BLUE));
        assert!(merged.attributes.is_bold());
    }

    #[test]
    fn from_chrome_name_binds_distinct_syntax_palettes() {
        // Each chrome theme resolves to a syntax theme; the keyword color differs across themes so
        // a fenced code block looks different under dark vs light/monokai/ocean.
        let dark = Theme::from_chrome_name("dark");
        let light = Theme::from_chrome_name("light");
        let monokai = Theme::from_chrome_name("monokai");
        let ocean = Theme::from_chrome_name("ocean");
        let vscode = Theme::from_chrome_name("vscode");

        let kw = |t: &Theme| t.get("keyword").fg;
        assert!(kw(&dark).is_some());
        assert_ne!(kw(&dark), kw(&light));
        assert_ne!(kw(&dark), kw(&monokai));
        assert_ne!(kw(&dark), kw(&ocean));

        // The brand dark chrome wraps a real VS Code Dark+ token set: blue keywords (#569CD6),
        // orange-brown strings, green comments, yellow functions (VAL-MD-016, VAL-MD-017).
        assert_eq!(dark.get("keyword").fg, Some(Color::from_rgb_u8(86, 156, 214)));
        assert_eq!(vscode.get("keyword").fg, dark.get("keyword").fg);
        assert_ne!(
            dark.get("keyword").fg,
            dark.get("string").fg,
            "multiple distinct token colors"
        );

        // Unknown names fall back to vscode_dark.
        assert_eq!(
            Theme::from_chrome_name("nonexistent").get("keyword").fg,
            dark.get("keyword").fg
        );
    }

    #[test]
    fn gallery_dark_uses_orange_keywords_and_vscode_uses_blue() {
        // The gallery's `dark` snippet uses the mono-orange brand syntax (orange keywords), which
        // is DISTINCT from the in-chat code block that binds `dark` to VS Code Dark+ blue keywords
        // (VAL-THEME-014 vs VAL-MD-016).
        let gallery_dark = Theme::gallery("dark");
        assert_eq!(
            gallery_dark.get("keyword").fg,
            Some(Color::from_rgb_u8(232, 146, 86)),
            "gallery dark keyword is brand ORANGE"
        );
        assert_ne!(
            gallery_dark.get("keyword").fg,
            Theme::from_chrome_name("dark").get("keyword").fg,
            "gallery dark diverges from the in-chat dark syntax"
        );
        // vscode gallery keyword is VS Code Dark+ blue (VAL-THEME-022).
        assert_eq!(
            Theme::gallery("vscode").get("keyword").fg,
            Some(Color::from_rgb_u8(86, 156, 214)),
            "gallery vscode keyword is VS Code blue"
        );
        // light gallery keyword is blue (VAL-THEME-016).
        assert_eq!(
            Theme::gallery("light").get("keyword").fg,
            Some(Color::from_rgb_u8(0, 0, 200)),
            "gallery light keyword is blue"
        );
        // Every gallery theme's keyword differs from its comment (multiple distinct colors).
        for name in ["dark", "light", "monokai", "ocean", "vscode"] {
            let t = Theme::gallery(name);
            assert_ne!(
                t.get("keyword").fg,
                t.get("comment").fg,
                "{name}: keyword and comment are distinct colors"
            );
        }
    }

    #[test]
    fn test_theme_builder() {
        let theme = ThemeBuilder::new()
            .default_style(Style::new().fg(Color::WHITE))
            .fg("keyword", Color::BLUE)
            .italic("comment", Color::GREEN)
            .build();

        assert_eq!(theme.get("keyword").fg, Some(Color::BLUE));
        assert!(theme.get("comment").attributes.is_italic());
    }
}
