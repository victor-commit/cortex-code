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

    /// Get a theme by name.
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "light" => Self::light(),
            "ocean_dark" | "ocean" => Self::ocean_dark(),
            "monokai" => Self::monokai(),
            _ => Self::dark(),
        }
    }

    /// Get all available theme names.
    pub fn available_themes() -> &'static [&'static str] {
        &["dark", "light", "ocean_dark", "monokai"]
    }
}

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
        assert_eq!(
            ThemeColors::from_name("light").background,
            Color::Rgb(255, 255, 255)
        );
        assert_eq!(
            ThemeColors::from_name("monokai").primary,
            Color::Rgb(166, 226, 46)
        );
        // Unknown falls back to dark.
        assert_eq!(
            ThemeColors::from_name("nonexistent").primary,
            ThemeColors::dark().primary
        );
    }

    #[test]
    fn test_available_themes() {
        let themes = ThemeColors::available_themes();
        assert_eq!(themes.len(), 4);
        assert!(themes.contains(&"monokai"));
    }
}
