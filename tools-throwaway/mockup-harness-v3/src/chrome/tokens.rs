//! Semantic visual tokens and the closed glyph/font policy.

/// The canonical terminal source dimensions.
pub const CANVAS_COLUMNS: u16 = 120;
pub const CANVAS_ROWS: u16 = 40;

/// Pixel size of one rasterized terminal cell in gallery/gate PNGs.
pub const CELL_WIDTH: u32 = 10;
pub const CELL_HEIGHT: u32 = 21;

/// Shared monospaced baseline within each cell (row-relative, from top).
///
/// DejaVu Sans Mono at size 16 has design ascent ≈15 and descent ≈4. With this
/// constant, non-descender bottoms land near `CELL_BASELINE - 1` and descenders
/// remain inside the cell. Never center glyph bitmaps by height.
pub const CELL_BASELINE: i32 = 16;

/// The canonical raster dimensions. Each text cell maps to 10 by 21 pixels.
pub const PNG_WIDTH: u32 = CANVAS_COLUMNS as u32 * CELL_WIDTH;
pub const PNG_HEIGHT: u32 = CANVAS_ROWS as u32 * CELL_HEIGHT;

/// Accent is reserved for focus/current work, never decoration.
pub const ACCENT_USE_BUDGET: usize = 3;

/// Every non-ASCII character allowed in a canonical hybrid v3 text artifact.
pub const ALLOWED_GLYPHS: &str = "● ○ ✓ ✗ › → ↑ ↓ ─ │ ╭ ╮ ╰ ╯ █ ░ · ⎿ ▸ ▾ …";

/// Expanded pure-Cortex dump chrome glyphs (mirrors mockup-harness-v4 tokens).
/// Active only when the pure-ownership sidecar exists on the live gallery root.
/// MC V2 premium adds Factory slot cells / mini ticks / selection bar glyphs.
pub const PURE_CORTEX_ALLOWED_GLYPHS: &str = concat!(
    "● ○ ✓ ✗ › → ↑ ↓ ─ │ ╭ ╮ ╰ ╯ █ ░ · ⎿ ▸ ▾ … ",
    "❯ ◆ ◇ ◈ ◉ ◎ • ⧉ ↗ ⇣ ❙ ",
    "⠋ ⠙ ⠹ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏ ",
    "▮ ▯ ▰ ▱ ▌ ▐ ",
    " "
);

/// Returns the hybrid closed functional glyph policy as one stable string.
pub const fn allowed_glyphs() -> &'static str {
    ALLOWED_GLYPHS
}

const PURE_CORTEX_SIDECAR: &str =
    "/projects/cortex-code/plans/assets/mockups-v3/.renderer-manifest-v1-pure-cortex";

/// True when pure-Cortex dump owns the live gallery roots.
pub fn pure_cortex_glyph_mode() -> bool {
    std::path::Path::new(PURE_CORTEX_SIDECAR).is_file()
}

/// True when a full braille block codepoint is used for pure-Cortex logos/spinners.
pub fn is_braille_glyph(c: char) -> bool {
    (0x2800..=0x28FF).contains(&(c as u32))
}

/// Returns whether a non-ASCII character is allowed for pure Cortex dump gallery text.
pub fn pure_cortex_glyph_allowed(c: char) -> bool {
    c.is_ascii() || PURE_CORTEX_ALLOWED_GLYPHS.contains(c) || is_braille_glyph(c)
}

/// The result elbow is approved for semantic transcript nesting, but the
/// embedded DejaVu pair intentionally has no glyph for it. The PNG renderer
/// draws this one character with a fixed local stroke instead of falling back
/// to a host font or a `.notdef` box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphRendering {
    EmbeddedFont,
    SyntheticElbow,
}

/// Returns the deterministic renderer strategy for an approved non-ASCII glyph.
pub fn glyph_rendering(glyph: char) -> Option<GlyphRendering> {
    if glyph == '⎿' {
        Some(GlyphRendering::SyntheticElbow)
    } else if glyph.is_ascii() || ALLOWED_GLYPHS.contains(glyph) {
        Some(GlyphRendering::EmbeddedFont)
    } else {
        None
    }
}

/// Rejects non-ASCII text that has not been explicitly approved for the active
/// gallery mode. Hybrid stays on the closed Cortex set; pure dump accepts the
/// expanded Cortex product chrome (prompt arrow, diamonds, braille, branch).
pub fn validate_glyphs(text: &str) -> Result<(), String> {
    let pure = pure_cortex_glyph_mode();
    for character in text.chars() {
        if character.is_ascii() {
            continue;
        }
        if pure {
            if !pure_cortex_glyph_allowed(character) {
                // Keep the canonical "unsupported glyph" phrase so negative
                // fixtures remain stable under pure-mode expansion.
                return Err(format!(
                    "unsupported glyph: {character} (pure-Cortex U+{:04X})",
                    character as u32
                ));
            }
        } else if !ALLOWED_GLYPHS.contains(character) {
            return Err(format!("unsupported glyph: {character}"));
        }
    }
    Ok(())
}

/// Rejects text that cannot occupy one stable canvas row without control-byte
/// interpretation. Audit and render paths call this before writing cells.
pub fn validate_text_safety(text: &str) -> Result<(), String> {
    if text.chars().count() > CANVAS_COLUMNS as usize || text.chars().any(char::is_control) {
        return Err("text width/control hazard: control-escape".to_owned());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl Rgb {
    pub const fn hex(self) -> &'static str {
        match (self.red, self.green, self.blue) {
            (0x10, 0x10, 0x11) => "#101011",
            (0x1A, 0x1A, 0x1C) => "#1A1A1C",
            (0xDD, 0xDA, 0xD4) => "#DDDAD4",
            (0xF6, 0xF3, 0xED) => "#F6F3ED",
            (0x99, 0x95, 0x8E) => "#99958E",
            (0x5F, 0x5C, 0x57) => "#5F5C57",
            (0x39, 0x37, 0x35) => "#393735",
            (0xD9, 0x82, 0x5F) => "#D9825F",
            (0x7D, 0x9B, 0xB8) => "#7D9BB8",
            (0x79, 0xA8, 0x89) => "#79A889",
            (0xC2, 0x9A, 0x60) => "#C29A60",
            (0xC6, 0x6F, 0x6F) => "#C66F6F",
            (0x17, 0x24, 0x1D) => "#17241D",
            (0x28, 0x19, 0x1B) => "#28191B",
            (0xFA, 0xF7, 0xF2) => "#FAF7F2",
            (0xE8, 0xE4, 0xDC) => "#E8E4DC",
            (0x33, 0x38, 0x3F) => "#33383F",
            (0x14, 0x17, 0x1C) => "#14171C",
            (0x6B, 0x72, 0x80) => "#6B7280",
            (0xC3, 0xC8, 0xD1) => "#C3C8D1",
            (0x7C, 0x82, 0x8C) => "#7C828C",
            (0xC0, 0x6B, 0x44) => "#C06B44",
            (0x3D, 0x6F, 0xA8) => "#3D6FA8",
            (0x4F, 0x79, 0x42) => "#4F7942",
            (0xA8, 0x7A, 0x2E) => "#A87A2E",
            (0xB5, 0x4A, 0x55) => "#B54A55",
            (0xE4, 0xEF, 0xDA) => "#E4EFDA",
            (0xF3, 0xE0, 0xDD) => "#F3E0DD",
            _ => "#000000",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum ColorToken {
    Canvas,
    Surface,
    Text,
    Bright,
    Muted,
    Faint,
    Border,
    Accent,
    Info,
    Ok,
    Warn,
    Bad,
    AddBackground,
    DeleteBackground,
}

impl ColorToken {
    pub const ALL: [Self; 14] = [
        Self::Canvas,
        Self::Surface,
        Self::Text,
        Self::Bright,
        Self::Muted,
        Self::Faint,
        Self::Border,
        Self::Accent,
        Self::Info,
        Self::Ok,
        Self::Warn,
        Self::Bad,
        Self::AddBackground,
        Self::DeleteBackground,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    values: [Rgb; 14],
}

impl Palette {
    pub const fn color(self, token: ColorToken) -> Rgb {
        self.values[token as usize]
    }

    pub const fn hex(self, token: ColorToken) -> &'static str {
        self.color(token).hex()
    }
}

const DARK: Palette = Palette {
    values: [
        Rgb {
            red: 0x10,
            green: 0x10,
            blue: 0x11,
        },
        Rgb {
            red: 0x1A,
            green: 0x1A,
            blue: 0x1C,
        },
        Rgb {
            red: 0xDD,
            green: 0xDA,
            blue: 0xD4,
        },
        Rgb {
            red: 0xF6,
            green: 0xF3,
            blue: 0xED,
        },
        Rgb {
            red: 0x99,
            green: 0x95,
            blue: 0x8E,
        },
        Rgb {
            red: 0x5F,
            green: 0x5C,
            blue: 0x57,
        },
        Rgb {
            red: 0x39,
            green: 0x37,
            blue: 0x35,
        },
        Rgb {
            red: 0xD9,
            green: 0x82,
            blue: 0x5F,
        },
        Rgb {
            red: 0x7D,
            green: 0x9B,
            blue: 0xB8,
        },
        Rgb {
            red: 0x79,
            green: 0xA8,
            blue: 0x89,
        },
        Rgb {
            red: 0xC2,
            green: 0x9A,
            blue: 0x60,
        },
        Rgb {
            red: 0xC6,
            green: 0x6F,
            blue: 0x6F,
        },
        Rgb {
            red: 0x17,
            green: 0x24,
            blue: 0x1D,
        },
        Rgb {
            red: 0x28,
            green: 0x19,
            blue: 0x1B,
        },
    ],
};

const LIGHT: Palette = Palette {
    values: [
        Rgb {
            red: 0xFA,
            green: 0xF7,
            blue: 0xF2,
        },
        Rgb {
            red: 0xE8,
            green: 0xE4,
            blue: 0xDC,
        },
        Rgb {
            red: 0x33,
            green: 0x38,
            blue: 0x3F,
        },
        Rgb {
            red: 0x14,
            green: 0x17,
            blue: 0x1C,
        },
        Rgb {
            red: 0x6B,
            green: 0x72,
            blue: 0x80,
        },
        Rgb {
            red: 0xC3,
            green: 0xC8,
            blue: 0xD1,
        },
        Rgb {
            red: 0x7C,
            green: 0x82,
            blue: 0x8C,
        },
        Rgb {
            red: 0xC0,
            green: 0x6B,
            blue: 0x44,
        },
        Rgb {
            red: 0x3D,
            green: 0x6F,
            blue: 0xA8,
        },
        Rgb {
            red: 0x4F,
            green: 0x79,
            blue: 0x42,
        },
        Rgb {
            red: 0xA8,
            green: 0x7A,
            blue: 0x2E,
        },
        Rgb {
            red: 0xB5,
            green: 0x4A,
            blue: 0x55,
        },
        Rgb {
            red: 0xE4,
            green: 0xEF,
            blue: 0xDA,
        },
        Rgb {
            red: 0xF3,
            green: 0xE0,
            blue: 0xDD,
        },
    ],
};

/// Returns the exact approved token table for a canonical v3 theme.
pub const fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

/// Confirms that a caller is using the declared semantic color value for a
/// token instead of an arbitrary per-screen RGB override.
pub fn validate_token_hex(token: ColorToken, value: &str) -> Result<(), String> {
    if palette(Theme::Dark).hex(token) != value {
        return Err(format!("token/hex mismatch: accent:{value}"));
    }
    Ok(())
}

/// Semantic styling is declared in tokens, never with per-screen RGB values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextStyle {
    pub foreground: ColorToken,
    pub background: Option<ColorToken>,
    pub bold: bool,
}

impl TextStyle {
    pub const fn text() -> Self {
        Self {
            foreground: ColorToken::Text,
            background: None,
            bold: false,
        }
    }

    pub const fn bright() -> Self {
        Self {
            foreground: ColorToken::Bright,
            background: None,
            bold: true,
        }
    }

    pub const fn muted() -> Self {
        Self {
            foreground: ColorToken::Muted,
            background: None,
            bold: false,
        }
    }

    pub const fn accent() -> Self {
        Self {
            foreground: ColorToken::Accent,
            background: None,
            bold: true,
        }
    }

    /// Soft surface wash for user-turn surfaces (Mission Control density / VAL-V3-CROSS-020).
    pub const fn user_turn() -> Self {
        Self {
            foreground: ColorToken::Bright,
            background: Some(ColorToken::Surface),
            bold: true,
        }
    }
}

#[cfg(feature = "png")]
const FONT_REGULAR: &[u8] = include_bytes!("../../assets/DejaVuSansMono.ttf");
#[cfg(feature = "png")]
const FONT_BOLD: &[u8] = include_bytes!("../../assets/DejaVuSansMono-Bold.ttf");

/// Loads only the pair of font files embedded in the isolated harness.
#[cfg(feature = "png")]
pub fn embedded_fonts() -> Result<(fontdue::Font, fontdue::Font), String> {
    let regular = fontdue::Font::from_bytes(FONT_REGULAR, fontdue::FontSettings::default())
        .map_err(|error| format!("embedded regular font is invalid: {error}"))?;
    let bold = fontdue::Font::from_bytes(FONT_BOLD, fontdue::FontSettings::default())
        .map_err(|error| format!("embedded bold font is invalid: {error}"))?;
    Ok((regular, bold))
}

/// Fails before rendering when either embedded font cannot display a functional glyph.
#[cfg(feature = "png")]
pub fn verify_embedded_font_coverage() -> Result<(), String> {
    let (regular, bold) = embedded_fonts()?;
    for glyph in ALLOWED_GLYPHS
        .chars()
        .filter(|glyph| !glyph.is_whitespace())
    {
        if regular.lookup_glyph_index(glyph) == 0
            && glyph_rendering(glyph) != Some(GlyphRendering::SyntheticElbow)
        {
            return Err(format!("regular font lacks allowed glyph: {glyph}"));
        }
        if bold.lookup_glyph_index(glyph) == 0
            && glyph_rendering(glyph) != Some(GlyphRendering::SyntheticElbow)
        {
            return Err(format!("bold font lacks allowed glyph: {glyph}"));
        }
    }
    Ok(())
}
