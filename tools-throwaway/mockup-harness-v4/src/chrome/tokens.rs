//! Pure CortexNight / CortexDay tokens and expanded Cortex glyph policy.
//!
//! Source of truth: `library/cortex-pure-dump-spec.md` +
//! `/tmp/cortex-build-ref/.../theme/{cortexnight,cortexday}.rs`.
//! No copper Cortex Identity tokens live here.

/// Canonical gallery terminal source dimensions.
pub const CANVAS_COLUMNS: u16 = 120;
pub const CANVAS_ROWS: u16 = 40;

/// Pixel size of one rasterized terminal cell in gallery/gate PNGs.
pub const CELL_WIDTH: u32 = 10;
pub const CELL_HEIGHT: u32 = 21;

/// Shared monospaced baseline within each cell (row-relative, from top).
pub const CELL_BASELINE: i32 = 16;

/// Canonical raster dimensions (1200x840).
pub const PNG_WIDTH: u32 = CANVAS_COLUMNS as u32 * CELL_WIDTH;
pub const PNG_HEIGHT: u32 = CANVAS_ROWS as u32 * CELL_HEIGHT;

/// Expanded Cortex glyph set. Pure dump allows diamonds, prompt arrow, braille,
/// powerline branch, and other product chrome beyond the old Cortex closed set.
/// MC V2 premium adds Factory slot cells / mini ticks / selection bar glyphs.
pub const ALLOWED_GLYPHS: &str = concat!(
    "● ○ ✓ ✗ › → ↑ ↓ ─ │ ╭ ╮ ╰ ╯ █ ░ · ⎿ ▸ ▾ … ",
    "❯ ◆ ◇ ◈ ◉ ◎ • ⧉ ↗ ⇣ ❙ ",
    "⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏ ",
    "▮ ▯ ▰ ▱ ▌ ▐ ",
    " "
);

/// Full braille cell block (U+2800–U+28FF) is allowed for logos/spinners.
pub fn is_braille(c: char) -> bool {
    (0x2800..=0x28FF).contains(&(c as u32))
}

/// Returns whether a non-ASCII character may appear in pure Cortex TXT.
pub fn glyph_allowed(c: char) -> bool {
    c.is_ascii() || ALLOWED_GLYPHS.contains(c) || is_braille(c)
}

/// Rejects non-approved non-ASCII text before publish.
pub fn validate_glyphs(text: &str) -> Result<(), String> {
    for character in text.chars() {
        if !glyph_allowed(character) {
            return Err(format!("unsupported pure-Cortex glyph: {character} (U+{:04X})", character as u32));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphRendering {
    EmbeddedFont,
    SyntheticElbow,
    SyntheticPromptArrow,
    SyntheticDiamond,
    SyntheticBrailleDot,
    /// Factory MC slot / progress / selection bar blocks (▮ ▯ ▰ ▱ ▌ ▐).
    SyntheticBlock,
}

/// Deterministic strategy when DejaVu coverage is incomplete for product chrome.
pub fn glyph_rendering(glyph: char) -> Option<GlyphRendering> {
    if glyph == '⎿' {
        Some(GlyphRendering::SyntheticElbow)
    } else if glyph == '❯' {
        Some(GlyphRendering::SyntheticPromptArrow)
    } else if matches!(glyph, '◆' | '◇' | '◈') {
        Some(GlyphRendering::SyntheticDiamond)
    } else if matches!(glyph, '▮' | '▯' | '▰' | '▱' | '▌' | '▐') {
        Some(GlyphRendering::SyntheticBlock)
    } else if is_braille(glyph) {
        // Prefer emb font if present; synthetic dots as fail-safe.
        Some(GlyphRendering::SyntheticBrailleDot)
    } else if glyph.is_ascii() || ALLOWED_GLYPHS.contains(glyph) {
        Some(GlyphRendering::EmbeddedFont)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub fn hex_string(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

/// Semantic color roles for pure product chrome paints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum ColorToken {
    Canvas,
    Surface,
    CodeBg,
    Text,
    Bright,
    Secondary,
    Muted,
    Dim,
    Border,
    /// Magenta / violet Cortex accent (`#bb9af7` night / `#7D4BC6` day).
    Accent,
    Info,
    Ok,
    Warn,
    Bad,
    Running,
    Path,
    Model,
    Plan,
    AddBackground,
    DeleteBackground,
}

impl ColorToken {
    pub const ALL: [Self; 20] = [
        Self::Canvas,
        Self::Surface,
        Self::CodeBg,
        Self::Text,
        Self::Bright,
        Self::Secondary,
        Self::Muted,
        Self::Dim,
        Self::Border,
        Self::Accent,
        Self::Info,
        Self::Ok,
        Self::Warn,
        Self::Bad,
        Self::Running,
        Self::Path,
        Self::Model,
        Self::Plan,
        Self::AddBackground,
        Self::DeleteBackground,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    /// CortexNight product default.
    CortexNight,
    /// CortexDay light peer for theme galleries.
    CortexDay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    values: [Rgb; 20],
}

impl Palette {
    pub const fn color(self, token: ColorToken) -> Rgb {
        self.values[token as usize]
    }

    pub fn hex(self, token: ColorToken) -> String {
        self.color(token).hex_string()
    }
}

/// CortexNight raster tokens (truecolor RGB from source theme + runtime SGR).
pub const CORTEX_NIGHT: Palette = Palette {
    values: [
        Rgb::new(0x14, 0x14, 0x14), // Canvas      #141414
        Rgb::new(0x24, 0x24, 0x24), // Surface     #242424
        Rgb::new(0x1c, 0x1c, 0x1c), // CodeBg      #1c1c1c
        Rgb::new(0xe1, 0xe1, 0xe1), // Text        #e1e1e1
        Rgb::new(0xf3, 0xf3, 0xf3), // Bright      #f3f3f3
        Rgb::new(0xc8, 0xc8, 0xc8), // Secondary   #c8c8c8
        Rgb::new(0x6c, 0x6c, 0x6c), // Muted       #6c6c6c
        Rgb::new(0x58, 0x58, 0x58), // Dim         #585858
        Rgb::new(0x32, 0x32, 0x37), // Border      #323237
        Rgb::new(0xbb, 0x9a, 0xf7), // Accent      #bb9af7 magenta
        Rgb::new(0x7a, 0xa2, 0xf7), // Info        #7aa2f7
        Rgb::new(0x9e, 0xce, 0x6a), // Ok          #9ece6a
        Rgb::new(0xe0, 0xaf, 0x68), // Warn        #e0af68
        Rgb::new(0xf7, 0x76, 0x8e), // Bad         #f7768e
        Rgb::new(0x7d, 0xcf, 0xff), // Running     #7dcfff
        Rgb::new(0xff, 0x9e, 0x64), // Path        #ff9e64
        Rgb::new(0x1a, 0xbc, 0x9c), // Model       #1abc9c
        Rgb::new(0xff, 0xdb, 0x8d), // Plan        #FFDB8D
        Rgb::new(0x06, 0x38, 0x06), // AddBackground
        Rgb::new(0x42, 0x0e, 0x14), // DeleteBackground
    ],
};

/// CortexDay light peer (deepened TokyoNight hues).
pub const CORTEX_DAY: Palette = Palette {
    values: [
        Rgb::new(0xee, 0xee, 0xee), // Canvas      #eeeeee
        Rgb::new(0xde, 0xde, 0xde), // Surface     #dedede
        Rgb::new(0xe4, 0xe4, 0xe4), // CodeBg
        Rgb::new(0x26, 0x26, 0x26), // Text        #262626
        Rgb::new(0x1a, 0x1a, 0x1a), // Bright
        Rgb::new(0x44, 0x44, 0x44), // Secondary   #444444
        Rgb::new(0x76, 0x76, 0x76), // Muted       #767676
        Rgb::new(0xa5, 0xa5, 0xa5), // Dim
        Rgb::new(0xc8, 0xc8, 0xcd), // Border
        Rgb::new(0x7d, 0x4b, 0xc6), // Accent      #7D4BC6
        Rgb::new(0x2f, 0x64, 0xd2), // Info
        Rgb::new(0x37, 0x8e, 0x23), // Ok
        Rgb::new(0xa2, 0x76, 0x12), // Warn
        Rgb::new(0xcd, 0x30, 0x48), // Bad
        Rgb::new(0x00, 0x82, 0xaa), // Running
        Rgb::new(0xc3, 0x69, 0x1e), // Path
        Rgb::new(0x0a, 0x8e, 0x70), // Model
        Rgb::new(0xa8, 0x78, 0x0a), // Plan
        Rgb::new(0xda, 0xf2, 0xdc), // AddBackground
        Rgb::new(0xf5, 0xda, 0xde), // DeleteBackground
    ],
};

pub const fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::CortexNight => CORTEX_NIGHT,
        Theme::CortexDay => CORTEX_DAY,
    }
}

/// Copper Cortex Identity hex values must never win as product tokens in pure dump.
pub const FORBIDDEN_COPPER_ACCENT: &str = "#d9825f";
pub const FORBIDDEN_CODE_BRAND: &str = "Code";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextStyle {
    pub foreground: ColorToken,
    pub background: Option<ColorToken>,
    pub bold: bool,
}

impl TextStyle {
    pub const fn plain(fg: ColorToken) -> Self {
        Self {
            foreground: fg,
            background: None,
            bold: false,
        }
    }

    pub const fn bold(fg: ColorToken) -> Self {
        Self {
            foreground: fg,
            background: None,
            bold: true,
        }
    }

    pub const fn on(fg: ColorToken, bg: ColorToken) -> Self {
        Self {
            foreground: fg,
            background: Some(bg),
            bold: false,
        }
    }

    pub const fn accent() -> Self {
        Self::bold(ColorToken::Accent)
    }

    pub const fn user_turn() -> Self {
        Self {
            foreground: ColorToken::Secondary,
            background: Some(ColorToken::Surface),
            bold: true,
        }
    }
}

#[cfg(feature = "png")]
const FONT_REGULAR: &[u8] = include_bytes!("../../assets/DejaVuSansMono.ttf");
#[cfg(feature = "png")]
const FONT_BOLD: &[u8] = include_bytes!("../../assets/DejaVuSansMono-Bold.ttf");

#[cfg(feature = "png")]
pub fn embedded_fonts() -> Result<(fontdue::Font, fontdue::Font), String> {
    let regular = fontdue::Font::from_bytes(FONT_REGULAR, fontdue::FontSettings::default())
        .map_err(|error| format!("embedded regular font is invalid: {error}"))?;
    let bold = fontdue::Font::from_bytes(FONT_BOLD, fontdue::FontSettings::default())
        .map_err(|error| format!("embedded bold font is invalid: {error}"))?;
    Ok((regular, bold))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cortexnight_core_hex_values() {
        let p = palette(Theme::CortexNight);
        assert_eq!(p.hex(ColorToken::Canvas), "#141414");
        assert_eq!(p.hex(ColorToken::Text), "#e1e1e1");
        assert_eq!(p.hex(ColorToken::Accent), "#bb9af7");
        assert_eq!(p.hex(ColorToken::Secondary), "#c8c8c8");
        assert_eq!(p.hex(ColorToken::Muted), "#6c6c6c");
        assert_eq!(p.hex(ColorToken::Running), "#7dcfff");
        assert_eq!(p.hex(ColorToken::Bad), "#f7768e");
        assert_eq!(p.hex(ColorToken::Ok), "#9ece6a");
        assert_ne!(p.hex(ColorToken::Accent).to_lowercase(), FORBIDDEN_COPPER_ACCENT);
    }

    #[test]
    fn cortexday_core_hex_values() {
        let p = palette(Theme::CortexDay);
        assert_eq!(p.hex(ColorToken::Canvas), "#eeeeee");
        assert_eq!(p.hex(ColorToken::Text), "#262626");
        assert_eq!(p.hex(ColorToken::Accent), "#7d4bc6");
    }

    #[test]
    fn expanded_glyphs_cover_prompt_diamonds_stop_putty() {
        for c in [
            '❯', '◆', '◇', '◈', '◉', '◎', '•', '✓', '✗', '⇣', '', '⧉', '↗', '▮', '▯',
            '▰', '▱', '▌', '▐',
        ] {
            assert!(glyph_allowed(c), "expected {c} allowed");
        }
        // Braille block used by logo / spinner.
        assert!(glyph_allowed('\u{2800}'));
        assert!(glyph_allowed('\u{28FF}'));
        assert!(glyph_allowed('⠋'));
        assert!(glyph_allowed('⠧'));
        // `[stop]` chars are ASCII.
        assert!(validate_glyphs("[stop]").is_ok());
        // Shift+Tab puttyline is ASCII plus middots/pipes already allowed.
        assert!(validate_glyphs("Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts").is_ok());
        // Copper star still banned.
        assert!(validate_glyphs("✦").is_err());
    }
}
