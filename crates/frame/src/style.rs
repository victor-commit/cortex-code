// SPDX-License-Identifier: MIT
// Provenance: ported source (direct-port-allowed), adapted to this workspace
// Source-Repo: CortexLM/cortex-code
// Source-Commit: 7954d0218faf6f26aa84df0f6d8b09ea9f2e5ac9
// Source-Path: src/cortex-core/src/style.rs
// Target-Path: crates/frame/src/style.rs
// Port-Mode: adapt
// Adaptation: crate renamed cortex-frame -> frame (cortex_frame -> frame in doc references). The
//   full upstream palette is ported (constants + ThemeColors + CortexStyle + legacy aliases) so the
//   ported tui-components widgets can consume the same theme tokens the markdown renderer uses.
// Upstream-License: MIT
// See: NOTICE (Direct-Port Provenance Ledger)

//! Cortex color palette and style helpers used across the ported TUI framework.
//!
//! The default `dark` theme is the mono-orange Cortex brand on near-black `#0d0d0d` (see the
//! BRAND PALETTE constants). The legacy ocean/cyan constants are retained for the ported
//! `tui-components` widgets and the non-default themes. `ThemeColors` is the palette source of
//! truth consumed by both the markdown renderer (`frame::markdown`) and the TUI screens.

use std::collections::BTreeMap;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use ratatui::style::{Color, Modifier, Style};

// ============================================================
// BRAND COLORS - Primary accent colors
// ============================================================

/// Primary accent color.
pub const CYAN_PRIMARY: Color = Color::Rgb(0, 255, 163); // #00FFA3

/// Light secondary accent.
pub const SKY_BLUE: Color = Color::Rgb(100, 255, 180); // #64FFB4

/// Bright accent for highlights.
pub const ELECTRIC_BLUE: Color = Color::Rgb(50, 255, 150); // #32FF96

/// Mid accent used for links and interactive elements.
pub const DEEP_CYAN: Color = Color::Rgb(0, 200, 130); // #00C882

/// Dark accent color.
pub const TEAL: Color = Color::Rgb(0, 139, 87); // #008B57

// ============================================================
// BACKGROUND COLORS - Dark navy base
// ============================================================

/// Main background - deep void.
pub const VOID: Color = Color::Rgb(10, 22, 40); // #0A1628

/// Surface level 0 - darkest surface.
pub const SURFACE_0: Color = Color::Rgb(13, 27, 42); // #0D1B2A

/// Surface level 1 - mid surface.
pub const SURFACE_1: Color = Color::Rgb(27, 40, 56); // #1B2838

/// Surface level 2 - light surface.
pub const SURFACE_2: Color = Color::Rgb(36, 59, 83); // #243B53

/// Surface level 3 - lightest surface.
pub const SURFACE_3: Color = Color::Rgb(51, 78, 104); // #334E68

// ============================================================
// TEXT COLORS
// ============================================================

/// Primary text.
pub const TEXT: Color = Color::Rgb(255, 255, 255); // #FFFFFF

/// Dimmed secondary text.
pub const TEXT_DIM: Color = Color::Rgb(130, 154, 177); // #829AB1

/// Muted text for background elements.
pub const TEXT_MUTED: Color = Color::Rgb(72, 101, 129); // #486581

/// Bright text for emphasis.
pub const TEXT_BRIGHT: Color = Color::Rgb(255, 255, 255); // #FFFFFF

// ============================================================
// SEMANTIC COLORS
// ============================================================

/// Success accent.
pub const SUCCESS: Color = Color::Rgb(0, 245, 212); // #00F5D4

/// Warning - golden amber for warnings.
pub const WARNING: Color = Color::Rgb(255, 200, 87); // #FFC857

/// Error - coral red for errors.
pub const ERROR: Color = Color::Rgb(255, 107, 107); // #FF6B6B

/// Informational accent.
pub const INFO: Color = Color::Rgb(72, 202, 228); // #48CAE4

/// Highlight / emphasis accent.
pub const HIGHLIGHT: Color = Color::Rgb(125, 249, 255); // #7DF9FF

// ============================================================
// BORDER COLORS
// ============================================================

/// Normal border color.
pub const BORDER: Color = Color::Rgb(27, 73, 101); // #1B4965

/// Focused border - bright accent for active elements.
pub const BORDER_FOCUS: Color = Color::Rgb(0, 255, 163); // #00FFA3

/// Dim border - very subtle border.
pub const BORDER_DIM: Color = Color::Rgb(16, 42, 67); // #102A43

// ============================================================
// BRAND PALETTE - mono-orange on near-black (#0d0d0d)
// ============================================================
//
// The brand palette is the source of truth for the default `dark` theme and is
// the token set every screen draws from. ORANGE is the only UI hue; greens,
// reds and blues are reserved for status dots, diff +/- rows and code syntax.
// Exact RGB values mirror `library/design-system.md` (the Python layout kit).

/// Page background — near-black `#0d0d0d`.
pub const CANVAS: Color = Color::Rgb(13, 13, 13);

/// Primary brand accent: prompts, brand glyphs, selection fill.
pub const ORANGE: Color = Color::Rgb(232, 146, 86);

/// Dim orange: borders and secondary marks.
pub const ORANGE_D: Color = Color::Rgb(150, 92, 54);

/// Lighter accent: titles, highlights, active marks.
pub const AMBER: Color = Color::Rgb(242, 183, 120);

/// Body text.
pub const TXT: Color = Color::Rgb(208, 208, 206);

/// Emphasized / selected text.
pub const BRIGHT: Color = Color::Rgb(242, 242, 240);

/// Secondary text.
pub const DIM: Color = Color::Rgb(140, 140, 140);

/// Tertiary text / placeholders / hints.
pub const FAINT: Color = Color::Rgb(92, 92, 98);

/// Rules, dividers, faded backdrop.
pub const GHOST: Color = Color::Rgb(60, 60, 66);

/// Panel surface.
pub const PANEL: Color = Color::Rgb(22, 22, 26);

/// Alternate panel surface.
pub const PANEL2: Color = Color::Rgb(28, 28, 33);

/// Status-bar / find-bar background.
pub const BAR: Color = Color::Rgb(20, 20, 24);

/// Warm selected-row background (the inverse-selection fill).
pub const SELBG: Color = Color::Rgb(44, 35, 26);

/// Very soft warm tint (working band).
pub const SOFT: Color = Color::Rgb(34, 28, 22);

/// Brand card / panel border.
pub const BORDER_BRAND: Color = Color::Rgb(92, 66, 46);

/// Saturated success green (reads green at small sizes).
pub const OKG: Color = Color::Rgb(96, 204, 110);

/// Saturated error red.
pub const ERR: Color = Color::Rgb(224, 96, 96);

/// Info dot blue (status only, never chrome).
pub const INFO_DOT: Color = Color::Rgb(96, 170, 224);

/// Diff insert meta `+N` green.
pub const GREEN: Color = Color::Rgb(130, 196, 130);

/// Diff delete meta `-N` red.
pub const RED: Color = Color::Rgb(216, 122, 122);

/// Rare info-dot blue.
pub const BLUE: Color = Color::Rgb(138, 176, 224);

/// Diff insert text.
pub const ADD_FG: Color = Color::Rgb(124, 196, 124);

/// Diff insert row background.
pub const ADD_BG: Color = Color::Rgb(18, 38, 22);

/// Diff delete text.
pub const DEL_FG: Color = Color::Rgb(212, 118, 108);

/// Diff delete row background.
pub const DEL_BG: Color = Color::Rgb(46, 22, 22);

/// Diff line numbers.
pub const LNUM: Color = Color::Rgb(110, 110, 118);

/// Fenced code-block background.
pub const CODEBG: Color = Color::Rgb(24, 24, 28);

/// Inline-code chip background.
pub const INLBG: Color = Color::Rgb(40, 34, 28);

// ============================================================
// THEME COLORS STRUCT - For future theme switching
// ============================================================

/// Theme color configuration for supporting multiple themes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    /// Primary accent color.
    pub primary: Color,
    /// Secondary accent color.
    pub secondary: Color,
    /// Bright accent for highlights.
    pub accent: Color,
    /// Main background color.
    pub background: Color,
    /// Surface colors (0=darkest, 3=lightest).
    pub surface: [Color; 4],
    /// Primary text color.
    pub text: Color,
    /// Dimmed text color.
    pub text_dim: Color,
    /// Muted text color.
    pub text_muted: Color,
    /// Emphasized / brightest text color.
    pub text_bright: Color,
    /// Faded rule / divider / backdrop color.
    pub ghost: Color,
    /// Selected-row foreground (explicit inverse pair with `selection_bg`).
    pub selection_fg: Color,
    /// Selected-row background (explicit inverse pair with `selection_fg`).
    pub selection_bg: Color,
    /// Success color.
    pub success: Color,
    /// Warning color.
    pub warning: Color,
    /// Error color.
    pub error: Color,
    /// Info color.
    pub info: Color,
    /// Normal border color.
    pub border: Color,
    /// Focused border color.
    pub border_focus: Color,
    /// Card/panel border color for brand cards (the MOTD welcome card and meta panels); a dimmer
    /// tone than `border` on the brand theme so cards read softer than active chrome.
    pub card_border: Color,
    /// Very soft warm surface for the working/loading band (a tint just above the canvas); every
    /// theme defines a non-canvas value so the band never falls through to a default-black bar.
    pub soft: Color,
}

impl ThemeColors {
    /// Dark theme (default) — the mono-orange Cortex brand on near-black `#0d0d0d`.
    pub fn dark() -> Self {
        Self {
            primary: ORANGE,
            secondary: AMBER,
            accent: AMBER,
            background: CANVAS,
            surface: [BAR, PANEL, PANEL2, SELBG],
            text: TXT,
            text_dim: DIM,
            text_muted: FAINT,
            text_bright: BRIGHT,
            ghost: GHOST,
            selection_fg: CANVAS,
            selection_bg: ORANGE,
            success: OKG,
            warning: AMBER,
            error: ERR,
            info: INFO_DOT,
            border: ORANGE_D,
            border_focus: ORANGE,
            card_border: BORDER_BRAND,
            soft: SOFT,
        }
    }

    /// Light theme — dark text on a light canvas.
    pub fn light() -> Self {
        Self {
            primary: Color::Rgb(0, 150, 100),
            secondary: Color::Rgb(0, 120, 80),
            accent: Color::Rgb(0, 120, 80),
            background: Color::Rgb(255, 255, 255),
            surface: [
                Color::Rgb(245, 245, 245),
                Color::Rgb(235, 235, 235),
                Color::Rgb(225, 225, 225),
                Color::Rgb(215, 215, 215),
            ],
            text: Color::Rgb(30, 30, 30),
            text_dim: Color::Rgb(100, 100, 100),
            text_muted: Color::Rgb(150, 150, 150),
            text_bright: Color::Rgb(10, 10, 10),
            ghost: Color::Rgb(200, 200, 200),
            selection_fg: Color::Rgb(255, 255, 255),
            selection_bg: Color::Rgb(0, 150, 100),
            success: Color::Rgb(0, 150, 0),
            warning: Color::Rgb(200, 150, 0),
            error: Color::Rgb(200, 50, 50),
            info: Color::Rgb(50, 100, 200),
            border: Color::Rgb(200, 200, 200),
            border_focus: Color::Rgb(0, 150, 100),
            card_border: Color::Rgb(205, 205, 205),
            soft: Color::Rgb(240, 238, 232),
        }
    }

    /// Ocean theme — cool blues/teals on deep navy.
    pub fn ocean_dark() -> Self {
        Self {
            primary: Color::Rgb(0, 200, 200),
            secondary: Color::Rgb(100, 200, 220),
            accent: Color::Rgb(0, 180, 180),
            background: Color::Rgb(10, 25, 47),
            surface: [
                Color::Rgb(15, 35, 60),
                Color::Rgb(25, 50, 80),
                Color::Rgb(35, 65, 100),
                Color::Rgb(45, 80, 120),
            ],
            text: Color::Rgb(230, 240, 250),
            text_dim: Color::Rgb(140, 170, 200),
            text_muted: Color::Rgb(80, 110, 140),
            text_bright: Color::Rgb(240, 248, 255),
            ghost: Color::Rgb(40, 80, 120),
            selection_fg: Color::Rgb(10, 25, 47),
            selection_bg: Color::Rgb(0, 200, 200),
            success: Color::Rgb(0, 220, 180),
            warning: Color::Rgb(255, 200, 100),
            error: Color::Rgb(255, 100, 100),
            info: Color::Rgb(100, 180, 255),
            border: Color::Rgb(40, 80, 120),
            border_focus: Color::Rgb(0, 200, 200),
            card_border: Color::Rgb(40, 80, 120),
            soft: Color::Rgb(18, 40, 64),
        }
    }

    /// Monokai theme — classic code-editor palette.
    pub fn monokai() -> Self {
        Self {
            primary: Color::Rgb(166, 226, 46),
            secondary: Color::Rgb(102, 217, 239),
            accent: Color::Rgb(249, 38, 114),
            background: Color::Rgb(39, 40, 34),
            surface: [
                Color::Rgb(45, 46, 40),
                Color::Rgb(55, 56, 50),
                Color::Rgb(65, 66, 60),
                Color::Rgb(75, 76, 70),
            ],
            text: Color::Rgb(248, 248, 242),
            text_dim: Color::Rgb(180, 180, 170),
            text_muted: Color::Rgb(117, 113, 94),
            text_bright: Color::Rgb(255, 255, 250),
            ghost: Color::Rgb(70, 71, 65),
            selection_fg: Color::Rgb(39, 40, 34),
            selection_bg: Color::Rgb(166, 226, 46),
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(230, 219, 116),
            error: Color::Rgb(249, 38, 114),
            info: Color::Rgb(102, 217, 239),
            border: Color::Rgb(70, 71, 65),
            border_focus: Color::Rgb(166, 226, 46),
            card_border: Color::Rgb(70, 71, 65),
            soft: Color::Rgb(50, 51, 44),
        }
    }

    /// VS Code Dark+ chrome theme — the full VS Code palette, not just code syntax.
    pub fn vscode() -> Self {
        Self {
            primary: Color::Rgb(0, 122, 204),
            secondary: Color::Rgb(86, 156, 214),
            accent: Color::Rgb(86, 156, 214),
            background: Color::Rgb(30, 30, 30),
            surface: [
                Color::Rgb(37, 37, 38),
                Color::Rgb(45, 45, 46),
                Color::Rgb(55, 55, 56),
                Color::Rgb(65, 65, 66),
            ],
            text: Color::Rgb(212, 212, 212),
            text_dim: Color::Rgb(150, 150, 150),
            text_muted: Color::Rgb(110, 110, 110),
            text_bright: Color::Rgb(255, 255, 255),
            ghost: Color::Rgb(62, 62, 66),
            selection_fg: Color::Rgb(255, 255, 255),
            selection_bg: Color::Rgb(0, 122, 204),
            success: Color::Rgb(137, 209, 133),
            warning: Color::Rgb(204, 167, 0),
            error: Color::Rgb(244, 71, 71),
            info: Color::Rgb(86, 156, 214),
            border: Color::Rgb(62, 62, 66),
            border_focus: Color::Rgb(0, 122, 204),
            card_border: Color::Rgb(62, 62, 66),
            soft: Color::Rgb(42, 42, 40),
        }
    }

    /// Get a theme by name.
    pub fn from_name(name: &str) -> Self {
        let key = name.to_lowercase();
        if let Some(theme) = CUSTOM_THEMES
            .read()
            .ok()
            .and_then(|themes| themes.get(&key).cloned())
        {
            return theme;
        }
        match key.as_str() {
            "light" => Self::light(),
            "ocean" | "ocean_dark" => Self::ocean_dark(),
            "monokai" => Self::monokai(),
            "vscode" | "vscode_dark" | "vscode-dark" => Self::vscode(),
            // Default (and any unknown name) resolves to the mono-orange brand `dark`.
            _ => Self::dark(),
        }
    }

    /// Get all available theme names.
    pub fn available_themes() -> &'static [&'static str] {
        BUILTIN_THEME_NAMES
    }

    /// Get all theme names, including custom JSON themes loaded at runtime.
    #[must_use]
    pub fn available_theme_names() -> Vec<String> {
        let mut names = BUILTIN_THEME_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        if let Ok(custom) = CUSTOM_THEMES.read() {
            names.extend(custom.keys().cloned());
        }
        names.sort();
        names.dedup();
        names
    }

    /// Register one custom theme by name (case-insensitive key).
    pub fn register_custom_theme(name: &str, theme: ThemeColors) {
        let key = name.trim().to_lowercase();
        if key.is_empty() {
            return;
        }
        if let Ok(mut custom) = CUSTOM_THEMES.write() {
            custom.insert(key, theme);
        }
    }

    /// Remove every custom runtime theme.
    pub fn clear_custom_themes() {
        if let Ok(mut custom) = CUSTOM_THEMES.write() {
            custom.clear();
        }
    }
}

/// The five canonical chrome themes. `dark` (the mono-orange brand) is first, so it is the
/// built-in default before any config is loaded.
const BUILTIN_THEME_NAMES: &[&str] = &["dark", "light", "monokai", "ocean", "vscode"];

static CUSTOM_THEMES: Lazy<RwLock<BTreeMap<String, ThemeColors>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));

impl Default for ThemeColors {
    fn default() -> Self {
        Self::dark()
    }
}

// ============================================================
// CORTEX STYLE HELPER
// ============================================================

/// Helper struct providing pre-configured styles for common UI elements.
///
/// All methods return fresh `Style` instances - no internal state is maintained.
pub struct CortexStyle;

impl CortexStyle {
    /// Default style: primary text on void background.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Style {
        Style::default().fg(TEXT).bg(VOID)
    }

    /// Header style: bright white bold text for titles and headers.
    #[inline]
    pub fn header() -> Style {
        Style::default()
            .fg(TEXT_BRIGHT)
            .add_modifier(Modifier::BOLD)
    }

    /// Selected item style: near-black text on solid brand-orange background (an explicit inverse
    /// pair, never `Modifier::REVERSED`).
    #[inline]
    pub fn selected() -> Style {
        Style::default().fg(CANVAS).bg(ORANGE)
    }

    /// Error style: coral red text for error messages.
    #[inline]
    pub fn error() -> Style {
        Style::default().fg(ERROR)
    }

    /// Success style: cyan-green text for success messages.
    #[inline]
    pub fn success() -> Style {
        Style::default().fg(SUCCESS)
    }

    /// Warning style: golden text for warnings.
    #[inline]
    pub fn warning() -> Style {
        Style::default().fg(WARNING)
    }

    /// Info style: light blue text for informational messages.
    #[inline]
    pub fn info() -> Style {
        Style::default().fg(INFO)
    }

    /// Dimmed style: secondary text color.
    #[inline]
    pub fn dimmed() -> Style {
        Style::default().fg(TEXT_DIM)
    }

    /// Muted style: very dim text for background elements.
    #[inline]
    pub fn muted() -> Style {
        Style::default().fg(TEXT_MUTED)
    }

    /// Highlight style: bright bold text.
    #[inline]
    pub fn highlight() -> Style {
        Style::default().fg(HIGHLIGHT).add_modifier(Modifier::BOLD)
    }

    /// User message style: cyan primary text.
    #[inline]
    pub fn user_message() -> Style {
        Style::default().fg(CYAN_PRIMARY)
    }

    /// Assistant message style: sky blue text.
    #[inline]
    pub fn assistant_message() -> Style {
        Style::default().fg(SKY_BLUE)
    }

    /// System message style: muted italic text for informational system messages.
    #[inline]
    pub fn system_message() -> Style {
        Style::default()
            .fg(TEXT_MUTED)
            .add_modifier(Modifier::ITALIC)
    }

    /// Error message style: red italic text for backend error messages.
    #[inline]
    pub fn error_message() -> Style {
        Style::default().fg(ERROR).add_modifier(Modifier::ITALIC)
    }

    /// Code style: electric blue text on surface background.
    #[inline]
    pub fn code() -> Style {
        Style::default().fg(ELECTRIC_BLUE).bg(SURFACE_1)
    }

    /// Border style: standard border color.
    #[inline]
    pub fn border() -> Style {
        Style::default().fg(BORDER)
    }

    /// Focused border style: bright cyan border for focused elements.
    #[inline]
    pub fn border_focused() -> Style {
        Style::default().fg(BORDER_FOCUS)
    }

    /// Brain pulse style: interpolates CYAN_PRIMARY -> SKY_BLUE -> ELECTRIC_BLUE based on intensity.
    ///
    /// # Arguments
    /// * `intensity` - Value from 0.0 to 1.0 representing pulse position.
    pub fn brain_pulse(intensity: f32) -> Style {
        let color = interpolate_brain_pulse(intensity);
        Style::default().fg(color)
    }

    /// Brain cyan style: fixed cyan color with brightness variation.
    ///
    /// # Arguments
    /// * `brightness` - Value from 0.0 to 1.0 representing brightness.
    pub fn brain_cyan(brightness: f32) -> Style {
        let b = brightness.clamp(0.0, 1.0);
        let r = (0.0 * b) as u8;
        let g = (255.0 * b) as u8;
        let bl = (255.0 * b) as u8;
        Style::default().fg(Color::Rgb(r, g, bl))
    }
}

/// Interpolates between CYAN_PRIMARY -> SKY_BLUE -> ELECTRIC_BLUE based on intensity (0.0 to 1.0).
///
/// Uses linear interpolation in RGB space for smooth color transitions.
fn interpolate_brain_pulse(intensity: f32) -> Color {
    let t = intensity.clamp(0.0, 1.0);

    let (r, g, b) = if t < 0.5 {
        let local_t = t * 2.0;
        (
            lerp(0.0, 135.0, local_t),
            lerp(255.0, 206.0, local_t),
            lerp(255.0, 235.0, local_t),
        )
    } else {
        let local_t = (t - 0.5) * 2.0;
        (
            lerp(135.0, 125.0, local_t),
            lerp(206.0, 249.0, local_t),
            lerp(235.0, 255.0, local_t),
        )
    };

    Color::Rgb(r as u8, g as u8, b as u8)
}

/// Linear interpolation between two values.
#[inline]
fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CUSTOM_THEME_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn canonical_v2_tokens_locked() {
        // The mono-orange brand palette (the source of truth for the default `dark` theme).
        // Exact RGB mirror `library/design-system.md`.
        assert_eq!(CANVAS, Color::Rgb(13, 13, 13));
        assert_eq!(ORANGE, Color::Rgb(232, 146, 86));
        assert_eq!(ORANGE_D, Color::Rgb(150, 92, 54));
        assert_eq!(AMBER, Color::Rgb(242, 183, 120));
        assert_eq!(TXT, Color::Rgb(208, 208, 206));
        assert_eq!(BRIGHT, Color::Rgb(242, 242, 240));
        assert_eq!(DIM, Color::Rgb(140, 140, 140));
        assert_eq!(FAINT, Color::Rgb(92, 92, 98));
        assert_eq!(GHOST, Color::Rgb(60, 60, 66));
        assert_eq!(PANEL, Color::Rgb(22, 22, 26));
        assert_eq!(PANEL2, Color::Rgb(28, 28, 33));
        assert_eq!(BAR, Color::Rgb(20, 20, 24));
        assert_eq!(SELBG, Color::Rgb(44, 35, 26));
        assert_eq!(SOFT, Color::Rgb(34, 28, 22));
        assert_eq!(BORDER_BRAND, Color::Rgb(92, 66, 46));
        assert_eq!(OKG, Color::Rgb(96, 204, 110));
        assert_eq!(ERR, Color::Rgb(224, 96, 96));
        assert_eq!(ADD_FG, Color::Rgb(124, 196, 124));
        assert_eq!(ADD_BG, Color::Rgb(18, 38, 22));
        assert_eq!(DEL_FG, Color::Rgb(212, 118, 108));
        assert_eq!(DEL_BG, Color::Rgb(46, 22, 22));
        assert_eq!(LNUM, Color::Rgb(110, 110, 118));
        assert_eq!(CODEBG, Color::Rgb(24, 24, 28));
        assert_eq!(INLBG, Color::Rgb(40, 34, 28));
    }

    #[test]
    fn dark_theme_is_mono_orange() {
        let t = ThemeColors::dark();
        assert_eq!(t.background, CANVAS, "near-black #0d0d0d canvas");
        assert_eq!(t.primary, ORANGE, "primary accent is brand ORANGE");
        assert_eq!(t.secondary, AMBER);
        assert_eq!(t.accent, AMBER);
        assert_eq!(t.text, TXT);
        assert_eq!(t.text_bright, BRIGHT);
        assert_eq!(t.text_dim, DIM);
        assert_eq!(t.text_muted, FAINT);
        assert_eq!(t.ghost, GHOST);
        assert_eq!(t.border, ORANGE_D);
        assert_eq!(t.card_border, BORDER_BRAND);
        assert_eq!(t.soft, SOFT, "working-band soft surface is the brand SOFT tint");
        assert_eq!(t.success, OKG);
        assert_eq!(t.error, ERR);
        // `from_name("dark")` and the Default both resolve to the brand palette.
        assert_eq!(ThemeColors::from_name("dark"), t);
        assert_eq!(ThemeColors::default(), t);
    }

    #[test]
    fn selection_is_explicit_inverse_pair() {
        // Selection is a solid inverse pair (near-black text on solid ORANGE), never REVERSED.
        let t = ThemeColors::dark();
        assert_eq!(t.selection_bg, ORANGE);
        assert_eq!(t.selection_fg, CANVAS);
        assert_ne!(
            t.selection_fg, t.selection_bg,
            "fg and bg of a selection differ (a real inverse pair)"
        );
        // Every theme defines a distinct selection pair.
        for theme in [
            ThemeColors::dark(),
            ThemeColors::light(),
            ThemeColors::monokai(),
            ThemeColors::ocean_dark(),
            ThemeColors::vscode(),
        ] {
            assert_ne!(theme.selection_fg, theme.selection_bg);
        }
    }

    #[test]
    fn all_five_themes_resolve_with_new_tokens() {
        let _guard = CUSTOM_THEME_TEST_LOCK.lock().expect("theme test lock");
        ThemeColors::clear_custom_themes();
        for name in ["dark", "light", "monokai", "ocean", "vscode"] {
            let t = ThemeColors::from_name(name);
            // The four new tokens are always populated (no field falls back to a default-black).
            assert_ne!(t.text_bright, t.background, "{name}: text_bright is set");
            assert_ne!(
                t.selection_fg, t.selection_bg,
                "{name}: selection pair is a real inverse"
            );
            // ghost is a real, distinct neutral tone.
            let _ = t.ghost;
            // The working-band soft surface is an explicit non-canvas tint (no black-bar fallthrough).
            assert_ne!(t.soft, t.background, "{name}: soft band surface is set");
        }
        // `vscode` resolves to full chrome (a distinct background), not just a syntax theme.
        assert_eq!(
            ThemeColors::from_name("vscode").background,
            Color::Rgb(30, 30, 30)
        );
    }

    #[test]
    fn test_brain_pulse_at_boundaries() {
        let color = interpolate_brain_pulse(0.0);
        assert_eq!(color, Color::Rgb(0, 255, 255));

        let color = interpolate_brain_pulse(1.0);
        assert_eq!(color, Color::Rgb(125, 249, 255));

        let color = interpolate_brain_pulse(0.5);
        assert_eq!(color, Color::Rgb(135, 206, 235));
    }

    #[test]
    fn test_lerp() {
        assert_eq!(lerp(0.0, 100.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 100.0, 0.5), 50.0);
        assert_eq!(lerp(0.0, 100.0, 1.0), 100.0);
    }

    #[test]
    fn test_theme_colors() {
        let theme = ThemeColors::dark();
        assert_eq!(theme.primary, ORANGE);
        assert_eq!(theme.background, CANVAS);
        assert_eq!(theme.surface[1], PANEL);
        assert_eq!(theme.border_focus, ORANGE);
        assert_eq!(ThemeColors::default().primary, theme.primary);
    }

    #[test]
    fn test_theme_from_name() {
        let _guard = CUSTOM_THEME_TEST_LOCK.lock().expect("theme test lock");
        ThemeColors::clear_custom_themes();
        assert_eq!(
            ThemeColors::from_name("light").background,
            Color::Rgb(255, 255, 255)
        );
        assert_eq!(
            ThemeColors::from_name("monokai").primary,
            Color::Rgb(166, 226, 46)
        );
        assert_eq!(
            ThemeColors::from_name("ocean").background,
            Color::Rgb(10, 25, 47)
        );
        assert_eq!(
            ThemeColors::from_name("vscode").primary,
            Color::Rgb(0, 122, 204)
        );
        // The `vscode-dark` alias resolves to the same chrome theme.
        assert_eq!(
            ThemeColors::from_name("vscode-dark"),
            ThemeColors::from_name("vscode")
        );
        // Unknown (and "dark") falls back to the mono-orange brand.
        assert_eq!(
            ThemeColors::from_name("nonexistent").primary,
            ThemeColors::dark().primary
        );
    }

    #[test]
    fn test_available_themes() {
        let _guard = CUSTOM_THEME_TEST_LOCK.lock().expect("theme test lock");
        ThemeColors::clear_custom_themes();
        let themes = ThemeColors::available_themes();
        assert_eq!(themes, &["dark", "light", "monokai", "ocean", "vscode"]);
        // `dark` is first so it is the built-in default.
        assert_eq!(themes.first(), Some(&"dark"));
    }

    #[test]
    fn custom_theme_registration_round_trips() {
        let _guard = CUSTOM_THEME_TEST_LOCK.lock().expect("theme test lock");
        ThemeColors::clear_custom_themes();
        let mut custom = ThemeColors::dark();
        custom.primary = Color::Rgb(255, 0, 255);
        ThemeColors::register_custom_theme("nebula_custom", custom.clone());

        let names = ThemeColors::available_theme_names();
        assert!(
            names.iter().any(|name| name == "nebula_custom"),
            "custom theme appears in the theme catalogue"
        );
        assert_eq!(
            ThemeColors::from_name("nebula_custom").primary,
            custom.primary,
            "lookup resolves to the registered runtime palette"
        );
        ThemeColors::clear_custom_themes();
    }
}
