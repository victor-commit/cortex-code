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

    /// Syntax-highlight a compact JSON object line (one object per line, no reflow), for the
    /// human/TTY branch of `run --jsonl`. Keys are `SYNTAX_KEY`, string values `SYNTAX_STRING`,
    /// numbers `SYNTAX_NUMBER`, `true`/`false`/`null` `SYNTAX_KEYWORD`, and structural punctuation
    /// `SYNTAX_PUNCT`; the two status exceptions (a `stop` finish/`reason` value and the `done`
    /// stream `type` discriminator) render in `OK_GREEN`. Returns the input unchanged when the
    /// styler is disabled, so a piped consumer never sees an ANSI byte (byte-stability).
    #[must_use]
    pub fn highlight_json(self, json: &str) -> String {
        if !self.enabled {
            return json.to_owned();
        }
        colorize_json(json, false)
    }

    /// Pretty-print (2-space indent, multi-line) AND syntax-highlight a compact JSON document, for
    /// the human/TTY branch of `run --format json`. The reflow reads the already-serialized compact
    /// bytes so field order (and any redaction) is preserved exactly; only insignificant whitespace
    /// is added. Returns the input unchanged when the styler is disabled.
    #[must_use]
    pub fn highlight_json_pretty(self, json: &str) -> String {
        if !self.enabled {
            return json.to_owned();
        }
        colorize_json(json, true)
    }
}

// --- `serve` banner + access-log rendering (VAL-CLI-012..016 / 037) --------------------------
//
// The `serve` banner, routes hint, and access-log rows are the human/stderr surface of
// `cortex serve`. Machine consumers read stdout, which stays untouched. Every helper below routes
// its color through a [`Styler`], so a disabled styler (piped/non-TTY stderr, or `NO_COLOR`)
// yields exactly the historical plain bytes with zero ANSI escapes.

/// Leading indent for an access-log row.
const ACCESS_LOG_INDENT: &str = "  ";
/// Method column width (fits `GET`/`POST`/`PUT`/`PATCH`/`DELETE`, always with a trailing gap).
const ACCESS_LOG_METHOD_W: usize = 6;
/// Path column width, sized so the status column aligns for the common short routes; longer paths
/// overflow gracefully while keeping at least a two-column gap before the status.
const ACCESS_LOG_PATH_W: usize = 26;

/// The status color for an HTTP status code in the serve access log: green for 2xx/3xx (success /
/// redirect), red otherwise. Green here is a sanctioned status hue, never a plain data number.
#[must_use]
pub fn http_status_color(status: u16) -> Rgb {
    if (200..400).contains(&status) {
        OK_GREEN
    } else {
        ERR_RED
    }
}

/// The `serve` listening banner line (no trailing newline).
///
/// Renders `cortex serve listening on http://<addr>` with the URL painted URL-blue. The leading
/// text is left unstyled so a disabled styler reproduces the exact historical plain banner byte
/// for byte.
#[must_use]
pub fn serve_banner_line(styler: Styler, addr: &str) -> String {
    let url = format!("http://{addr}");
    format!("cortex serve listening on {}", styler.paint(&url, URL_BLUE))
}

/// The faint `serve` routes-hint footer (no trailing newline), naming real server routes.
#[must_use]
pub fn serve_routes_hint(styler: Styler) -> String {
    let hint =
        format!("routes: /health /event /session /openapi.json {ELLIPSIS} (Ctrl-C to stop)");
    styler.paint(&hint, FAINT)
}

/// The human duration label for a completed request: `open` for a long-lived stream, `<n>ms` under
/// a second, else `<n>.<d>s`.
#[must_use]
pub fn serve_duration_label(duration: std::time::Duration, streaming: bool) -> String {
    if streaming {
        return "open".to_owned();
    }
    let millis = duration.as_millis();
    if millis < 1000 {
        format!("{millis}ms")
    } else {
        format!("{:.1}s", duration.as_secs_f64())
    }
}

/// One aligned `serve` access-log row (no trailing newline): method (ORANGE) + path (TXT) +
/// status (green for 2xx/3xx, red otherwise) + duration (FAINT). Streaming responses annotate the
/// path with `(SSE)` and show an `open` duration.
#[must_use]
pub fn serve_access_log_row(
    styler: Styler,
    method: &str,
    path: &str,
    status: u16,
    duration: std::time::Duration,
    streaming: bool,
) -> String {
    // Method cell: left-justified, guaranteed to end with at least one space so a long method
    // (e.g. DELETE) never abuts the path.
    let width = ACCESS_LOG_METHOD_W;
    let mut method_cell = format!("{method:<width$}");
    if !method_cell.ends_with(' ') {
        method_cell.push(' ');
    }

    // Path cell, padded so the status column lines up for the common routes; a streaming response
    // is annotated with `(SSE)`.
    let path_display = if streaming {
        format!("{path}  (SSE)")
    } else {
        path.to_owned()
    };
    let path_width = ACCESS_LOG_PATH_W;
    let path_cell = if path_display.len() < path_width {
        format!("{path_display:<path_width$}")
    } else {
        format!("{path_display}  ")
    };

    let status_str = status.to_string();
    let duration_label = serve_duration_label(duration, streaming);

    format!(
        "{ACCESS_LOG_INDENT}{method}{path}{status}  {duration}",
        method = styler.paint(&method_cell, ORANGE),
        path = styler.paint(&path_cell, TXT),
        status = styler.paint(&status_str, http_status_color(status)),
        duration = styler.paint(&duration_label, FAINT),
    )
}

/// Wrap `text` in a foreground-color SGR sequence. Used only on the enabled/TTY styling path.
fn wrap(text: &str, color: Rgb) -> String {
    format!("{}{text}{RESET}", Style::fg(color).open_sequence())
}

/// The VS Code Dark+ color for a JSON string *value*, honoring the two status-green exceptions:
/// a `finish_reason`/`reason` value of `"stop"` and a stream `type` value of `"done"`.
fn json_string_value_color(key: Option<&str>, value: &str) -> Rgb {
    match key {
        Some("finish_reason" | "reason") if value == "stop" => OK_GREEN,
        Some("type") if value == "done" => OK_GREEN,
        _ => SYNTAX_STRING,
    }
}

/// Tokenize a compact JSON string and emit a syntax-highlighted rendering, optionally reflowed to
/// a 2-space-indented multi-line form. The input is assumed to be well-formed compact JSON (no
/// insignificant whitespace between tokens), as produced by `serde_json::to_string`.
fn colorize_json(json: &str, pretty: bool) -> String {
    let bytes = json.as_bytes();
    let mut out = String::with_capacity(json.len() * 3);
    let mut indent: usize = 0;
    let mut last_key: Option<String> = None;
    let mut i = 0;

    let push_indent = |out: &mut String, indent: usize| {
        for _ in 0..indent {
            out.push_str("  ");
        }
    };

    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                let raw = &json[start..i];
                let is_key = i < bytes.len() && bytes[i] == b':';
                if is_key {
                    last_key = Some(json[start + 1..i - 1].to_owned());
                    out.push_str(&wrap(raw, SYNTAX_KEY));
                } else {
                    let value = &json[start + 1..i - 1];
                    let color = json_string_value_color(last_key.as_deref(), value);
                    last_key = None;
                    out.push_str(&wrap(raw, color));
                }
            }
            open @ (b'{' | b'[') => {
                out.push_str(&wrap(std::str::from_utf8(&[open]).unwrap_or("{"), SYNTAX_PUNCT));
                let close = if open == b'{' { b'}' } else { b']' };
                if i + 1 < bytes.len() && bytes[i + 1] == close {
                    // Empty container: keep `{}` / `[]` inline.
                    out.push_str(&wrap(std::str::from_utf8(&[close]).unwrap_or("}"), SYNTAX_PUNCT));
                    last_key = None;
                    i += 2;
                    continue;
                }
                if pretty {
                    indent += 1;
                    out.push('\n');
                    push_indent(&mut out, indent);
                }
                i += 1;
            }
            close @ (b'}' | b']') => {
                if pretty {
                    indent = indent.saturating_sub(1);
                    out.push('\n');
                    push_indent(&mut out, indent);
                }
                out.push_str(&wrap(std::str::from_utf8(&[close]).unwrap_or("}"), SYNTAX_PUNCT));
                last_key = None;
                i += 1;
            }
            b',' => {
                out.push_str(&wrap(",", SYNTAX_PUNCT));
                if pretty {
                    out.push('\n');
                    push_indent(&mut out, indent);
                }
                i += 1;
            }
            b':' => {
                out.push_str(&wrap(":", SYNTAX_PUNCT));
                if pretty {
                    out.push(' ');
                }
                i += 1;
            }
            b't' | b'f' | b'n' => {
                let rest = &json[i..];
                let literal = ["true", "false", "null"]
                    .into_iter()
                    .find(|lit| rest.starts_with(lit));
                if let Some(lit) = literal {
                    out.push_str(&wrap(lit, SYNTAX_KEYWORD));
                    last_key = None;
                    i += lit.len();
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => i += 1,
                        _ => break,
                    }
                }
                out.push_str(&wrap(&json[start..i], SYNTAX_NUMBER));
                last_key = None;
            }
            other => {
                // Only ASCII whitespace can legitimately appear here in well-formed JSON.
                out.push(other as char);
                i += 1;
            }
        }
    }
    out
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

    /// The SGR opening sequence for a foreground-only color, so tests can assert a specific token
    /// wraps a specific slice.
    fn fg_open(color: Rgb) -> String {
        Style::fg(color).open_sequence()
    }

    #[test]
    fn disabled_styler_json_highlighters_are_pure_passthrough() {
        let plain = Styler::new(false);
        let compact = r#"{"type":"done","reason":"stop","steps":2}"#;
        assert_eq!(plain.highlight_json(compact), compact);
        assert_eq!(plain.highlight_json_pretty(compact), compact);
        assert!(!plain.highlight_json(compact).contains('\x1b'));
        assert!(!plain.highlight_json_pretty(compact).contains('\x1b'));
    }

    #[test]
    fn highlight_json_colors_keys_strings_numbers_and_keywords() {
        let styled = Styler::new(true);
        let out = styled.highlight_json(
            r#"{"id":"c1","is_error":false,"total_tokens":4204}"#,
        );
        // Object key -> VS Code Dark+ VAR (light blue).
        assert!(out.contains(&format!("{}{}{RESET}", fg_open(SYNTAX_KEY), r#""id""#)));
        // String value -> STR (orange-brown).
        assert!(out.contains(&format!("{}{}{RESET}", fg_open(SYNTAX_STRING), r#""c1""#)));
        // Boolean literal -> KEYWORD/CONST.
        assert!(out.contains(&format!("{}false{RESET}", fg_open(SYNTAX_KEYWORD))));
        // Number -> NUM (green syntax token).
        assert!(out.contains(&format!("{}4204{RESET}", fg_open(SYNTAX_NUMBER))));
        // Single line (jsonl): no reflow.
        assert!(!out.contains('\n'), "jsonl highlight must stay single-line");
    }

    #[test]
    fn highlight_json_greens_the_done_type_and_stop_reason_only() {
        let styled = Styler::new(true);
        let done = styled.highlight_json(r#"{"type":"done","reason":"stop"}"#);
        // The `done` type discriminator is status-green (VAL-CLI-038).
        assert!(done.contains(&format!("{}{}{RESET}", fg_open(OK_GREEN), r#""done""#)));
        // The `stop` reason value is status-green (VAL-CLI-005/009).
        assert!(done.contains(&format!("{}{}{RESET}", fg_open(OK_GREEN), r#""stop""#)));

        // A non-terminal `type` value stays STR orange-brown, not green.
        let session = styled.highlight_json(r#"{"type":"session","agent":"build"}"#);
        assert!(session.contains(&format!("{}{}{RESET}", fg_open(SYNTAX_STRING), r#""session""#)));
        assert!(
            !session.contains(&format!("{}{}{RESET}", fg_open(OK_GREEN), r#""session""#)),
            "only the `done` type value is green"
        );
        // A generic string value that happens to equal `stop` under a non-reason key is NOT green.
        let generic = styled.highlight_json(r#"{"name":"stop"}"#);
        assert!(generic.contains(&format!("{}{}{RESET}", fg_open(SYNTAX_STRING), r#""stop""#)));
    }

    #[test]
    fn highlight_json_pretty_reflows_with_two_space_indent() {
        let styled = Styler::new(true);
        let out = styled.highlight_json_pretty(r#"{"role":"assistant","steps":1}"#);
        // Stripping ANSI recovers a canonical 2-space-indented multi-line document.
        let plain = strip_ansi(&out);
        assert_eq!(
            plain,
            "{\n  \"role\": \"assistant\",\n  \"steps\": 1\n}",
            "pretty output is 2-space indented, colon-spaced, multi-line"
        );
    }

    #[test]
    fn highlight_json_pretty_keeps_empty_containers_inline() {
        let styled = Styler::new(true);
        let out = styled.highlight_json_pretty(r#"{"tools":[],"meta":{}}"#);
        let plain = strip_ansi(&out);
        assert_eq!(plain, "{\n  \"tools\": [],\n  \"meta\": {}\n}");
    }

    #[test]
    fn highlight_json_pretty_stripped_of_ansi_reparses_to_the_same_value() {
        let styled = Styler::new(true);
        let compact =
            r#"{"role":"assistant","usage":{"input_tokens":10,"cost":0.04},"tools":["read","edit"]}"#;
        let out = styled.highlight_json_pretty(compact);
        let plain = strip_ansi(&out);
        let a: serde_json::Value = serde_json::from_str(compact).expect("compact json");
        let b: serde_json::Value = serde_json::from_str(&plain).expect("pretty json reparses");
        assert_eq!(a, b, "pretty reflow preserves the JSON value");
    }

    /// Remove every SGR escape sequence so a highlighted string can be compared structurally.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn http_status_color_greens_success_and_reds_errors() {
        for ok in [200_u16, 201, 204, 301, 302, 399] {
            assert_eq!(http_status_color(ok), OK_GREEN, "{ok} is a success/redirect");
        }
        for err in [400_u16, 401, 404, 429, 500, 503] {
            assert_eq!(http_status_color(err), ERR_RED, "{err} is an error");
        }
    }

    #[test]
    fn serve_banner_line_is_plain_and_ansi_free_when_disabled() {
        let plain = Styler::new(false);
        let line = serve_banner_line(plain, "127.0.0.1:3200");
        assert_eq!(line, "cortex serve listening on http://127.0.0.1:3200");
        assert!(!line.contains('\x1b'), "disabled banner has no ANSI");
    }

    #[test]
    fn serve_banner_line_paints_the_url_blue_when_enabled() {
        let styled = Styler::new(true);
        let line = serve_banner_line(styled, "127.0.0.1:3200");
        // The URL is wrapped in the URL-blue SGR; the lead text is unstyled.
        assert!(line.starts_with("cortex serve listening on "));
        assert!(line.contains(&format!(
            "{}http://127.0.0.1:3200\x1b[0m",
            Style::fg(URL_BLUE).open_sequence()
        )));
        // Stripping ANSI recovers the exact plain banner.
        assert_eq!(
            strip_ansi(&line),
            "cortex serve listening on http://127.0.0.1:3200"
        );
    }

    #[test]
    fn serve_routes_hint_names_real_routes_and_is_faint() {
        let styled = Styler::new(true);
        let hint = serve_routes_hint(styled);
        let plain = strip_ansi(&hint);
        for route in ["/health", "/event", "/session", "/openapi.json"] {
            assert!(plain.contains(route), "routes hint names {route}: {plain}");
        }
        assert!(plain.contains("(Ctrl-C to stop)"));
        // Whole line is faint; disabled styler is a plain passthrough.
        assert!(hint.contains(&Style::fg(FAINT).open_sequence()));
        let disabled = serve_routes_hint(Styler::new(false));
        assert!(!disabled.contains('\x1b'), "disabled routes hint has no ANSI");
        assert_eq!(disabled, plain);
    }

    #[test]
    fn serve_duration_label_formats_ms_seconds_and_open() {
        use std::time::Duration;
        assert_eq!(serve_duration_label(Duration::from_millis(1), false), "1ms");
        assert_eq!(serve_duration_label(Duration::from_millis(8), false), "8ms");
        assert_eq!(serve_duration_label(Duration::from_millis(999), false), "999ms");
        assert_eq!(serve_duration_label(Duration::from_millis(1400), false), "1.4s");
        // Streaming responses show `open` regardless of the elapsed time.
        assert_eq!(serve_duration_label(Duration::from_millis(3), true), "open");
    }

    #[test]
    fn serve_access_log_row_is_plain_and_ansi_free_when_disabled() {
        use std::time::Duration;
        let plain = Styler::new(false);
        let row = serve_access_log_row(plain, "GET", "/health", 200, Duration::from_millis(1), false);
        assert!(!row.contains('\x1b'), "disabled row has no ANSI: {row:?}");
        // Aligned columns: 2-space indent, method, path, status, duration.
        assert_eq!(row, "  GET   /health                   200  1ms");
    }

    #[test]
    fn serve_access_log_row_colors_method_status_and_duration() {
        use std::time::Duration;
        let styled = Styler::new(true);
        let ok = serve_access_log_row(styled, "POST", "/session", 201, Duration::from_millis(8), false);
        assert!(ok.contains(&format!("{}POST", Style::fg(ORANGE).open_sequence())));
        assert!(ok.contains(&format!("{}201\x1b[0m", Style::fg(OK_GREEN).open_sequence())));
        assert!(ok.contains(&format!("{}8ms\x1b[0m", Style::fg(FAINT).open_sequence())));
        // A server error paints the status red.
        let err = serve_access_log_row(styled, "GET", "/boom", 500, Duration::from_millis(2), false);
        assert!(err.contains(&format!("{}500\x1b[0m", Style::fg(ERR_RED).open_sequence())));
        // The visible (ANSI-stripped) columns stay aligned regardless of styling.
        assert_eq!(
            strip_ansi(&ok),
            "  POST  /session                  201  8ms"
        );
    }

    #[test]
    fn serve_access_log_row_annotates_streaming_with_sse_and_open() {
        use std::time::Duration;
        let styled = Styler::new(true);
        let row = serve_access_log_row(styled, "GET", "/event", 200, Duration::from_millis(0), true);
        let plain = strip_ansi(&row);
        assert!(plain.contains("/event  (SSE)"), "SSE annotation present: {plain}");
        assert!(plain.trim_end().ends_with("open"), "streaming shows open: {plain}");
        assert!(!plain.contains("0ms"), "streaming never shows a millisecond duration");
    }
}
