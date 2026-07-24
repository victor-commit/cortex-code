//! Shared Cortex shell chrome primitives for the product TUI.
//!
//! Structure authority: mockups-v3 session streaming/chat-turn DNA + pure-structure law.
//! These helpers emit plain layout strings (and optional ratatui [`Line`]s) so conversation,
//! pickers, and full-screen views share one fingerprint:
//!
//! ```text
//! cwd (muted, left) .................... context budget (right)
//! scrollback: bare ❯ user wash · ◆ Thought · assistant md
//! busy: ⠧ Responding… Ns  ....  Ts ⇣tok [stop]
//! composer jewel: ❯ draft ........ Cortex (high) · always-approve
//! putty: Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts
//! ```
//!
//! Bans (fail-closed): `❯ You` / `◆ Cortex` nameplates, copper `Code` brand, `0mNNs` timers,
//! middot-only putty as the primary dynasty.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Cortex prompt arrow (U+276F).
pub const PROMPT_ARROW: char = '❯';

/// Thought / group diamond (not an agent nameplate).
pub const THOUGHT_DIAMOND: char = '◆';

/// Default model label for the prompt jewel chip.
pub const MODEL_LABEL: &str = "Cortex";

/// Default effort tier shown in the prompt jewel (`(high)`).
pub const MODEL_EFFORT: &str = "high";

/// Default permission / work mode flag for the prompt jewel chip.
pub const MODEL_MODE_DEFAULT: &str = "always-approve";

/// Default authenticated putty footer (product `|` separators).
pub const DEFAULT_PUTTY: &str = "Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts";

/// Demo cwd used by deterministic fixtures and chrome unit tests.
pub const DEMO_CWD: &str = "~/projects/demo";

/// Demo context budget parts matching pure-structure streaming DNA.
pub const DEMO_CTX_USED: &str = "9.5K";
pub const DEMO_CTX_TOTAL: &str = "500K";

/// Compact token budget form used on authenticated top chrome (`9.5K / 500K`).
#[must_use]
pub fn format_context_budget(used: &str, total: &str) -> String {
    format!("{used} / {total}")
}

/// Format a raw token count into product compact units (`9.5K`, `1.0M`, bare `<1000`).
///
/// Product form prefers uppercase `K`/`M` so the top-bar budget regex
/// `\d+(\.\d+)?[KM] / \d+(\.\d+)?[KM]` matches mockups-v3.
#[must_use]
pub fn format_token_compact(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        let v = n as f64 / 1_000.0;
        if (v - v.round()).abs() < f64::EPSILON {
            format!("{}K", v.round() as u64)
        } else {
            format!("{v:.1}K")
        }
    } else {
        let v = n as f64 / 1_000_000.0;
        if (v - v.round()).abs() < f64::EPSILON {
            format!("{}M", v.round() as u64)
        } else {
            format!("{v:.1}M")
        }
    }
}

/// Product turn timer (source `format_turn_timer`):
/// - `<10s` → one decimal (`4.0s`, `1.4s`, `0.5s`)
/// - `10–59s` → whole seconds (`15s`, `59s`)
/// - `≥60s` → `1m20s` / `1h2m` (never `0mNNs` for under a minute)
#[must_use]
pub fn format_turn_timer(total_secs_f64: f64) -> String {
    let total_secs = total_secs_f64.max(0.0).floor() as u64;
    if total_secs < 10 {
        return format!("{:.1}s", total_secs_f64.max(0.0));
    }
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins < 60 {
        return format!("{mins}m{secs}s");
    }
    let hours = mins / 60;
    let remaining_mins = mins % 60;
    format!("{hours}h{remaining_mins}m")
}

/// Build the right-aligned model chip (`Cortex (high) · always-approve`).
#[must_use]
pub fn model_chip(model: &str, effort: Option<&str>, mode: &str) -> String {
    match effort {
        Some(e) if !e.is_empty() => format!("{model} ({e}) · {mode}"),
        _ => format!("{model} · {mode}"),
    }
}

/// Default model chip used by idle/busy conversation DNA.
#[must_use]
pub fn default_model_chip() -> String {
    model_chip(MODEL_LABEL, Some(MODEL_EFFORT), MODEL_MODE_DEFAULT)
}

/// True when `text` carries product context budget form (`9.5K / 500K`, `8.5K / 1.0M`, …).
#[must_use]
pub fn has_context_budget_form(text: &str) -> bool {
    text.lines().any(line_has_context_budget)
}

fn line_has_context_budget(line: &str) -> bool {
    // Prefer an explicit used/total pair with KM units.
    for chunk in line.split_whitespace() {
        // skip
        let _ = chunk;
    }
    // Scan for `N[KM] / N[KM]` anywhere on the line (spaces around slash optional).
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((used_end, total_start)) = find_budget_slash(bytes, i) {
            let used = &line[..used_end];
            // Find start of used token (digits).
            let used_tok = used
                .rsplit(|c: char| !(c.is_ascii_digit() || c == '.' || c == 'K' || c == 'M'))
                .next()
                .unwrap_or("");
            let rest = &line[total_start..];
            let total_tok = rest
                .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == 'K' || c == 'M'))
                .next()
                .unwrap_or("");
            if is_budget_token(used_tok) && is_budget_token(total_tok) {
                return true;
            }
            i = total_start;
            continue;
        }
        i += 1;
    }
    false
}

fn find_budget_slash(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            // optional spaces around slash
            let mut left = i;
            while left > from && bytes[left - 1] == b' ' {
                left -= 1;
            }
            let mut right = i + 1;
            while right < bytes.len() && bytes[right] == b' ' {
                right += 1;
            }
            return Some((left, right));
        }
        i += 1;
    }
    None
}

fn is_budget_token(t: &str) -> bool {
    let bytes = t.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let unit = *bytes.last().unwrap() as char;
    if unit != 'K' && unit != 'M' {
        return false;
    }
    let num = &t[..t.len() - 1];
    !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit() || c == '.')
        && num.chars().any(|c| c.is_ascii_digit())
}

/// Authenticated top chrome as a single width-padded line: muted cwd left + budget right.
#[must_use]
pub fn auth_top_bar_line(cwd: &str, used: &str, total: &str, width: usize) -> String {
    let left = format!("  {cwd}");
    let budget = format_context_budget(used, total);
    let right = format!("{budget}  ");
    pad_left_right(&left, &right, width)
}

/// Demo top bar matching streaming DNA (`~/projects/demo` + `9.5K / 500K`).
#[must_use]
pub fn demo_auth_top_bar(width: usize) -> String {
    auth_top_bar_line(DEMO_CWD, DEMO_CTX_USED, DEMO_CTX_TOTAL, width)
}

/// Bare user wash primary line: `❯ <prompt text>` (optional right timestamp).
///
/// BAN: never emit `❯ You` / `> You` speaker nameplates.
#[must_use]
pub fn user_wash_line(prompt: &str, timestamp: Option<&str>, width: usize) -> String {
    let head = format!("  {PROMPT_ARROW} {prompt}");
    match timestamp {
        Some(ts) if !ts.is_empty() => {
            let right = format!("{ts}   ");
            pad_left_right(&head, &right, width)
        }
        _ => pad_right(&head, width),
    }
}

/// Thought group line: `◆ Thought for {time_label}`.
///
/// Diamonds belong on Thought/group lines only — never as a `◆ Cortex` nameplate.
#[must_use]
pub fn thought_line(time_label: &str) -> String {
    format!("  {THOUGHT_DIAMOND} Thought for {time_label}")
}

/// Idle/busy prompt jewel draft row interior (no box borders):
/// `❯ <draft> ........ Cortex (high) · always-approve`
#[must_use]
pub fn prompt_jewel_inner(
    draft: &str,
    model: &str,
    effort: Option<&str>,
    mode: &str,
    inner_width: usize,
) -> String {
    let left = format!("{PROMPT_ARROW} {draft}");
    let chip = model_chip(model, effort, mode);
    // One space gap minimum between draft and chip; chip flush to the right with 1-col inset.
    let right = format!("{chip} ");
    pad_left_right(&format!(" {left}"), &right, inner_width.max(left.chars().count() + right.chars().count() + 2))
}

/// Default prompt jewel interior using Cortex (high) · always-approve.
#[must_use]
pub fn default_prompt_jewel_inner(draft: &str, inner_width: usize) -> String {
    prompt_jewel_inner(
        draft,
        MODEL_LABEL,
        Some(MODEL_EFFORT),
        MODEL_MODE_DEFAULT,
        inner_width,
    )
}

/// Busy turn-status row: activity left + timer/tokens/`[stop]` right.
///
/// Example: `  ⠧ Responding… 1.4s                         4.0s ⇣2.1k [stop]`
#[must_use]
pub fn turn_status_busy_line(
    activity: &str,
    elapsed: &str,
    right_timer: &str,
    tokens: &str,
    width: usize,
) -> String {
    let left = format!("  ⠧ {activity} {elapsed}");
    let right = format!("{right_timer} ⇣{tokens} [stop]  ");
    pad_left_right(&left, &right, width)
}

/// Putty footer line (left-padded product hints).
#[must_use]
pub fn putty_footer_line(hints: &str, width: usize) -> String {
    pad_right(&format!(" {hints}"), width)
}

/// Default putty footer.
#[must_use]
pub fn default_putty_line(width: usize) -> String {
    putty_footer_line(DEFAULT_PUTTY, width)
}

/// Fail-closed residual scan for copper / Cortex identity leftovers in chrome paint.
pub fn reject_copper_residual(text: &str) -> Result<(), String> {
    let banned = [
        "Cortex Code",
        "cortex-code",
        "Code ·",
        "Ask Code",
        "How Code should work",
        "Work mode",
        "Claude Opus",
        "Claude Sonnet",
        "Authorize OpenAI",
        "copper",
    ];
    for needle in banned {
        if text.contains(needle) {
            return Err(format!("product chrome residual refused: found {needle:?}"));
        }
    }
    for line in text.lines() {
        if line.trim() == "Code" {
            return Err("product chrome residual refused: ASCII Code monument".to_owned());
        }
    }
    Ok(())
}

/// Pure-structure residual scan (nameplates, bad timers, invent monologue).
pub fn reject_structure_residual(text: &str) -> Result<(), String> {
    if text.contains("❯ You") || text.contains("> You") {
        return Err("pure-structure residual refused: You nameplate".to_owned());
    }
    if text.contains("◆ Cortex") {
        return Err("pure-structure residual refused: Cortex nameplate".to_owned());
    }
    if text.contains("Draft ·") {
        return Err("pure-structure residual refused: Draft·N invent".to_owned());
    }
    for monologue in [
        "Multi-line jewel",
        "expanded thinking kept collapsed",
        "statusline promises",
        "putty lectures",
    ] {
        if text.contains(monologue) {
            return Err(format!(
                "pure-structure residual refused: gallery monologue {monologue:?}"
            ));
        }
    }
    // Zero-padded minute form for sub-minute timers (e.g. 0m04s).
    if text.split_whitespace().any(|tok| {
        tok.len() >= 4
            && tok.starts_with("0m")
            && tok.ends_with('s')
            && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
    }) {
        return Err("pure-structure residual refused: 0mNNs timer form".to_owned());
    }
    Ok(())
}

/// True when product putty lines use `|` (or box `│`) separators rather than middot-only.
#[must_use]
pub fn putty_product_separators_hold(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        let is_product_putty = trimmed.contains("Shift+Tab:mode")
            && (trimmed.contains("Ctrl+c") || trimmed.contains("Ctrl+c:cancel"));
        if !is_product_putty {
            continue;
        }
        if trimmed.contains('|') || trimmed.contains('│') {
            continue;
        }
        if trimmed.contains('·') {
            return false;
        }
    }
    true
}

/// CortexNight palette tokens (product default dark chrome).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CortexNightPalette;

impl CortexNightPalette {
    pub const CANVAS: Color = Color::Rgb(0x14, 0x14, 0x14);
    pub const SURFACE: Color = Color::Rgb(0x24, 0x24, 0x24);
    pub const TEXT: Color = Color::Rgb(0xe1, 0xe1, 0xe1);
    pub const BRIGHT: Color = Color::Rgb(0xf3, 0xf3, 0xf3);
    pub const SECONDARY: Color = Color::Rgb(0xc8, 0xc8, 0xc8);
    pub const MUTED: Color = Color::Rgb(0x6c, 0x6c, 0x6c);
    pub const DIM: Color = Color::Rgb(0x58, 0x58, 0x58);
    pub const BORDER: Color = Color::Rgb(0x32, 0x32, 0x37);
    /// Magenta / violet Cortex accent `#bb9af7`.
    pub const ACCENT: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
    pub const INFO: Color = Color::Rgb(0x7a, 0xa2, 0xf7);
    pub const OK: Color = Color::Rgb(0x9e, 0xce, 0x6a);
    pub const WARN: Color = Color::Rgb(0xe0, 0xaf, 0x68);
    pub const BAD: Color = Color::Rgb(0xf7, 0x76, 0x8e);
    pub const RUNNING: Color = Color::Rgb(0x7d, 0xcf, 0xff);
    /// Forbidden copper Cortex identity accent.
    pub const FORBIDDEN_COPPER: Color = Color::Rgb(0xd9, 0x82, 0x5f);

    #[must_use]
    pub fn canvas_hex() -> &'static str {
        "#141414"
    }
    #[must_use]
    pub fn text_hex() -> &'static str {
        "#e1e1e1"
    }
    #[must_use]
    pub fn accent_hex() -> &'static str {
        "#bb9af7"
    }
}

/// Ratatui line for the authenticated top bar.
#[must_use]
pub fn auth_top_bar_ratatui(
    cwd: &str,
    used: &str,
    total: &str,
    width: usize,
    muted: Style,
) -> Line<'static> {
    let raw = auth_top_bar_line(cwd, used, total, width);
    Line::from(Span::styled(raw, muted))
}

/// Ratatui line for a bare user wash row.
#[must_use]
pub fn user_wash_ratatui(
    prompt: &str,
    timestamp: Option<&str>,
    width: usize,
    style: Style,
    muted: Style,
) -> Line<'static> {
    let head = format!("  {PROMPT_ARROW} {prompt}");
    match timestamp {
        Some(ts) if !ts.is_empty() && width > head.chars().count() + ts.chars().count() + 4 => {
            let gap = width.saturating_sub(head.chars().count() + ts.chars().count() + 3);
            Line::from(vec![
                Span::styled(head, style),
                Span::raw(" ".repeat(gap)),
                Span::styled(format!("{ts}   "), muted),
            ])
        }
        _ => Line::from(Span::styled(pad_right(&head, width), style)),
    }
}

/// Ratatui line for a Thought group header.
#[must_use]
pub fn thought_ratatui(time_label: &str, accent: Style) -> Line<'static> {
    Line::from(Span::styled(
        thought_line(time_label),
        accent.add_modifier(Modifier::BOLD),
    ))
}

/// Ratatui line for the busy turn-status band.
#[must_use]
pub fn turn_status_busy_ratatui(
    activity: &str,
    elapsed: &str,
    right_timer: &str,
    tokens: &str,
    width: usize,
    running: Style,
    muted: Style,
    bad: Style,
) -> Line<'static> {
    let left = format!("  ⠧ {activity} {elapsed}");
    let mid = format!("{right_timer} ⇣{tokens} ");
    let stop = "[stop]";
    let right_w = mid.chars().count() + stop.chars().count() + 2;
    let gap = width.saturating_sub(left.chars().count() + right_w);
    Line::from(vec![
        Span::styled(left, running),
        Span::raw(" ".repeat(gap)),
        Span::styled(mid, muted),
        Span::styled(stop.to_owned(), bad),
        Span::raw("  "),
    ])
}

/// Ratatui line for the prompt jewel draft row (interior, no borders).
#[must_use]
pub fn prompt_jewel_ratatui(
    draft: &str,
    model: &str,
    effort: Option<&str>,
    mode: &str,
    inner_width: usize,
    accent: Style,
    text: Style,
    muted: Style,
) -> Line<'static> {
    let left = format!(" {PROMPT_ARROW} ");
    let chip = model_chip(model, effort, mode);
    let right = format!(" {chip} ");
    let draft_w = draft.chars().count();
    let used = left.chars().count() + draft_w + right.chars().count();
    let gap = inner_width.saturating_sub(used).max(1);
    Line::from(vec![
        Span::styled(left, accent.add_modifier(Modifier::BOLD)),
        Span::styled(draft.to_owned(), text),
        Span::raw(" ".repeat(gap)),
        Span::styled(right, muted),
    ])
}

/// Ratatui putty footer line.
#[must_use]
pub fn putty_ratatui(hints: &str, width: usize, dim: Style) -> Line<'static> {
    Line::from(Span::styled(putty_footer_line(hints, width), dim))
}

/// Render a full conversation chrome fingerprint as plain text lines
/// (top bar, optional user/thought/body sample, optional busy band, jewel, putty).
///
/// Used by unit tests and tmux-facing snapshot helpers to lock VAL-CORTEX-CHROME-*.
#[must_use]
pub fn render_chrome_fingerprint(width: usize, busy: bool) -> String {
    let w = width.max(40);
    let mut out = vec![
        demo_auth_top_bar(w),
        String::new(),
        user_wash_line(
            "Explain the race in the flush root briefly.",
            Some("8:19 AM"),
            w,
        ),
        String::new(),
        thought_line("1.4s"),
        String::new(),
        "  Race structure".to_owned(),
        "  Two writers can skip the watched counter when the snapshot freezes.".to_owned(),
    ];
    if busy {
        out.push(String::new());
        out.push(turn_status_busy_line(
            "Responding…",
            "1.4s",
            "4.0s",
            "2.1k",
            w,
        ));
    }
    out.push(String::new());
    // Jewel border sketch (text fingerprint only).
    let inner_w = w.saturating_sub(4).max(20);
    let top = format!("  ╭{}╮", "─".repeat(inner_w.saturating_sub(2)));
    let bot = format!("  ╰{}╯", "─".repeat(inner_w.saturating_sub(2)));
    let draft_row = format!(
        "  │{}│",
        default_prompt_jewel_inner("", inner_w.saturating_sub(2))
    );
    out.push(top);
    out.push(draft_row);
    out.push(bot);
    out.push(String::new());
    out.push(default_putty_line(w));
    out.join("\n")
}

fn pad_left_right(left: &str, right: &str, width: usize) -> String {
    let lw = left.chars().count();
    let rw = right.chars().count();
    if width == 0 {
        return String::new();
    }
    if lw + rw >= width {
        // Prefer keeping the right chip fully visible when tight.
        let keep_left = width.saturating_sub(rw);
        let mut s: String = left.chars().take(keep_left).collect();
        s.push_str(right);
        return s.chars().take(width).collect();
    }
    let gap = width - lw - rw;
    format!("{left}{}{right}", " ".repeat(gap))
}

fn pad_right(left: &str, width: usize) -> String {
    let lw = left.chars().count();
    if lw >= width {
        return left.chars().take(width).collect();
    }
    format!("{left}{}", " ".repeat(width - lw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bans_you_and_assistant_nameplates() {
        let fp = render_chrome_fingerprint(120, true);
        assert!(!fp.contains("❯ You"), "ban You nameplate:\n{fp}");
        assert!(!fp.contains("> You"), "ban ASCII You nameplate:\n{fp}");
        assert!(!fp.contains("◆ Cortex"), "ban Cortex nameplate:\n{fp}");
        reject_structure_residual(&fp).expect("structure clean");
    }

    #[test]
    fn user_wash_is_bare_prompt_arrow() {
        let line = user_wash_line("Check the retry path before we merge.", None, 80);
        assert!(line.contains("❯ Check the retry path before we merge."));
        assert!(!line.contains("You"));
        reject_structure_residual(&line).unwrap();
    }

    #[test]
    fn thought_line_uses_diamond_not_agent_nameplate() {
        let line = thought_line("1.4s");
        assert_eq!(line, "  ◆ Thought for 1.4s");
        assert!(!line.contains("◆ Cortex"));
    }

    #[test]
    fn context_budget_form_matches_product_regex() {
        assert_eq!(format_context_budget("9.5K", "500K"), "9.5K / 500K");
        assert_eq!(format_context_budget("8.5K", "1.0M"), "8.5K / 1.0M");
        assert!(has_context_budget_form("  ~/demo                    9.5K / 500K  "));
        assert!(has_context_budget_form("8.5K / 1.0M"));
        assert!(!has_context_budget_form("branch only master"));
        assert!(!has_context_budget_form("9.5k / 500k")); // lowercase rejected by KM rule
        let top = demo_auth_top_bar(120);
        assert!(has_context_budget_form(&top), "top bar: {top:?}");
        assert!(top.contains(DEMO_CWD));
    }

    #[test]
    fn format_token_compact_uppercase_units() {
        assert_eq!(format_token_compact(0), "0");
        assert_eq!(format_token_compact(500), "500");
        assert_eq!(format_token_compact(1_000), "1K");
        assert_eq!(format_token_compact(9_500), "9.5K");
        assert_eq!(format_token_compact(500_000), "500K");
        assert_eq!(format_token_compact(1_000_000), "1M");
        assert_eq!(format_token_compact(1_500_000), "1.5M");
    }

    #[test]
    fn jewel_right_aligns_effort_and_mode() {
        let inner = default_prompt_jewel_inner("draft text", 100);
        assert!(inner.contains('❯'));
        assert!(inner.contains("draft text"));
        let chip = default_model_chip();
        assert_eq!(chip, "Cortex (high) · always-approve");
        assert!(inner.contains(&chip), "chip on draft row: {inner:?}");
        let draft_pos = inner.find("draft text").unwrap();
        let chip_pos = inner.find(&chip).unwrap();
        assert!(chip_pos > draft_pos, "chip right of draft");
        assert!(!inner.contains("Draft ·"));
    }

    #[test]
    fn putty_uses_pipe_separators_not_middot_only() {
        assert!(DEFAULT_PUTTY.contains(" | "));
        assert!(
            !DEFAULT_PUTTY.contains(" · "),
            "middot-only dynasty banned as primary"
        );
        let line = default_putty_line(120);
        assert!(line.contains("Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts"));
        assert!(putty_product_separators_hold(&line));
        assert!(!putty_product_separators_hold(
            "Shift+Tab:mode · Ctrl+c:cancel · Ctrl+.:shortcuts"
        ));
    }

    #[test]
    fn busy_turn_status_includes_stop_and_product_timer() {
        assert_eq!(format_turn_timer(1.4), "1.4s");
        assert_eq!(format_turn_timer(4.0), "4.0s");
        assert_eq!(format_turn_timer(15.0), "15s");
        assert_eq!(format_turn_timer(59.0), "59s");
        assert_eq!(format_turn_timer(60.0), "1m0s");
        assert_eq!(format_turn_timer(80.0), "1m20s");
        // Never emit 0mNNs for under 60s.
        let under = format_turn_timer(4.0);
        assert!(!under.starts_with("0m"), "got {under}");

        let line = turn_status_busy_line("Responding…", "1.4s", "4.0s", "2.1k", 120);
        assert!(line.contains("[stop]"), "{line}");
        assert!(line.contains("Responding… 1.4s"), "{line}");
        assert!(line.contains("4.0s ⇣2.1k"), "{line}");
        assert!(!line.contains("0m"));
        reject_structure_residual(&line).unwrap();
    }

    #[test]
    fn fingerprint_locks_full_product_chrome_dna() {
        let fp = render_chrome_fingerprint(120, true);
        assert!(has_context_budget_form(&fp), "budget missing:\n{fp}");
        assert!(fp.contains("❯ Explain the race"));
        assert!(fp.contains("◆ Thought for 1.4s"));
        assert!(fp.contains("Cortex (high) · always-approve"));
        assert!(fp.contains("[stop]"));
        assert!(fp.contains("Shift+Tab:mode | Ctrl+c:cancel | Ctrl+.:shortcuts"));
        assert!(putty_product_separators_hold(&fp));
        reject_structure_residual(&fp).expect("structure");
        reject_copper_residual(&fp).expect("no copper");
        assert!(!fp.contains("Cortex Code"));
        assert!(!fp.contains("Code ·"));
    }

    #[test]
    fn copper_code_brand_rejected_in_chrome() {
        assert!(reject_copper_residual("hello Cortex").is_ok());
        assert!(reject_copper_residual("✦ Cortex Code · chat").is_err());
        assert!(reject_copper_residual("Code · rendering").is_err());
        assert!(reject_copper_residual("   Code   ").is_err());
    }

    #[test]
    fn cortexnight_palette_hex_identity() {
        assert_eq!(CortexNightPalette::canvas_hex(), "#141414");
        assert_eq!(CortexNightPalette::text_hex(), "#e1e1e1");
        assert_eq!(CortexNightPalette::accent_hex(), "#bb9af7");
        assert_ne!(
            CortexNightPalette::accent_hex().to_lowercase(),
            "#d9825f",
            "accent must not be copper"
        );
        assert_eq!(CortexNightPalette::ACCENT, Color::Rgb(0xbb, 0x9a, 0xf7));
        assert_eq!(CortexNightPalette::CANVAS, Color::Rgb(0x14, 0x14, 0x14));
    }

    #[test]
    fn idle_fingerprint_omits_stop_band() {
        let fp = render_chrome_fingerprint(120, false);
        assert!(!fp.contains("[stop]"));
        assert!(fp.contains("Cortex (high) · always-approve"));
        assert!(has_context_budget_form(&fp));
    }
}
