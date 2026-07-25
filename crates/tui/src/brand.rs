//! Brand assets shared by the home/MOTD and onboarding surfaces: the neutral `✦ Cortex` wordmark
//! lockup and the brain mascot block-art.
//!
//! Both are rendered through theme tokens (never hardcoded RGB) so they follow the active chrome
//! theme, and both draw only safe glyphs: the wordmark uses the `✦` brand star (never the `>_`
//! cliché) and the mascot is built from the safe block set `█ ▄ ▀ ▌ ▐`, so it renders mono-orange
//! with zero tofu. The mascot art is sourced from the shared `tui-components` widget, wiring that
//! previously-dead asset into the live render path for later milestones (M1 MOTD, M9 onboarding).
//!
//! Wave-2 retirement (VAL-CORTEX-UX-002): primary plates use the short shell identity `Cortex`,
//! never the copper product compound `Cortex Code`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tui_components::mascot::MASCOT_MINIMAL_LINES;
use tui_components::style::ThemeColors;

use crate::glyphs;

/// The product name shown in the brand wordmark (neutral shell identity).
///
/// Must not be the retired copper compound `Cortex Code` (VAL-CORTEX-UX-002).
pub const WORDMARK_NAME: &str = "Cortex";

/// The brain mascot block-art (4 lines, safe blocks only), shared with the `tui-components`
/// `WelcomeCard`/mascot widgets so home and onboarding draw the identical art.
pub const MASCOT_LINES: [&str; 4] = MASCOT_MINIMAL_LINES;

/// Build the `✦ Cortex` brand lockup as one styled line: the `✦` star in the brand accent
/// (ORANGE on the default theme) and the product name in the bright text tone.
#[must_use]
pub fn wordmark(theme: &ThemeColors) -> Line<'static> {
    Line::from(wordmark_spans(theme))
}

/// The spans of the brand lockup, for callers that compose the wordmark into a larger line
/// (e.g. a title-in-border with a trailing version).
#[must_use]
pub fn wordmark_spans(theme: &ThemeColors) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            glyphs::BRAND_STAR,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            WORDMARK_NAME,
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ]
}

/// Build the brain mascot block-art as styled lines in the brand accent (ORANGE on the default
/// theme), ready to stack onto a card. Each line keeps its fixed width so the rows stay aligned.
#[must_use]
pub fn mascot_lines(theme: &ThemeColors) -> Vec<Line<'static>> {
    let style = Style::default().fg(theme.primary);
    MASCOT_LINES
        .iter()
        .map(|line| Line::from(Span::styled((*line).to_string(), style)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only glyphs allowed in the mascot art: a space plus the safe block set `█ ▄ ▀ ▌ ▐`.
    const SAFE_BLOCKS: [char; 6] = [' ', '█', '▄', '▀', '▌', '▐'];

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wordmark_is_the_star_lockup_never_the_cliche() {
        let theme = ThemeColors::dark();
        let text = plain(&wordmark(&theme));
        assert!(text.contains('✦'), "the brand star is present: {text:?}");
        assert!(
            text.contains(WORDMARK_NAME),
            "the product name follows the star: {text:?}"
        );
        assert!(
            !text.contains("Cortex Code"),
            "VAL-CORTEX-UX-002: retired copper compound must not appear: {text:?}"
        );
        assert_eq!(WORDMARK_NAME, "Cortex");
        assert!(text.starts_with('✦'), "the star leads the lockup: {text:?}");
        assert!(!text.contains(">_"), "the `>_` cliché never appears");
    }

    #[test]
    fn wordmark_star_and_name_use_theme_tokens() {
        let theme = ThemeColors::dark();
        let spans = wordmark_spans(&theme);
        // Star carries the brand accent (ORANGE on the default theme); the name is the bright tone.
        assert_eq!(spans[0].content.as_ref(), "✦");
        assert_eq!(spans[0].style.fg, Some(theme.primary));
        assert_eq!(spans[2].content.as_ref(), WORDMARK_NAME);
        assert_eq!(spans[2].content.as_ref(), "Cortex");
        assert_eq!(spans[2].style.fg, Some(theme.text_bright));
    }

    #[test]
    fn mascot_renders_in_the_brand_accent() {
        let theme = ThemeColors::dark();
        let lines = mascot_lines(&theme);
        assert_eq!(lines.len(), 4, "the minimal mascot is 4 rows");
        for line in &lines {
            for span in &line.spans {
                assert_eq!(
                    span.style.fg,
                    Some(theme.primary),
                    "every mascot cell is the brand accent (ORANGE on default)"
                );
            }
        }
    }

    #[test]
    fn mascot_uses_only_safe_block_glyphs_no_tofu() {
        for line in MASCOT_LINES {
            for ch in line.chars() {
                assert!(
                    SAFE_BLOCKS.contains(&ch),
                    "mascot uses an unsafe glyph {ch:?} (not in the safe block set)"
                );
                // No braille anywhere in the mascot art.
                assert!(
                    !('\u{2800}'..='\u{28FF}').contains(&ch),
                    "braille codepoint {ch:?} in mascot"
                );
            }
        }
    }

    #[test]
    fn mascot_lines_are_equal_width_so_rows_align() {
        let widths: Vec<usize> = MASCOT_LINES.iter().map(|l| l.chars().count()).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "all mascot rows share a width so the art stays aligned: {widths:?}"
        );
    }

    #[test]
    fn no_forbidden_glyphs_in_any_brand_asset() {
        let theme = ThemeColors::dark();
        let mut text = plain(&wordmark(&theme));
        text.push_str(&MASCOT_LINES.concat());
        for ch in text.chars() {
            assert!(
                !['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'].contains(&ch),
                "forbidden glyph {ch:?} in a brand asset"
            );
        }
    }
}
