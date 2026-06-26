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
//! A cohesive ocean-inspired theme with cyan/green accents. All colors are constants. These
//! constants are the single source of truth for the finalized single theme consumed by both the
//! markdown renderer (`frame::markdown`) and the ported `tui-components` widgets.

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
// LEGACY ALIASES - Backward compatibility
// ============================================================

/// Alias for CYAN_PRIMARY (legacy: PINK).
pub const PINK: Color = CYAN_PRIMARY;

/// Alias for TEAL (legacy: PURPLE).
pub const PURPLE: Color = TEAL;

/// Alias for SUCCESS (legacy: GREEN).
pub const GREEN: Color = SUCCESS;

/// Alias for WARNING (legacy: ORANGE).
pub const ORANGE: Color = WARNING;

/// Alias for INFO (legacy: BLUE).
pub const BLUE: Color = INFO;

/// Alias for ERROR (legacy: RED).
pub const RED: Color = ERROR;

/// Alias for HIGHLIGHT (legacy: YELLOW).
pub const YELLOW: Color = HIGHLIGHT;

/// Alias for BORDER_FOCUS (legacy: BORDER_HIGHLIGHT).
pub const BORDER_HIGHLIGHT: Color = BORDER_FOCUS;

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
}

impl ThemeColors {
    /// Ocean/Cyan theme - the default Cortex theme.
    pub fn ocean_cyan() -> Self {
        Self {
            primary: CYAN_PRIMARY,
            secondary: SKY_BLUE,
            accent: ELECTRIC_BLUE,
            background: VOID,
            surface: [SURFACE_0, SURFACE_1, SURFACE_2, SURFACE_3],
            text: TEXT,
            text_dim: TEXT_DIM,
            text_muted: TEXT_MUTED,
            success: SUCCESS,
            warning: WARNING,
            error: ERROR,
            info: INFO,
            border: BORDER,
            border_focus: BORDER_FOCUS,
        }
    }

    /// Dark theme (default) - cyan/green accent on dark background.
    pub fn dark() -> Self {
        Self::ocean_cyan()
    }

    /// Light theme - darker accents on light background.
    pub fn light() -> Self {
        Self {
            primary: Color::Rgb(0, 150, 100),
            secondary: Color::Rgb(0, 120, 80),
            accent: Color::Rgb(0, 100, 70),
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
            success: Color::Rgb(0, 150, 0),
            warning: Color::Rgb(200, 150, 0),
            error: Color::Rgb(200, 50, 50),
            info: Color::Rgb(50, 100, 200),
            border: Color::Rgb(200, 200, 200),
            border_focus: Color::Rgb(0, 150, 100),
        }
    }

    /// Ocean dark theme - deep blue/cyan aesthetic.
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
            success: Color::Rgb(0, 220, 180),
            warning: Color::Rgb(255, 200, 100),
            error: Color::Rgb(255, 100, 100),
            info: Color::Rgb(100, 180, 255),
            border: Color::Rgb(40, 80, 120),
            border_focus: Color::Rgb(0, 200, 200),
        }
    }

    /// Monokai theme - classic code editor colors.
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
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(230, 219, 116),
            error: Color::Rgb(249, 38, 114),
            info: Color::Rgb(102, 217, 239),
            border: Color::Rgb(70, 71, 65),
            border_focus: Color::Rgb(166, 226, 46),
        }
    }

    /// Dracula-inspired theme.
    pub fn dracula() -> Self {
        Self {
            primary: Color::Rgb(189, 147, 249),
            secondary: Color::Rgb(139, 233, 253),
            accent: Color::Rgb(255, 121, 198),
            background: Color::Rgb(40, 42, 54),
            surface: [
                Color::Rgb(45, 48, 62),
                Color::Rgb(53, 57, 75),
                Color::Rgb(64, 69, 90),
                Color::Rgb(76, 82, 106),
            ],
            text: Color::Rgb(248, 248, 242),
            text_dim: Color::Rgb(189, 192, 210),
            text_muted: Color::Rgb(98, 114, 164),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(241, 250, 140),
            error: Color::Rgb(255, 85, 85),
            info: Color::Rgb(139, 233, 253),
            border: Color::Rgb(98, 114, 164),
            border_focus: Color::Rgb(189, 147, 249),
        }
    }

    /// Nord-inspired dark theme.
    pub fn nord() -> Self {
        Self {
            primary: Color::Rgb(143, 188, 187),
            secondary: Color::Rgb(136, 192, 208),
            accent: Color::Rgb(129, 161, 193),
            background: Color::Rgb(46, 52, 64),
            surface: [
                Color::Rgb(59, 66, 82),
                Color::Rgb(67, 76, 94),
                Color::Rgb(76, 86, 106),
                Color::Rgb(94, 103, 124),
            ],
            text: Color::Rgb(236, 239, 244),
            text_dim: Color::Rgb(216, 222, 233),
            text_muted: Color::Rgb(129, 161, 193),
            success: Color::Rgb(163, 190, 140),
            warning: Color::Rgb(235, 203, 139),
            error: Color::Rgb(191, 97, 106),
            info: Color::Rgb(136, 192, 208),
            border: Color::Rgb(76, 86, 106),
            border_focus: Color::Rgb(143, 188, 187),
        }
    }

    /// Solarized-inspired dark theme.
    pub fn solarized_dark() -> Self {
        Self {
            primary: Color::Rgb(38, 139, 210),
            secondary: Color::Rgb(42, 161, 152),
            accent: Color::Rgb(108, 113, 196),
            background: Color::Rgb(0, 43, 54),
            surface: [
                Color::Rgb(7, 54, 66),
                Color::Rgb(15, 63, 76),
                Color::Rgb(28, 74, 89),
                Color::Rgb(39, 86, 102),
            ],
            text: Color::Rgb(131, 148, 150),
            text_dim: Color::Rgb(101, 123, 131),
            text_muted: Color::Rgb(88, 110, 117),
            success: Color::Rgb(133, 153, 0),
            warning: Color::Rgb(181, 137, 0),
            error: Color::Rgb(220, 50, 47),
            info: Color::Rgb(38, 139, 210),
            border: Color::Rgb(88, 110, 117),
            border_focus: Color::Rgb(42, 161, 152),
        }
    }

    /// Gruvbox-inspired dark theme.
    pub fn gruvbox_dark() -> Self {
        Self {
            primary: Color::Rgb(250, 189, 47),
            secondary: Color::Rgb(131, 165, 152),
            accent: Color::Rgb(211, 134, 155),
            background: Color::Rgb(40, 40, 40),
            surface: [
                Color::Rgb(50, 48, 47),
                Color::Rgb(60, 56, 54),
                Color::Rgb(80, 73, 69),
                Color::Rgb(102, 92, 84),
            ],
            text: Color::Rgb(235, 219, 178),
            text_dim: Color::Rgb(213, 196, 161),
            text_muted: Color::Rgb(146, 131, 116),
            success: Color::Rgb(184, 187, 38),
            warning: Color::Rgb(250, 189, 47),
            error: Color::Rgb(251, 73, 52),
            info: Color::Rgb(131, 165, 152),
            border: Color::Rgb(102, 92, 84),
            border_focus: Color::Rgb(250, 189, 47),
        }
    }

    /// Tokyo Night-inspired dark theme.
    pub fn tokyo_night() -> Self {
        Self {
            primary: Color::Rgb(122, 162, 247),
            secondary: Color::Rgb(125, 207, 255),
            accent: Color::Rgb(187, 154, 247),
            background: Color::Rgb(26, 27, 38),
            surface: [
                Color::Rgb(31, 34, 50),
                Color::Rgb(36, 40, 59),
                Color::Rgb(52, 59, 88),
                Color::Rgb(73, 83, 130),
            ],
            text: Color::Rgb(192, 202, 245),
            text_dim: Color::Rgb(169, 177, 214),
            text_muted: Color::Rgb(86, 95, 137),
            success: Color::Rgb(158, 206, 106),
            warning: Color::Rgb(224, 175, 104),
            error: Color::Rgb(247, 118, 142),
            info: Color::Rgb(122, 162, 247),
            border: Color::Rgb(65, 72, 104),
            border_focus: Color::Rgb(125, 207, 255),
        }
    }

    /// Catppuccin Mocha-inspired dark theme.
    pub fn catppuccin_mocha() -> Self {
        Self {
            primary: Color::Rgb(203, 166, 247),
            secondary: Color::Rgb(137, 220, 235),
            accent: Color::Rgb(245, 194, 231),
            background: Color::Rgb(30, 30, 46),
            surface: [
                Color::Rgb(49, 50, 68),
                Color::Rgb(69, 71, 90),
                Color::Rgb(88, 91, 112),
                Color::Rgb(108, 112, 134),
            ],
            text: Color::Rgb(205, 214, 244),
            text_dim: Color::Rgb(186, 194, 222),
            text_muted: Color::Rgb(127, 132, 156),
            success: Color::Rgb(166, 227, 161),
            warning: Color::Rgb(249, 226, 175),
            error: Color::Rgb(243, 139, 168),
            info: Color::Rgb(137, 220, 235),
            border: Color::Rgb(108, 112, 134),
            border_focus: Color::Rgb(203, 166, 247),
        }
    }

    /// One Dark-inspired theme.
    pub fn one_dark() -> Self {
        Self {
            primary: Color::Rgb(97, 175, 239),
            secondary: Color::Rgb(86, 182, 194),
            accent: Color::Rgb(198, 120, 221),
            background: Color::Rgb(40, 44, 52),
            surface: [
                Color::Rgb(44, 49, 60),
                Color::Rgb(50, 55, 67),
                Color::Rgb(58, 63, 77),
                Color::Rgb(72, 78, 94),
            ],
            text: Color::Rgb(171, 178, 191),
            text_dim: Color::Rgb(146, 153, 166),
            text_muted: Color::Rgb(92, 99, 112),
            success: Color::Rgb(152, 195, 121),
            warning: Color::Rgb(229, 192, 123),
            error: Color::Rgb(224, 108, 117),
            info: Color::Rgb(97, 175, 239),
            border: Color::Rgb(72, 78, 94),
            border_focus: Color::Rgb(198, 120, 221),
        }
    }

    /// Rose Pine-inspired dark theme.
    pub fn rose_pine() -> Self {
        Self {
            primary: Color::Rgb(196, 167, 231),
            secondary: Color::Rgb(156, 207, 216),
            accent: Color::Rgb(235, 188, 186),
            background: Color::Rgb(25, 23, 36),
            surface: [
                Color::Rgb(31, 29, 46),
                Color::Rgb(38, 35, 58),
                Color::Rgb(49, 44, 63),
                Color::Rgb(64, 58, 82),
            ],
            text: Color::Rgb(224, 222, 244),
            text_dim: Color::Rgb(144, 140, 170),
            text_muted: Color::Rgb(110, 106, 134),
            success: Color::Rgb(49, 116, 143),
            warning: Color::Rgb(246, 193, 119),
            error: Color::Rgb(235, 111, 146),
            info: Color::Rgb(156, 207, 216),
            border: Color::Rgb(64, 58, 82),
            border_focus: Color::Rgb(196, 167, 231),
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
            "ocean_dark" | "ocean" => Self::ocean_dark(),
            "monokai" => Self::monokai(),
            "dracula" => Self::dracula(),
            "nord" => Self::nord(),
            "solarized_dark" | "solarized" => Self::solarized_dark(),
            "gruvbox_dark" | "gruvbox" => Self::gruvbox_dark(),
            "tokyo_night" | "tokyonight" => Self::tokyo_night(),
            "catppuccin_mocha" | "catppuccin" => Self::catppuccin_mocha(),
            "one_dark" | "onedark" => Self::one_dark(),
            "rose_pine" | "rosepine" => Self::rose_pine(),
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

const BUILTIN_THEME_NAMES: &[&str] = &[
    "dark",
    "light",
    "ocean_dark",
    "monokai",
    "dracula",
    "nord",
    "solarized_dark",
    "gruvbox_dark",
    "tokyo_night",
    "catppuccin_mocha",
    "one_dark",
    "rose_pine",
];

static CUSTOM_THEMES: Lazy<RwLock<BTreeMap<String, ThemeColors>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));

impl Default for ThemeColors {
    fn default() -> Self {
        Self::ocean_cyan()
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

    /// Selected item style: void text on cyan background.
    #[inline]
    pub fn selected() -> Style {
        Style::default().fg(VOID).bg(CYAN_PRIMARY)
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
        assert_eq!(VOID, Color::Rgb(10, 22, 40));
        assert_eq!(BORDER, Color::Rgb(27, 73, 101));
        assert_eq!(INFO, Color::Rgb(72, 202, 228));
        assert_eq!(CYAN_PRIMARY, Color::Rgb(0, 255, 163));
        assert_eq!(SURFACE_0, Color::Rgb(13, 27, 42));
        assert_eq!(TEXT, Color::Rgb(255, 255, 255));
        assert_eq!(TEXT_DIM, Color::Rgb(130, 154, 177));
        assert_eq!(ERROR, Color::Rgb(255, 107, 107));
        assert_eq!(SUCCESS, Color::Rgb(0, 245, 212));
        assert_eq!(WARNING, Color::Rgb(255, 200, 87));
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
    fn test_legacy_aliases() {
        assert_eq!(PINK, CYAN_PRIMARY);
        assert_eq!(PURPLE, TEAL);
        assert_eq!(GREEN, SUCCESS);
        assert_eq!(ORANGE, WARNING);
        assert_eq!(BLUE, INFO);
        assert_eq!(RED, ERROR);
        assert_eq!(YELLOW, HIGHLIGHT);
        assert_eq!(BORDER_HIGHLIGHT, BORDER_FOCUS);
    }

    #[test]
    fn test_theme_colors() {
        let theme = ThemeColors::ocean_cyan();
        assert_eq!(theme.primary, CYAN_PRIMARY);
        assert_eq!(theme.background, VOID);
        assert_eq!(theme.surface[2], SURFACE_2);
        assert_eq!(theme.border_focus, BORDER_FOCUS);
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
            ThemeColors::from_name("dracula").background,
            Color::Rgb(40, 42, 54)
        );
        // Unknown falls back to dark.
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
        assert_eq!(themes.len(), 12);
        assert!(themes.contains(&"monokai"));
        assert!(themes.contains(&"catppuccin_mocha"));
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
