//! TTY-gated ANSI styling for the headless CLI surface.
//!
//! Headless commands emit plain, line-oriented bytes. To match the Wave 9 mockups the *human*
//! text branch of `run`/`stats`/`search`/`models`/`replay`/`--help` and the `serve` banner may be
//! styled with the mono-orange brand palette + safe glyphs. That styling is applied ONLY through a
//! [`Styler`] whose escape sequences collapse to empty strings unless the target stream is a real
//! terminal and color is not suppressed.
//!
//! Two hard invariants make this safe for machine consumers:
//! - **Auto-disable off a TTY.** When stdout is piped/redirected the [`Styler`] is disabled, so no
//!   ANSI escape byte is ever emitted (VAL-CLI-001).
//! - **`NO_COLOR` honored.** When `NO_COLOR` is present and non-empty the [`Styler`] is disabled
//!   even on a TTY, matching the informal `NO_COLOR` standard (VAL-CLI-002).
//!
//! The machine formats (`--format json`, `--jsonl`) never route through this module, so their
//! bytes stay byte-identical regardless of TTY-ness or `NO_COLOR` (VAL-CLI-003 / byte-stability).
//! This crate cannot import the private `tui::glyphs` inventory, so the safe glyphs it needs live
//! here as the headless surface's own source of truth, guarded by the same forbidden-set tests.

// The palette + glyph inventory is the shared foundation for every Wave 9 command; the sibling
// features (`run`/`stats`/`search`/`models`/`replay`/`help`/`serve`) consume the remaining
// constants, so some are not yet referenced by this first foundation feature.
#![allow(dead_code)]

use std::io::IsTerminal;

/// A 24-bit truecolor value, emitted as an SGR `38;2;r;g;b` (fg) / `48;2;r;g;b` (bg) sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

// --- Brand palette (mono-orange on near-black; exact RGB from library/design-system.md §1.2) ---

/// Primary brand accent (prompts, marks, headers).
pub const ORANGE: Rgb = Rgb(232, 146, 86);
/// Dim orange: borders, secondary marks.
pub const ORANGE_D: Rgb = Rgb(150, 92, 54);
/// Lighter accent: titles, highlights, cost.
pub const AMBER: Rgb = Rgb(242, 183, 120);
/// Body text.
pub const TXT: Rgb = Rgb(208, 208, 206);
/// Emphasized / selected text.
pub const BRIGHT: Rgb = Rgb(242, 242, 240);
/// Secondary text.
pub const DIM: Rgb = Rgb(140, 140, 140);
/// Tertiary text / placeholders / hints.
pub const FAINT: Rgb = Rgb(92, 92, 98);
/// Rules / dividers / faded backdrop.
pub const GHOST: Rgb = Rgb(60, 60, 66);
/// Near-black page background.
pub const PAGE: Rgb = Rgb(13, 13, 13);

// --- Status / semantic accents (the only sanctioned non-orange hues) ---

/// Success green: HTTP 2xx/3xx, `finish_reason`/`done` == "stop". Never a plain data number.
pub const OK_GREEN: Rgb = Rgb(96, 204, 110);
/// Error red: HTTP >=4xx, failures.
pub const ERR_RED: Rgb = Rgb(224, 96, 96);
/// Link-ish blue for URLs/paths in serve logs.
pub const URL_BLUE: Rgb = Rgb(120, 190, 224);
/// Search-hit highlight background (warm tint).
pub const HIT_BG: Rgb = Rgb(70, 56, 30);

// --- VS Code Dark+ JSON syntax palette (documented exception; used by run json/jsonl styling) ---

/// JSON key (light blue).
pub const SYNTAX_KEY: Rgb = Rgb(156, 220, 254);
/// JSON string value (orange-brown).
pub const SYNTAX_STRING: Rgb = Rgb(206, 145, 120);
/// JSON number (green).
pub const SYNTAX_NUMBER: Rgb = Rgb(181, 206, 168);
/// JSON keyword literal (`true`/`false`/`null`, blue).
pub const SYNTAX_KEYWORD: Rgb = Rgb(86, 156, 214);
/// JSON punctuation / operators.
pub const SYNTAX_PUNCT: Rgb = Rgb(212, 212, 212);

// --- Safe glyph inventory (DejaVu Sans Mono verified; the headless surface's source of truth) ---

/// Brand star / assistant author mark.
pub const BRAND_STAR: &str = "✦";
/// Prompt / user mark.
pub const PROMPT: &str = "›";
/// Shell prompt mark used by the serve/prompt banners.
pub const SHELL_PROMPT: &str = "$";
/// Faint inline separator.
pub const SEPARATOR: &str = "·";
/// Em dash (detail separator / "not applicable").
pub const EM_DASH: &str = "—";
/// Ellipsis.
pub const ELLIPSIS: &str = "…";
/// Filled status dot.
pub const DOT_FILLED: &str = "●";
/// Hollow status dot.
pub const DOT_HOLLOW: &str = "○";
/// Diamond status/severity marker.
pub const DOT_DIAMOND: &str = "◆";
/// Done / ok mark.
pub const MARK_DONE: &str = "✓";
/// Error mark.
pub const MARK_ERROR: &str = "✗";
/// Running-tool mark.
pub const MARK_RUNNING: &str = "⚙";

// Box drawing (rounded only).
/// Top-left rounded corner.
pub const BOX_TOP_LEFT: &str = "╭";
/// Top-right rounded corner.
pub const BOX_TOP_RIGHT: &str = "╮";
/// Bottom-left rounded corner.
pub const BOX_BOTTOM_LEFT: &str = "╰";
/// Bottom-right rounded corner.
pub const BOX_BOTTOM_RIGHT: &str = "╯";
/// Horizontal rule.
pub const BOX_HORIZONTAL: &str = "─";
/// Vertical edge.
pub const BOX_VERTICAL: &str = "│";

/// The ANSI reset sequence (all attributes off).
const RESET: &str = "\x1b[0m";

/// A composable text style: optional truecolor foreground/background plus a bold attribute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    fg: Option<Rgb>,
    bg: Option<Rgb>,
    bold: bool,
}

impl Style {
    /// A foreground-only style.
    #[must_use]
    pub const fn fg(color: Rgb) -> Self {
        Self {
            fg: Some(color),
            bg: None,
            bold: false,
        }
    }

    /// Add a background fill.
    #[must_use]
    pub const fn on(mut self, color: Rgb) -> Self {
        self.bg = Some(color);
        self
    }

    /// Mark the style bold.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// The SGR opening sequence for this style (empty when the style is a no-op).
    fn open_sequence(self) -> String {
        let mut params: Vec<String> = Vec::new();
        if self.bold {
            params.push("1".to_owned());
        }
        if let Some(Rgb(r, g, b)) = self.fg {
            params.push(format!("38;2;{r};{g};{b}"));
        }
        if let Some(Rgb(r, g, b)) = self.bg {
            params.push(format!("48;2;{r};{g};{b}"));
        }
        if params.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", params.join(";"))
        }
    }
}

/// Decide whether styling should be applied for a stream.
///
/// Styling is on only when the stream is a TTY AND color is not suppressed. This is the single
/// gate every headless styling decision flows through, kept as a free function so it is trivially
/// unit-testable across the full `(is_tty, no_color)` matrix.
#[must_use]
pub fn should_style(is_tty: bool, no_color: bool) -> bool {
    is_tty && !no_color
}

/// Whether a raw `NO_COLOR` value suppresses color.
///
/// Follows the informal `NO_COLOR` standard: color is disabled when the variable is present and
/// non-empty, regardless of its value; an empty value (or an absent variable) does not disable
/// color. Kept pure (takes the resolved value) so it is testable without mutating the environment.
#[must_use]
pub fn no_color_from_value(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Whether the process `NO_COLOR` environment variable suppresses color.
#[must_use]
pub fn no_color_from_env() -> bool {
    no_color_from_value(std::env::var_os("NO_COLOR").as_deref())
}

/// A stream-scoped styler. When disabled, every styling method returns its input unchanged, so no
/// ANSI escape byte can reach a piped/redirected consumer.
#[derive(Debug, Clone, Copy)]
pub struct Styler {
    enabled: bool,
}

impl Styler {
    /// A styler with an explicit enabled flag (used for tests and for callers that already know
    /// the stream disposition).
    #[must_use]
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// A styler gated on the real `stdout` stream: enabled only when stdout is a TTY and
    /// `NO_COLOR` does not suppress color.
    #[must_use]
    pub fn for_stdout() -> Self {
        Self::new(should_style(
            std::io::stdout().is_terminal(),
            no_color_from_env(),
        ))
    }

    /// A styler gated on the real `stderr` stream (used by the `serve` banner, which prints to
    /// stderr so stdout stays clean for machine consumers).
    #[must_use]
    pub fn for_stderr() -> Self {
        Self::new(should_style(
            std::io::stderr().is_terminal(),
            no_color_from_env(),
        ))
    }

    /// Whether this styler emits ANSI.
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        self.enabled
    }

    /// Wrap `text` in `style`'s SGR sequence, or return it unchanged when styling is disabled or
    /// the style is a no-op.
    #[must_use]
    pub fn apply(self, text: &str, style: Style) -> String {
        if !self.enabled {
            return text.to_owned();
        }
        let open = style.open_sequence();
        if open.is_empty() {
            return text.to_owned();
        }
        format!("{open}{text}{RESET}")
    }

    /// Convenience: paint `text` in a foreground color.
    #[must_use]
    pub fn paint(self, text: &str, color: Rgb) -> String {
        self.apply(text, Style::fg(color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_style_only_on_a_tty_without_no_color() {
        assert!(should_style(true, false), "TTY + color allowed → styled");
        assert!(!should_style(false, false), "pipe → never styled");
        assert!(!should_style(true, true), "NO_COLOR on a TTY → not styled");
        assert!(!should_style(false, true), "pipe + NO_COLOR → not styled");
    }

    #[test]
    fn disabled_styler_is_a_pure_passthrough_with_zero_ansi() {
        let plain = Styler::new(false);
        let out = plain.paint("hello", ORANGE);
        assert_eq!(out, "hello", "disabled styler must not alter the text");
        assert!(
            !out.contains('\x1b'),
            "disabled styler must emit no ESC byte"
        );
        let boxed = plain.apply("x", Style::fg(AMBER).on(PAGE).bold());
        assert_eq!(boxed, "x");
        assert!(!boxed.contains('\x1b'));
    }

    #[test]
    fn enabled_styler_emits_truecolor_fg_and_reset() {
        let styled = Styler::new(true);
        let out = styled.paint("hi", ORANGE);
        assert_eq!(
            out, "\x1b[38;2;232;146;86mhi\x1b[0m",
            "foreground must be a 24-bit truecolor SGR wrapped by a reset"
        );
    }

    #[test]
    fn enabled_styler_composes_bold_fg_and_bg_in_order() {
        let styled = Styler::new(true);
        let out = styled.apply("t", Style::fg(BRIGHT).on(GHOST).bold());
        assert_eq!(
            out, "\x1b[1;38;2;242;242;240;48;2;60;60;66mt\x1b[0m",
            "bold precedes fg precedes bg, all truecolor, one reset"
        );
    }

    #[test]
    fn empty_style_is_a_noop_even_when_enabled() {
        let styled = Styler::new(true);
        assert_eq!(styled.apply("t", Style::default()), "t");
    }

    #[test]
    fn no_color_env_follows_the_informal_standard() {
        use std::ffi::OsStr;
        // Present + non-empty disables; empty or absent does not. Pure helper → no env mutation.
        assert!(
            no_color_from_value(Some(OsStr::new("1"))),
            "NO_COLOR=1 disables color"
        );
        assert!(
            no_color_from_value(Some(OsStr::new("anything"))),
            "any non-empty NO_COLOR disables color"
        );
        assert!(
            !no_color_from_value(Some(OsStr::new(""))),
            "empty NO_COLOR does not disable color"
        );
        assert!(
            !no_color_from_value(None),
            "unset NO_COLOR does not disable color"
        );
    }

    /// Every glyph this module exposes, for whole-inventory safety assertions.
    fn glyph_inventory() -> Vec<&'static str> {
        vec![
            BRAND_STAR,
            PROMPT,
            SHELL_PROMPT,
            SEPARATOR,
            EM_DASH,
            ELLIPSIS,
            DOT_FILLED,
            DOT_HOLLOW,
            DOT_DIAMOND,
            MARK_DONE,
            MARK_ERROR,
            MARK_RUNNING,
            BOX_TOP_LEFT,
            BOX_TOP_RIGHT,
            BOX_BOTTOM_LEFT,
            BOX_BOTTOM_RIGHT,
            BOX_HORIZONTAL,
            BOX_VERTICAL,
        ]
    }

    #[test]
    fn glyphs_are_brand_safe_no_braille_no_forbidden() {
        let forbidden = ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'];
        for glyph in glyph_inventory() {
            for c in glyph.chars() {
                assert!(
                    !('\u{2800}'..='\u{28FF}').contains(&c),
                    "braille codepoint {c:?} in glyph {glyph:?}"
                );
                assert!(
                    !forbidden.contains(&c),
                    "forbidden glyph {c:?} in {glyph:?}"
                );
                assert!(
                    !('\u{1F000}'..='\u{1FAFF}').contains(&c),
                    "emoji codepoint {c:?} in {glyph:?}"
                );
            }
        }
        let joined: String = glyph_inventory().concat();
        assert!(!joined.contains(">_"), "the `>_` cliché must never appear");
        // The brand mark must be the star, never the terminal cliché.
        assert_eq!(BRAND_STAR, "✦");
    }

    #[test]
    fn brand_palette_matches_the_locked_rgb_tokens() {
        assert_eq!(ORANGE, Rgb(232, 146, 86));
        assert_eq!(ORANGE_D, Rgb(150, 92, 54));
        assert_eq!(AMBER, Rgb(242, 183, 120));
        assert_eq!(PAGE, Rgb(13, 13, 13));
        assert_eq!(OK_GREEN, Rgb(96, 204, 110));
        assert_eq!(ERR_RED, Rgb(224, 96, 96));
    }
}
