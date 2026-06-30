//! Single source of truth for the DejaVu-safe glyph inventory and animation frame sets.
//!
//! Every glyph here is verified present in `DejaVuSansMono.ttf` (fontTools cmap check), so
//! nothing in this module renders as tofu. Screens import these constants instead of inlining
//! raw glyph literals, which keeps the safe set in one place.
//!
//! Hard rules enforced by the tests below:
//! - No braille (U+2800..=U+28FF) appears in any spinner/frame constant (the legacy braille
//!   spinner is gone).
//! - No off-brand glyphs: the thin disclosure triangles `▸`/`▾`, the command glyph `⌘`, the
//!   `>_` prompt cliché, hexagons, curly/smart quotes, or emoji.

// This module is the app-wide glyph source of truth; later milestone screens (pickers, diff
// cards, progress bars, onboarding) import these constants, so some are not yet referenced.
#![allow(dead_code)]

// --- Brand marks ---

/// Brand star / assistant author mark (AMBER).
pub const BRAND_STAR: &str = "✦";
/// Prompt / user mark / selection marker (ORANGE).
pub const PROMPT: &str = "›";

// --- Composer accents / cursor ---

/// Composer left accent edge (ORANGE).
pub const ACCENT_BAR: &str = "▌";
/// Caret / blinking cursor at the input or stream tail (ORANGE).
pub const CURSOR: &str = "▏";

// --- Key caps (hints) ---

/// Enter / submit key cap.
pub const KEY_ENTER: &str = "↵";
/// Tab / autocomplete key cap.
pub const KEY_TAB: &str = "⇥";

// --- Box drawing (rounded only) ---

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

// --- Disclosure (heavy triangles only; never the thin `▸`/`▾`) ---

/// Collapsed disclosure affordance.
pub const DISCLOSURE_COLLAPSED: &str = "▶";
/// Expanded disclosure affordance.
pub const DISCLOSURE_EXPANDED: &str = "▼";
/// Scrollback "more above" marker (GHOST); shown alone on row 0 when earlier rows are clipped off
/// the top of the viewport (the heavy up-triangle, never the thin `▴`).
pub const SCROLLBACK_ABOVE: &str = "▲";

// --- Status dots / marks ---

/// Filled status dot (timeline/status/tool done).
pub const DOT_FILLED: &str = "●";
/// Hollow status dot.
pub const DOT_HOLLOW: &str = "○";
/// Diamond severity/status marker.
pub const DOT_DIAMOND: &str = "◆";
/// Done / ok mark (OKG green).
pub const MARK_DONE: &str = "✓";
/// Error mark (ERR red).
pub const MARK_ERROR: &str = "✗";
/// Running-tool mark (ORANGE).
pub const MARK_RUNNING: &str = "⚙";
/// Faint inline separator.
pub const SEPARATOR: &str = "·";
/// Ellipsis.
pub const ELLIPSIS: &str = "…";

/// Selection bar marker (alias of [`PROMPT`]); rendered on an inverse selection bar.
pub const SELECT_MARKER: &str = PROMPT;

// --- Eighths (fractional progress fill, ascending 1/8 .. 8/8) ---

/// Progress-bar fill by eighths, ascending.
pub const EIGHTHS: [&str; 8] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

// --- Block-art helpers (safe blocks for mascot / heavy fills) ---

/// Full block.
pub const BLOCK_FULL: &str = "█";
/// Lower half block.
pub const BLOCK_LOWER_HALF: &str = "▄";
/// Upper half block.
pub const BLOCK_UPPER_HALF: &str = "▀";
/// Right half block.
pub const BLOCK_RIGHT_HALF: &str = "▐";

// --- Animation frame sets (both safe; pick per surface) ---

/// Half-circle rotation spinner (tool cards).
pub const SPINNER_ROTATE: [&str; 4] = ["◐", "◑", "◒", "◓"];
/// Breathing spinner (working / loading band).
pub const SPINNER_BREATHE: [&str; 4] = ["·", "✢", "✻", "✽"];

/// Select the active frame of `frames` for the elapsed time, at a fixed cadence.
///
/// Phase is derived from elapsed time (not a per-iteration counter) so it stays correct across
/// variable wake intervals. `period_ms` is the per-frame duration.
#[must_use]
pub fn frame_at(frames: &[&'static str], elapsed_ms: u128, period_ms: u128) -> &'static str {
    debug_assert!(!frames.is_empty());
    debug_assert!(period_ms > 0);
    let idx = (elapsed_ms / period_ms.max(1)) as usize % frames.len();
    frames[idx]
}

/// Whether `s` contains any braille codepoint (U+2800..=U+28FF).
#[must_use]
pub fn contains_braille(s: &str) -> bool {
    s.chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every glyph/frame this module exposes, flattened, for whole-inventory assertions.
    fn inventory() -> Vec<&'static str> {
        let scalars = [
            BRAND_STAR,
            PROMPT,
            ACCENT_BAR,
            CURSOR,
            KEY_ENTER,
            KEY_TAB,
            BOX_TOP_LEFT,
            BOX_TOP_RIGHT,
            BOX_BOTTOM_LEFT,
            BOX_BOTTOM_RIGHT,
            BOX_HORIZONTAL,
            BOX_VERTICAL,
            DISCLOSURE_COLLAPSED,
            DISCLOSURE_EXPANDED,
            SCROLLBACK_ABOVE,
            DOT_FILLED,
            DOT_HOLLOW,
            DOT_DIAMOND,
            MARK_DONE,
            MARK_ERROR,
            MARK_RUNNING,
            SEPARATOR,
            ELLIPSIS,
            SELECT_MARKER,
            BLOCK_FULL,
            BLOCK_LOWER_HALF,
            BLOCK_UPPER_HALF,
            BLOCK_RIGHT_HALF,
        ];
        let mut all: Vec<&'static str> = scalars.to_vec();
        all.extend_from_slice(&EIGHTHS);
        all.extend_from_slice(&SPINNER_ROTATE);
        all.extend_from_slice(&SPINNER_BREATHE);
        all
    }

    #[test]
    fn no_braille_in_any_spinner_or_frame_constant() {
        for frames in [&EIGHTHS[..], &SPINNER_ROTATE[..], &SPINNER_BREATHE[..]] {
            for f in frames {
                assert!(
                    !contains_braille(f),
                    "braille codepoint found in a frame constant: {f:?}"
                );
            }
        }
    }

    #[test]
    fn no_braille_anywhere_in_inventory() {
        for g in inventory() {
            assert!(!contains_braille(g), "braille codepoint in inventory: {g:?}");
        }
    }

    #[test]
    fn no_forbidden_glyphs_in_inventory() {
        // Thin triangles, command glyph, hexagon, curly quotes, and the `>_` cliché never appear.
        let forbidden = ['▸', '▾', '⌘', '⬡', '“', '”', '‘', '’'];
        for g in inventory() {
            for c in g.chars() {
                assert!(
                    !forbidden.contains(&c),
                    "forbidden glyph {c:?} present in inventory entry {g:?}"
                );
                // No emoji / pictographs (rough supplementary-plane + symbol guard).
                assert!(
                    !('\u{1F000}'..='\u{1FAFF}').contains(&c),
                    "emoji codepoint {c:?} present in inventory entry {g:?}"
                );
            }
        }
        let joined: String = inventory().concat();
        assert!(!joined.contains(">_"), "the `>_` cliché must never appear");
    }

    #[test]
    fn spinner_sets_are_the_safe_documented_frames() {
        assert_eq!(SPINNER_ROTATE, ["◐", "◑", "◒", "◓"]);
        assert_eq!(SPINNER_BREATHE, ["·", "✢", "✻", "✽"]);
    }

    #[test]
    fn eighths_are_ascending_and_full_at_the_end() {
        assert_eq!(EIGHTHS.len(), 8);
        assert_eq!(EIGHTHS[7], "█");
        assert_eq!(EIGHTHS[0], "▏");
    }

    #[test]
    fn disclosure_uses_heavy_triangles() {
        assert_eq!(DISCLOSURE_COLLAPSED, "▶");
        assert_eq!(DISCLOSURE_EXPANDED, "▼");
        // The scrollback marker is the heavy up-triangle, matching the heavy disclosure set.
        assert_eq!(SCROLLBACK_ABOVE, "▲");
    }

    #[test]
    fn frame_at_is_a_pure_function_of_elapsed_time() {
        // Same elapsed → same frame; advances by cadence; wraps over the set length.
        assert_eq!(frame_at(&SPINNER_ROTATE, 0, 100), "◐");
        assert_eq!(frame_at(&SPINNER_ROTATE, 50, 100), "◐");
        assert_eq!(frame_at(&SPINNER_ROTATE, 100, 100), "◑");
        assert_eq!(frame_at(&SPINNER_ROTATE, 300, 100), "◓");
        assert_eq!(frame_at(&SPINNER_ROTATE, 400, 100), "◐");
    }
}
