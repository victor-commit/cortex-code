//! Shared visual chrome for every mockup screen - the Cortex Code v2 design
//! system in code (architecture 3.1-3.4).
//!
//! This module is the single source of truth for the palette, the closed
//! glyph set, and the shared primitives. Screens compose these helpers and
//! NEVER use raw `Color::Rgb`; re-theming the gallery means editing this file
//! only. Design intent: Claude Code-grade restraint - chrome is always gray,
//! green/red are verdicts only, ONE bordered zone per screen, the copper
//! accent is budgeted (<=~3 elements, one per region).

// The module carries the full v2 primitive contract up front; screens adopt
// helpers incrementally as the gallery grows, so unused items are expected.
#![allow(dead_code)]

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

// ============================================================================
// PALETTE (architecture 3.1) - 13 semantic tokens, nothing else.
// State is always glyph + color, never color alone. One emphasis channel at a
// time (bold OR color OR bg). Red is reserved for error/deletion.
// ============================================================================

/// Background everywhere.
pub const CANVAS: Color = Color::Rgb(13, 13, 13); // #0d0d0d
/// Copper brand accent: wordmark, spinner, active mode badge, focus/selection
/// marker `›`. Budget: <=5% of visible cells, ONE accent element per region.
pub const ACCENT: Color = Color::Rgb(217, 130, 95); // #D9825F
/// Verdicts only: tool success dot/check, diff additions, pass, connected.
pub const OK: Color = Color::Rgb(127, 176, 105); // #7FB069
/// Verdicts only: failures, diff deletions, blocking errors. NEVER selection.
pub const BAD: Color = Color::Rgb(217, 108, 117); // #D96C75
/// Rare: degraded/attention states (rate-limit, near-full context).
pub const WARN: Color = Color::Rgb(217, 179, 108); // #D9B36C
/// Rare: links, informational badges (plan-mode badge).
pub const INFO: Color = Color::Rgb(108, 155, 217); // #6C9BD9
/// Headers, focused row text, key data.
pub const BRIGHT: Color = Color::Rgb(230, 233, 239); // #E6E9EF
/// Default body text.
pub const TXT: Color = Color::Rgb(200, 205, 214); // #C8CDD6
/// Secondary text: hints, counters, metadata, timestamps.
pub const MUTED: Color = Color::Rgb(154, 163, 175); // #9AA3AF
/// Borders, rules `─`, disabled, ghost text.
pub const FAINT: Color = Color::Rgb(76, 85, 102); // #4C5566
/// Fill bg ONLY for the selected list row / current plan step (rule N-1:
/// statusline and cards keep the CANVAS bg).
pub const SURFACE: Color = Color::Rgb(38, 43, 51); // #262B33
/// Diff added-line background wash.
pub const ADD_BG: Color = Color::Rgb(38, 51, 31); // #26331F
/// Diff removed-line background wash.
pub const DEL_BG: Color = Color::Rgb(51, 33, 31); // #33211F

// ============================================================================
// GLYPHS (architecture 3.2) - the closed functional set. All single-width and
// DejaVu-safe: layout math is `chars().count()`, double-width chars break it.
// ============================================================================

/// Prompt / selection marker (never a filled arrow).
pub const SELECT_MARKER: &str = "›";
/// Inline separator (also stands in for the forbidden em dash).
pub const SEP: &str = "·";
/// Filled progress cell.
pub const PROGRESS_FULL: &str = "█";
/// Empty progress cell (drawn FAINT).
pub const PROGRESS_EMPTY: &str = "░";
/// Result elbow: attaches tool output under its one-line tool call.
pub const ELBOW: &str = "⎿";
/// State dot (paired with a word label or an outcome, never color alone).
pub const DOT: &str = "●";
/// Pending / empty dot.
pub const DOT_EMPTY: &str = "○";
/// Disclosure marker, collapsed.
pub const DISCLOSURE_CLOSED: &str = "▸";
/// Disclosure marker, expanded.
pub const DISCLOSURE_OPEN: &str = "▾";

// ============================================================================
// SPAN VOCABULARY - tiny constructors the screens build lines from.
// ============================================================================

/// Body-text span (TXT).
pub fn t(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(TXT))
}

/// Muted span (MUTED).
pub fn m(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(MUTED))
}

/// Colored span.
pub fn c(s: &str, color: Color) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(color))
}

/// Bold colored span (use sparingly: one emphasis channel at a time).
pub fn cb(s: &str, color: Color) -> Span<'static> {
    Span::styled(
        s.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// State dot span in the given hue. Pair it with a word - never color alone.
pub fn dot(state_color: Color) -> Span<'static> {
    Span::styled(DOT.to_string(), Style::default().fg(state_color))
}

/// Inline status: `● label`, dot and label in the state hue (glyph + color).
pub fn status(state_color: Color, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{DOT} "), Style::default().fg(state_color)),
        Span::styled(label.to_string(), Style::default().fg(state_color)),
    ]
}

/// Info row: `  label: value` with a muted label; empty label = continuation.
pub fn info(label: &str, value: &str, value_color: Color) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    if !label.is_empty() {
        spans.push(Span::styled(
            format!("{label}: "),
            Style::default().fg(MUTED),
        ));
    }
    spans.push(Span::styled(
        value.to_string(),
        Style::default().fg(value_color),
    ));
    Line::from(spans)
}

// ============================================================================
// TIME (pattern 9) - ONE duration formatter used everywhere.
// ============================================================================

/// Duration grammar: `0.4s`, `12s`, `1m04s`, `2h10m`. Bare seconds appear
/// only below 60; minutes zero-pad seconds; hours drop seconds entirely.
/// Relative ages (`2h ago`, `yesterday`) and wall-clock `HH:MM` are distinct
/// grammars written literally by screens - this is for durations only.
pub fn timefmt(secs: f64) -> String {
    if secs < 60.0 {
        if secs < 10.0 && secs.fract() > f64::EPSILON {
            format!("{secs:.1}s")
        } else {
            format!("{}s", secs as u64)
        }
    } else if secs < 3600.0 {
        let t = secs as u64;
        format!("{}m{:02}s", t / 60, t % 60)
    } else {
        let t = secs as u64;
        format!("{}h{:02}m", t / 3600, (t % 3600) / 60)
    }
}

// ============================================================================
// BARS - progress / usage bars `███░░░` (glyph set `█ ░`).
// ============================================================================

/// Ratio bar `width` cells wide: filled cells in TXT, empty track in FAINT.
pub fn bar(filled: usize, total: usize, width: usize) -> Vec<Span<'static>> {
    bar_in(filled, total, width, TXT)
}

/// Ratio bar with an explicit fill hue (WARN for near-full context, etc.).
pub fn bar_in(filled: usize, total: usize, width: usize, color: Color) -> Vec<Span<'static>> {
    let cells = (filled.min(total) * width + total / 2)
        .checked_div(total)
        .unwrap_or(0);
    vec![
        Span::styled(PROGRESS_FULL.repeat(cells), Style::default().fg(color)),
        Span::styled(
            PROGRESS_EMPTY.repeat(width - cells),
            Style::default().fg(FAINT),
        ),
    ]
}

// ============================================================================
// TOOL CALLS (pattern 1) - one line + elbow result. Never a box.
// ============================================================================

/// Tool-call outcome; the bullet carries it as color (plus the elbow content
/// as words, keeping the glyph+color rule).
pub enum ToolStatus {
    /// In flight. `Some(secs)` appends the elapsed time; pass `None` when a
    /// working band already announces the activity - elapsed renders exactly
    /// once per activity (pattern 5).
    Running(Option<f64>),
    Done,
    Failed,
}

/// ` ● Name(args)` - dot MUTED while running, OK when done, BAD when failed.
/// Empty `args` keep a plain one-line semantic header, for pending proposals
/// whose path belongs in the label rather than function-style parentheses.
pub fn tool_line(status: ToolStatus, name: &str, args: &str) -> Line<'static> {
    let (hue, elapsed) = match status {
        ToolStatus::Running(e) => (MUTED, e),
        ToolStatus::Done => (OK, None),
        ToolStatus::Failed => (BAD, None),
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{DOT} "), Style::default().fg(hue)),
        Span::styled(name.to_string(), Style::default().fg(BRIGHT)),
    ];
    if !args.is_empty() {
        spans.push(Span::styled(format!("({args})"), Style::default().fg(MUTED)));
    }
    if let Some(secs) = elapsed {
        spans.push(Span::styled(
            format!(" {SEP} {}", timefmt(secs)),
            Style::default().fg(MUTED),
        ));
    }
    Line::from(spans)
}

/// Shell command line with a shared status column. Finished commands pair
/// their OK dot with `✓ exit 0`; running commands expose their elapsed time
/// in the same column so the outcome remains readable without color.
pub enum ShellStatus {
    Done(f64),
    Running(f64),
}

/// ` ● ! command                                  ✓ exit 0 · 0.2s`
///
/// Shell commands follow the tool-line plus elbow anatomy, but keep their
/// outcome at one fixed scan column across completed and running commands.
pub fn shell_line(status: ShellStatus, command: &str) -> Line<'static> {
    const STATUS_COL: usize = 82;

    let (dot_hue, done, elapsed) = match status {
        ShellStatus::Done(secs) => (OK, true, secs),
        ShellStatus::Running(secs) => (MUTED, false, secs),
    };
    let used = 4 + command.chars().count();
    let pad = " ".repeat(STATUS_COL.saturating_sub(used).max(2));
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{DOT} "), Style::default().fg(dot_hue)),
        Span::styled("! ".to_string(), Style::default().fg(MUTED)),
        Span::styled(command.to_string(), Style::default().fg(BRIGHT)),
        Span::raw(pad),
    ];
    if done {
        spans.push(Span::styled("✓".to_string(), Style::default().fg(OK)));
        spans.push(Span::styled(" exit 0 ".to_string(), Style::default().fg(TXT)));
    } else {
        spans.push(Span::styled(
            "running ".to_string(),
            Style::default().fg(BRIGHT),
        ));
    }
    spans.push(Span::styled(
        format!("{SEP} {}", timefmt(elapsed)),
        Style::default().fg(MUTED),
    ));
    Line::from(spans)
}

/// Attach result rows under a tool line: the first row gets the `⎿` elbow,
/// continuation rows indent to the same content column.
///
/// ```text
///  ● Read(crates/tui/src/app.rs)
///    ⎿ 214 lines
/// ```
pub fn elbow(rows: Vec<Vec<Span<'static>>>) -> Vec<Line<'static>> {
    rows.into_iter()
        .enumerate()
        .map(|(i, spans)| {
            let mut out = if i == 0 {
                vec![
                    Span::raw("   "),
                    Span::styled(format!("{ELBOW} "), Style::default().fg(FAINT)),
                ]
            } else {
                vec![Span::raw("     ")]
            };
            out.extend(spans);
            Line::from(out)
        })
        .collect()
}

/// Convenience: a single muted elbow row (`⎿ 214 lines`).
pub fn elbow_note(text: &str) -> Vec<Line<'static>> {
    elbow(vec![vec![m(text)]])
}

/// Verdict lead for elbow rows: `✓ text` (OK) or `✗ text` (BAD). The glyph
/// carries the outcome color, the text stays TXT - glyph + color, and only
/// one emphasis channel. Append muted meta (` · exit 0 · 0.2s`) after it.
pub fn verdict(ok: bool, text: &str) -> Vec<Span<'static>> {
    let (glyph, hue) = if ok { ("✓", OK) } else { ("✗", BAD) };
    vec![
        Span::styled(format!("{glyph} "), Style::default().fg(hue)),
        Span::styled(text.to_string(), Style::default().fg(TXT)),
    ]
}

/// Inline key-hint spans in the footer grammar (key MUTED, label FAINT):
/// recovery affordances like `r retry` attached to a failed block.
pub fn key_span(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(key.to_string(), Style::default().fg(MUTED)),
        Span::styled(format!(" {label}"), Style::default().fg(FAINT)),
    ]
}

/// Failure-escalation rows (pattern 8): rows of a failed block carry a BAD
/// `│` left bar aligned under the `⎿` elbow, content on the elbow's text
/// column. Put the verdict on the elbow row above; the recovery keys
/// (`key_span`) belong in the block as its last row.
pub fn fail_rows(rows: Vec<Vec<Span<'static>>>) -> Vec<Line<'static>> {
    rows.into_iter()
        .map(|spans| {
            let mut out = vec![
                Span::raw("   "),
                Span::styled("\u{2502} ".to_string(), Style::default().fg(BAD)),
            ];
            out.extend(spans);
            Line::from(out)
        })
        .collect()
}

/// Failure header for an inline transcript block: the BAD bar and `✗` begin
/// at the same columns as `fail_rows`, so the entire failure reads as one
/// contiguous region without a box.
pub fn fail_head(label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled("│ ".to_string(), Style::default().fg(BAD)),
        Span::styled("✗ ".to_string(), Style::default().fg(BAD)),
        Span::styled(label.to_string(), Style::default().fg(TXT)),
    ])
}

// ============================================================================
// DIFFS (pattern 6) - tinted background lines, ASCII +/- markers (never
// U+2212), line numbers. The wash only shows in the PNG, not the .txt.
// ============================================================================

/// One diff row for `diff_lines`.
pub enum DiffRow {
    Ctx(&'static str),
    Add(&'static str),
    Del(&'static str),
}

/// Fixed wash band width: rows span cols 3..=110 on the 120-col canvas.
const DIFF_BAND: usize = 108;

/// Render a hunk: `  <old> <new> <sign> <code>` per row. Added rows wash
/// ADD_BG with an OK `+`, removed rows wash DEL_BG with a BAD `-`, context
/// rows stay muted on the canvas. Gutter numbers advance automatically.
pub fn diff_lines(old_start: u32, new_start: u32, rows: &[DiffRow]) -> Vec<Line<'static>> {
    let mut old = old_start;
    let mut new = new_start;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let (o, n, sign, code, code_hue, wash) = match row {
            DiffRow::Ctx(code) => {
                let r = (Some(old), Some(new), ' ', *code, MUTED, None);
                old += 1;
                new += 1;
                r
            }
            DiffRow::Add(code) => {
                let r = (None, Some(new), '+', *code, TXT, Some(ADD_BG));
                new += 1;
                r
            }
            DiffRow::Del(code) => {
                let r = (Some(old), None, '-', *code, TXT, Some(DEL_BG));
                old += 1;
                r
            }
        };
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            match wash {
                Some(b) => s.bg(b),
                None => s,
            }
        };
        let gut = |v: Option<u32>| match v {
            Some(v) => format!("{v:>4}"),
            None => " ".repeat(4),
        };
        let sign_hue = match sign {
            '+' => OK,
            '-' => BAD,
            _ => MUTED,
        };
        let used = 4 + 1 + 4 + 1 + 1 + 1 + code.chars().count();
        let pad = DIFF_BAND.saturating_sub(used);
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(gut(o), sty(FAINT)),
            Span::styled(" ".to_string(), sty(FAINT)),
            Span::styled(gut(n), sty(FAINT)),
            Span::styled(" ".to_string(), sty(FAINT)),
            Span::styled(sign.to_string(), sty(sign_hue)),
            Span::styled(" ".to_string(), sty(code_hue)),
            Span::styled(code.to_string(), sty(code_hue)),
            Span::styled(" ".repeat(pad), sty(code_hue)),
        ]));
    }
    out
}

// ============================================================================
// SELECTION LISTS (pattern 7) - SURFACE wash + ACCENT `›` + BRIGHT text on
// the selected row; fixed name / detail / meta columns; optional filter line
// (REQUIRED when the list is longer than 8 rows).
// ============================================================================

/// Render an aligned selection list. `rows` are `(name, detail, meta)`;
/// `width` is the full row width to wash and right-align meta against
/// (pass the target Rect's width). `filter: Some("")` shows the ghost prompt.
pub fn select_list(
    width: usize,
    rows: &[(&str, &str, &str)],
    selected: usize,
    filter: Option<&str>,
) -> Vec<Line<'static>> {
    let name_w = rows
        .iter()
        .map(|(n, _, _)| n.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(rows.len() + 1);
    if let Some(fil) = filter {
        let text = if fil.is_empty() {
            Span::styled("type to filter".to_string(), Style::default().fg(FAINT))
        } else {
            Span::styled(fil.to_string(), Style::default().fg(TXT))
        };
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("/ ".to_string(), Style::default().fg(MUTED)),
            text,
        ]));
    }
    for (i, (name, detail, meta)) in rows.iter().enumerate() {
        let sel = i == selected;
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            if sel {
                s.bg(SURFACE)
            } else {
                s
            }
        };
        let marker = if sel {
            Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
        } else {
            Span::styled("   ".to_string(), sty(TXT))
        };
        let name_hue = if sel { BRIGHT } else { TXT };
        let detail_hue = if sel { TXT } else { MUTED };
        let meta_hue = if sel { MUTED } else { FAINT };
        let name_cell = format!("{name:<w$}  ", w = name_w);
        let used = 3 + name_cell.chars().count() + detail.chars().count();
        let pad = width
            .saturating_sub(used + meta.chars().count() + 1)
            .max(1);
        out.push(Line::from(vec![
            marker,
            Span::styled(name_cell, sty(name_hue)),
            Span::styled(detail.to_string(), sty(detail_hue)),
            Span::styled(" ".repeat(pad), sty(detail_hue)),
            Span::styled(meta.to_string(), sty(meta_hue)),
            Span::styled(" ".to_string(), sty(meta_hue)),
        ]));
    }
    out
}

/// One numbered choice in a permission dock.
pub struct PermissionChoice {
    pub label: &'static str,
    pub detail: &'static str,
}

/// Render numbered permission choices with the shared neutral selection
/// treatment. Numbers stay muted even for the selected choice so red and
/// green remain reserved for verdicts.
pub fn permission_choices(
    width: usize,
    choices: &[PermissionChoice],
    selected: usize,
) -> Vec<Line<'static>> {
    let label_w = choices
        .iter()
        .map(|choice| choice.label.chars().count())
        .max()
        .unwrap_or(0);
    choices
        .iter()
        .enumerate()
        .map(|(i, choice)| {
            let active = i == selected;
            let sty = |hue: Color| {
                let style = Style::default().fg(hue);
                if active {
                    style.bg(SURFACE)
                } else {
                    style
                }
            };
            let marker = if active {
                Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
            } else {
                Span::styled("   ".to_string(), sty(TXT))
            };
            let number = format!("{} ", i + 1);
            let label = format!("{:<label_w$}", choice.label);
            let label_hue = if active { BRIGHT } else { TXT };
            let detail_hue = if active { TXT } else { MUTED };
            let used = 3 + number.chars().count() + label.chars().count() + 2
                + choice.detail.chars().count();
            let pad = width.saturating_sub(used + 1).max(1);
            Line::from(vec![
                marker,
                Span::styled(number, sty(MUTED)),
                Span::styled(label, sty(label_hue)),
                Span::styled("  ".to_string(), sty(detail_hue)),
                Span::styled(choice.detail.to_string(), sty(detail_hue)),
                Span::styled(" ".repeat(pad), sty(detail_hue)),
            ])
        })
        .collect()
}

/// One skills-picker row. The source column makes skills loaded for the
/// current session distinct from discoverable but inactive skills, while the
/// rightmost state stays readable with color removed.
pub struct ToggleListRow {
    pub name: &'static str,
    pub detail: &'static str,
    pub source: &'static str,
    pub enabled: bool,
}

/// Render a selection list whose final column is an explicit on/off toggle.
/// This keeps selection treatment, semantic state color, and right-aligned
/// toggle anatomy in one shared primitive for settings-style pickers.
pub fn toggle_select_list(
    width: usize,
    rows: &[ToggleListRow],
    selected: usize,
    filter: Option<&str>,
) -> Vec<Line<'static>> {
    let name_w = rows
        .iter()
        .map(|row| row.name.chars().count())
        .max()
        .unwrap_or(0);
    let source_w = rows
        .iter()
        .map(|row| row.source.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(rows.len() + usize::from(filter.is_some()));
    if let Some(query) = filter {
        out.push(list_filter("/", query, "filter skills"));
    }
    for (i, row) in rows.iter().enumerate() {
        let sel = i == selected;
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            if sel {
                s.bg(SURFACE)
            } else {
                s
            }
        };
        let name_hue = if sel { BRIGHT } else { TXT };
        let detail_hue = if sel { TXT } else { MUTED };
        let source_hue = if row.enabled { MUTED } else { FAINT };
        let marker = if sel {
            Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
        } else {
            Span::styled("   ".to_string(), sty(TXT))
        };
        let name_cell = format!("{:<name_w$}  ", row.name);
        let source_cell = format!("{:<source_w$}  ", row.source);
        let state_word = if row.enabled { "on" } else { "off" };
        let state_glyph = if row.enabled { "●" } else { "○" };
        let state_hue = if row.enabled { OK } else { FAINT };
        let used = 3
            + name_cell.chars().count()
            + row.detail.chars().count()
            + source_cell.chars().count()
            + state_glyph.chars().count()
            + 1
            + state_word.chars().count();
        let pad = width.saturating_sub(used + 1).max(1);
        out.push(Line::from(vec![
            marker,
            Span::styled(name_cell, sty(name_hue)),
            Span::styled(row.detail.to_string(), sty(detail_hue)),
            Span::styled(" ".repeat(pad), sty(detail_hue)),
            Span::styled(source_cell, sty(source_hue)),
            Span::styled(format!("{state_glyph} "), sty(state_hue)),
            Span::styled(state_word.to_string(), sty(state_hue)),
            Span::styled(" ".to_string(), sty(state_hue)),
        ]));
    }
    out
}

/// One row of a session picker. Age and message count are deliberately
/// separate fields so their right-aligned columns never drift as text varies.
pub struct SessionListRow {
    pub title: &'static str,
    pub age: &'static str,
    pub messages: &'static str,
    pub matches_query: bool,
}

/// Render session rows with fixed title, match, age, and message-count
/// columns. A textual match badge makes the typed query's corresponding row
/// observable without introducing a second selection convention.
pub fn session_select_list(
    width: usize,
    rows: &[SessionListRow],
    selected: usize,
    query: &str,
) -> Vec<Line<'static>> {
    const MATCH_W: usize = 7;
    const AGE_W: usize = 11;
    const MSG_W: usize = 9;

    let title_w = rows
        .iter()
        .map(|row| row.title.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = vec![list_filter("/", query, "filter sessions")];
    for (i, row) in rows.iter().enumerate() {
        let sel = i == selected;
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            if sel {
                s.bg(SURFACE)
            } else {
                s
            }
        };
        let marker = if sel {
            Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
        } else {
            Span::styled("   ".to_string(), sty(TXT))
        };
        let title_hue = if sel { BRIGHT } else { TXT };
        let detail_hue = if sel { MUTED } else { FAINT };
        let title_cell = format!("{:<title_w$}  ", row.title);
        let match_cell = if row.matches_query { "● match" } else { "" };
        let match_cell = format!("{match_cell:<MATCH_W$}");
        let age_cell = format!("{:<AGE_W$}", row.age);
        let message_cell = format!("{:<MSG_W$}", row.messages);
        let used = 3
            + title_cell.chars().count()
            + match_cell.chars().count()
            + age_cell.chars().count()
            + message_cell.chars().count();
        let pad = width.saturating_sub(used + 1).max(1);
        out.push(Line::from(vec![
            marker,
            Span::styled(title_cell, sty(title_hue)),
            Span::styled(match_cell, sty(INFO)),
            Span::styled(" ".repeat(pad), sty(detail_hue)),
            Span::styled(age_cell, sty(MUTED)),
            Span::styled(message_cell, sty(MUTED)),
            Span::styled(" ".to_string(), sty(MUTED)),
        ]));
    }
    out
}

/// One fuzzy file row, split around the matched substring so it can receive
/// the stronger BRIGHT bold treatment without hiding the remainder of the
/// path.
pub struct FuzzyFileRow {
    pub before: &'static str,
    pub matched: &'static str,
    pub after: &'static str,
    pub meta: &'static str,
}

/// Render file matches with the shared selection treatment and a right-aligned
/// `dir · size` meta column. Matched filename characters are BRIGHT and bold
/// in every row, making the fuzzy query legible at a glance.
pub fn fuzzy_file_list(
    width: usize,
    rows: &[FuzzyFileRow],
    selected: usize,
) -> Vec<Line<'static>> {
    let name_w = rows
        .iter()
        .map(|row| row.before.len() + row.matched.len() + row.after.len())
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let sel = i == selected;
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            if sel {
                s.bg(SURFACE)
            } else {
                s
            }
        };
        let name_hue = if sel { BRIGHT } else { TXT };
        let name_len = row.before.len() + row.matched.len() + row.after.len();
        let name_pad = " ".repeat(name_w.saturating_sub(name_len) + 2);
        let used = 3 + name_w + 2 + row.meta.chars().count();
        let pad = width.saturating_sub(used + 1).max(1);
        let marker = if sel {
            Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
        } else {
            Span::styled("   ".to_string(), sty(TXT))
        };
        out.push(Line::from(vec![
            marker,
            Span::styled(row.before.to_string(), sty(name_hue)),
            Span::styled(
                row.matched.to_string(),
                sty(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(row.after.to_string(), sty(name_hue)),
            Span::styled(name_pad, sty(name_hue)),
            Span::styled(" ".repeat(pad), sty(name_hue)),
            Span::styled(row.meta.to_string(), sty(MUTED)),
            Span::styled(" ".to_string(), sty(MUTED)),
        ]));
    }
    out
}

/// One queued prompt row. The visible position is a separate fixed-width
/// column so the queue order remains scannable when prompts have different
/// lengths.
pub struct NumberedListRow {
    pub prompt: &'static str,
    pub meta: &'static str,
}

/// Render numbered queue rows with the normal list selection wash. The active
/// row makes both its marker and its position copper so reordering has an
/// obvious target without using a second border or a status color.
pub fn numbered_select_list(
    width: usize,
    rows: &[NumberedListRow],
    selected: usize,
) -> Vec<Line<'static>> {
    let number_w = rows.len().to_string().chars().count();
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let sel = i == selected;
        let sty = |hue: Color| {
            let s = Style::default().fg(hue);
            if sel {
                s.bg(SURFACE)
            } else {
                s
            }
        };
        let meta_hue = if sel { MUTED } else { FAINT };
        let used = 3 + number_w + 2 + row.prompt.chars().count();
        let pad = width
            .saturating_sub(used + row.meta.chars().count() + 1)
            .max(1);
        let marker = if sel {
            Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
        } else {
            Span::styled("   ".to_string(), sty(TXT))
        };
        let number_hue = if sel { ACCENT } else { MUTED };
        let prompt_hue = if sel { BRIGHT } else { TXT };
        out.push(Line::from(vec![
            marker,
            Span::styled(format!("{:>number_w$}", i + 1), sty(number_hue)),
            Span::styled("  ".to_string(), sty(prompt_hue)),
            Span::styled(row.prompt.to_string(), sty(prompt_hue)),
            Span::styled(" ".repeat(pad), sty(prompt_hue)),
            Span::styled(row.meta.to_string(), sty(meta_hue)),
            Span::styled(" ".to_string(), sty(meta_hue)),
        ]));
    }
    out
}

/// A row for a sectioned selection list. Section labels are chrome, so they
/// always render in FAINT; items retain the shared selection treatment.
pub enum ListRow {
    Section(&'static str),
    Item {
        name: &'static str,
        detail: &'static str,
        meta: &'static str,
    },
    /// A dedicated-key action, visually grouped with a section but excluded
    /// from the arrow-key selection cycle.
    Action {
        name: &'static str,
        detail: &'static str,
        meta: &'static str,
    },
}

/// One list filter row. The marker is intentionally neutral so the ACCENT
/// marker remains exclusive to the active selection.
pub fn list_filter(prompt: &str, query: &str, placeholder: &str) -> Line<'static> {
    let text = if query.is_empty() {
        Span::styled(placeholder.to_string(), Style::default().fg(FAINT))
    } else {
        Span::styled(query.to_string(), Style::default().fg(TXT))
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{prompt} "), Style::default().fg(MUTED)),
        text,
    ])
}

/// A right-aligned MUTED result count for a list or palette.
pub fn list_counter(width: usize, note: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(width.saturating_sub(note.chars().count()))),
        Span::styled(note.to_string(), Style::default().fg(MUTED)),
    ])
}

/// Render a selection list with FAINT-only section separators. `selected`
/// indexes items only, never section labels. This is the shared list anatomy
/// for long command menus where grouping improves scanning without introducing
/// an extra card or semantic color.
pub fn sectioned_select_list(
    width: usize,
    rows: &[ListRow],
    selected: usize,
    filter: Option<&str>,
) -> Vec<Line<'static>> {
    let name_w = rows
        .iter()
        .filter_map(|row| match row {
            ListRow::Item { name, .. } | ListRow::Action { name, .. } => {
                Some(name.chars().count())
            }
            ListRow::Section(_) => None,
        })
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(rows.len() + usize::from(filter.is_some()));
    if let Some(fil) = filter {
        out.push(list_filter("/", fil, "type to filter"));
    }
    let mut item_index = 0;
    for row in rows {
        match row {
            ListRow::Section(title) => {
                let label = format!("  {title} ");
                let rule = "─".repeat(width.saturating_sub(label.chars().count()));
                out.push(Line::from(vec![
                    Span::styled(label, Style::default().fg(FAINT)),
                    Span::styled(rule, Style::default().fg(FAINT)),
                ]));
            }
            ListRow::Item { name, detail, meta } => {
                let selected_item = item_index == selected;
                item_index += 1;
                let sty = |hue: Color| {
                    let s = Style::default().fg(hue);
                    if selected_item {
                        s.bg(SURFACE)
                    } else {
                        s
                    }
                };
                let marker = if selected_item {
                    Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
                } else {
                    Span::styled("   ".to_string(), sty(TXT))
                };
                let name_hue = if selected_item { BRIGHT } else { TXT };
                let detail_hue = if selected_item { TXT } else { MUTED };
                let meta_hue = if selected_item { MUTED } else { FAINT };
                let name_cell = format!("{name:<w$}  ", w = name_w);
                let used = 3 + name_cell.chars().count() + detail.chars().count();
                let pad = width
                    .saturating_sub(used + meta.chars().count() + 1)
                    .max(1);
                out.push(Line::from(vec![
                    marker,
                    Span::styled(name_cell, sty(name_hue)),
                    Span::styled(detail.to_string(), sty(detail_hue)),
                    Span::styled(" ".repeat(pad), sty(detail_hue)),
                    Span::styled(meta.to_string(), sty(meta_hue)),
                    Span::styled(" ".to_string(), sty(meta_hue)),
                ]));
            }
            ListRow::Action { name, detail, meta } => {
                let name_cell = format!("{name:<w$}  ", w = name_w);
                let used = 3 + name_cell.chars().count() + detail.chars().count();
                let pad = width
                    .saturating_sub(used + meta.chars().count() + 1)
                    .max(1);
                out.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(name_cell, Style::default().fg(MUTED)),
                    Span::styled(detail.to_string(), Style::default().fg(FAINT)),
                    Span::styled(" ".repeat(pad), Style::default().fg(FAINT)),
                    Span::styled(meta.to_string(), Style::default().fg(MUTED)),
                    Span::styled(" ".to_string(), Style::default().fg(MUTED)),
                ]));
            }
        }
    }
    out
}

// ============================================================================
// STEP LISTS - onboarding / plan-progress rows: state glyph + label, `·`
// leader dots, and a right-aligned status-label column (audit: the status
// labels align at one column across the 120-col width). The current row is
// marked by glyph + SURFACE wash + BRIGHT label, never color alone.
// ============================================================================

/// State of one step row.
pub enum StepState {
    Done,
    /// Current onboarding step, using the shared selection marker.
    Current,
    /// Current transcript plan item, using the required state dot.
    CurrentDot,
    Pending,
}

/// One leader-dot step row: ` <glyph> <label>  <detail> ···· <status>`.
/// `label` pads to `label_w` and `status` left-aligns in a fixed `status_w`
/// column at the right edge so the columns line up across rows; FAINT `·`
/// dots fill the gap. Current rows use a BRIGHT label and may use a SURFACE
/// wash as their extra emphasis channel, depending on the state marker.
pub fn step_row(
    width: usize,
    state: StepState,
    label: &str,
    label_w: usize,
    detail: &str,
    status: &str,
    status_w: usize,
) -> Line<'static> {
    let (glyph, glyph_hue, label_hue, detail_hue, status_hue, wash) = match state {
        StepState::Done => ("✓", OK, TXT, MUTED, MUTED, None),
        StepState::Current => (SELECT_MARKER, ACCENT, BRIGHT, TXT, BRIGHT, Some(SURFACE)),
        StepState::CurrentDot => (DOT, ACCENT, BRIGHT, TXT, BRIGHT, None),
        StepState::Pending => (DOT_EMPTY, MUTED, MUTED, MUTED, MUTED, None),
    };
    let sty = |hue: Color| {
        let s = Style::default().fg(hue);
        match wash {
            Some(b) => s.bg(b),
            None => s,
        }
    };
    let label_cell = format!("{label:<label_w$}  ");
    let used = 3 + label_cell.chars().count() + detail.chars().count() + 2 + status_w;
    let dots = width.saturating_sub(used);
    Line::from(vec![
        Span::styled(" ".to_string(), sty(TXT)),
        Span::styled(format!("{glyph} "), sty(glyph_hue)),
        Span::styled(label_cell, sty(label_hue)),
        Span::styled(detail.to_string(), sty(detail_hue)),
        Span::styled(" ".to_string(), sty(FAINT)),
        Span::styled(SEP.repeat(dots), sty(FAINT)),
        Span::styled(" ".to_string(), sty(FAINT)),
        Span::styled(format!("{status:<status_w$}"), sty(status_hue)),
    ])
}

// ============================================================================
// WORKING BAND (patterns 5/10) - the ONE canonical status band, drawn above
// the composer. A screen shows at most one; personality lives in the verb.
// ============================================================================

/// ` ● Distilling… 12s · 3.1k tokens · esc to interrupt` - single ACCENT
/// spinner glyph, one working verb, dim counters.
pub fn working_band(verb: &str, elapsed: f64, tokens: Option<&str>) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(format!("{DOT} "), Style::default().fg(ACCENT)),
        Span::styled(format!("{verb}… "), Style::default().fg(TXT)),
        Span::styled(timefmt(elapsed), Style::default().fg(MUTED)),
    ];
    if let Some(tok) = tokens {
        spans.push(Span::styled(
            format!(" {SEP} {tok}"),
            Style::default().fg(MUTED),
        ));
    }
    spans.push(Span::styled(
        format!(" {SEP} esc to interrupt"),
        Style::default().fg(FAINT),
    ));
    Line::from(spans)
}

// ============================================================================
// STATUSLINE (pattern 3, rule N-8) - dim gray, NO filled background (rule
// N-1), and MINIMAL: `~/path · branch · <Model Display Name>` and nothing
// else. No ctx %, no cost, no state dot - those live in /status. Liveness is
// conveyed by the working band / on-screen content, never by the statusline.
// ============================================================================

/// Statusline content. Left: `path · branch · model`; right: the single
/// optional element, a mode badge shown only when a non-default mode is
/// active.
pub struct StatusLine {
    pub path: &'static str,
    pub branch: &'static str,
    /// Human display name ("Claude Opus 4.8"), NEVER a raw id like
    /// `claude-opus` (rule N-10/F-7) - here, in pickers, and in /status alike.
    pub model: &'static str,
    /// Mode badge in the fixed right-side slot: `("plan", INFO)` is the
    /// single INFO badge; any elevated/active mode badge is ACCENT.
    pub badge: Option<(&'static str, Color)>,
}

impl Default for StatusLine {
    fn default() -> Self {
        Self {
            path: "~/projects/cortex-code",
            branch: "master",
            model: "Claude Opus 4.8",
            badge: None,
        }
    }
}

/// Draw the statusline into `area` (one row, canvas background).
pub fn statusbar(f: &mut Frame, area: Rect, s: &StatusLine) {
    let sep = || Span::styled(format!(" {SEP} "), Style::default().fg(FAINT));
    let seg = |v: &str| Span::styled(v.to_string(), Style::default().fg(MUTED));
    let left = vec![
        Span::raw(" "),
        seg(s.path),
        sep(),
        seg(s.branch),
        sep(),
        seg(s.model),
    ];
    f.render_widget(Paragraph::new(Line::from(left)), area);

    if let Some((label, hue)) = s.badge {
        let right = vec![
            Span::styled(format!(" {label} "), Style::default().fg(CANVAS).bg(hue)),
            Span::raw(" "),
        ];
        f.render_widget(
            Paragraph::new(Line::from(right)).alignment(Alignment::Right),
            area,
        );
    }
}

// ============================================================================
// KEY LINES (pattern 4, rule N-9) - there is NO global footer hint row.
// Keyboard affordances render where they act: ONE MUTED key line INSIDE a
// dock/picker card (max ~5 items), recovery keys ADJACENT to a failure block
// (see `key_span`), or at most one inline hint in the composer placeholder.
// ============================================================================

/// ` key label · key label` - the in-card / adjacent key line. Keys MUTED,
/// labels FAINT, chords lowercase (`ctrl+p`, `esc`). Max ~5 pairs; never
/// rendered as a global row under the statusline.
pub fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for (i, (k, l)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                format!(" {SEP} "),
                Style::default().fg(FAINT),
            ));
        }
        spans.push(Span::styled(k.to_string(), Style::default().fg(MUTED)));
        spans.push(Span::styled(format!(" {l}"), Style::default().fg(FAINT)));
    }
    Line::from(spans)
}

// ============================================================================
// FRAMES - the three screen shapes. Each paints CANVAS and draws the
// statusline as the last row; there is NO hint row under it (rule N-9). The
// composer (or the dock that replaces it) is the screen's ONE bordered zone.
// ============================================================================

/// Chat frame: transcript column + composer + statusline. The transcript
/// body is 36 rows tall at the fixed 120x40 capture (40 minus composer 3,
/// statusline 1).
pub fn cli_page(f: &mut Frame, composer_ghost: &str, status: &StatusLine) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // transcript body
            Constraint::Length(3), // composer
            Constraint::Length(1), // statusline
        ])
        .split(area);
    composer(f, rows[1], composer_ghost);
    statusbar(f, rows[2], status);
    rows[0]
}

/// Chat frame with a typed multi-line composer. The lines are rendered inside
/// the same single rounded composer zone as `cli_page`; the first row has the
/// prompt marker and continuation rows hang under its text edge.
pub fn cli_page_multiline(
    f: &mut Frame,
    typed_lines: Vec<Line<'static>>,
    status: &StatusLine,
) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let composer_h = (typed_lines.len() as u16).saturating_add(2).max(3);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ])
        .split(area);
    composer_lines(f, rows[1], typed_lines);
    statusbar(f, rows[2], status);
    rows[0]
}

/// Full-screen view frame (help / status / sidebar / gallery): a one-row
/// header (` title ` + faint rule), full-height body, statusline. No
/// composer - you read these, you don't type. Navigation keys, where needed,
/// render inside the view (header line or attached to the interactive
/// element), never as a global row.
pub fn cli_view(f: &mut Frame, title: &str, status: &StatusLine) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header rule
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // body
            Constraint::Length(1), // statusline
        ])
        .split(area);
    rule_head(f, rows[0], title);
    statusbar(f, rows[3], status);
    rows[2]
}

/// Dock frame: the composer slot is REPLACED by an action dock `dock_h` rows
/// tall (rule F-5 / N-7: decisions and pickers anchor where the user would
/// type, never a floating centered modal). Returns `(transcript, dock)`;
/// the caller draws a `card()` into the dock and puts the card's key line
/// INSIDE it (rule N-9).
pub fn cli_action(f: &mut Frame, dock_h: u16, status: &StatusLine) -> (Rect, Rect) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),         // transcript body
            Constraint::Length(dock_h), // action dock
            Constraint::Length(1),      // statusline
        ])
        .split(area);
    statusbar(f, rows[2], status);
    (rows[0], rows[1])
}

/// Bottom input composer: ONE rounded FAINT-bordered box with an ACCENT `›`
/// prompt and a FAINT ghost placeholder.
fn composer(f: &mut Frame, area: Rect, ghost: &str) {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(FAINT));
    let inner = b.inner(area);
    f.render_widget(b, area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{SELECT_MARKER} "),
                Style::default().fg(ACCENT),
            ),
            Span::styled(ghost.to_string(), Style::default().fg(FAINT)),
        ])),
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(1), 1),
    );
}

/// Typed composer rows for `cli_page_multiline`, retaining the canonical
/// rounded border and one copper prompt marker on the first draft row.
fn composer_lines(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(FAINT));
    let inner = b.inner(area);
    f.render_widget(b, area);
    f.render_widget(
        Paragraph::new(lines),
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(1), inner.height),
    );
}

// ============================================================================
// TRANSCRIPT VOCABULARY - turns, headings, dividers, cards.
// ============================================================================

/// Brand wordmark row: `   Cortex Code  vX.Y.Z` - the copper accent's one
/// home in the transcript (welcome blocks, auth screens). Indented to the
/// transcript's column-4 text edge.
pub fn wordmark(version: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled("Cortex Code".to_string(), Style::default().fg(ACCENT)),
        Span::styled(format!("  {version}"), Style::default().fg(MUTED)),
    ])
}

// ============================================================================
// BANNER (pattern 13, rule F-6) - the droid-style ASCII-art "Code" wordmark.
// home and onboarding open with it; NO other screen carries a banner. On
// those two screens it counts as ONE accent element (budget: banner + <=2
// other accent elements).
// ============================================================================

/// The block-letter rows spelling "Code" - built only from `█` and spaces
/// (authorized set), 6 rows tall, 31 columns wide.
const BANNER_ROWS: [&str; 6] = [
    " ██████              ██",
    "██                   ██",
    "██        ████    █████   ████",
    "██       ██  ██  ██  ██  ██████",
    "██       ██  ██  ██  ██  ██",
    " ██████   ████    █████   █████",
];

/// The ASCII-art "Code" banner in ACCENT copper, indented to the col-4 text
/// edge. Returns the 6 transcript lines; put version + greeting nearby.
pub fn banner() -> Vec<Line<'static>> {
    BANNER_ROWS
        .iter()
        .map(|row| {
            Line::from(vec![
                Span::raw("   "),
                Span::styled((*row).to_string(), Style::default().fg(ACCENT)),
            ])
        })
        .collect()
}

/// A plain FAINT `─` rule as a transcript line, one-space left margin -
/// separates regions without a title (use `rule_head` for titled rules).
pub fn rule_line(width: usize) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled("\u{2500}".repeat(width), Style::default().fg(FAINT)),
    ])
}

/// A user turn: ` › cmd` with an optional trailing muted age (`2m ago`).
pub fn user_cmd(cmd: &str, meta: Option<&str>) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{SELECT_MARKER} "),
            Style::default().fg(ACCENT),
        ),
        Span::styled(cmd.to_string(), Style::default().fg(BRIGHT)),
    ];
    if let Some(meta) = meta {
        spans.push(Span::styled(
            format!("  {SEP} {meta}"),
            Style::default().fg(MUTED),
        ));
    }
    Line::from(spans)
}

/// An agent turn heading; agent prose below indents to the same 3-col pad.
pub fn agent_head(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled(title.to_string(), Style::default().fg(BRIGHT)),
    ])
}

/// Render transcript lines bottom-anchored in `body`: the newest turn hugs
/// the composer, spare space sits at the top - real scrollback behavior.
/// Lines beyond the body height are truncated from the top; keep counts under
/// the body height (36 rows in `cli_page`).
pub fn transcript(f: &mut Frame, body: Rect, lines: Vec<Line<'static>>) {
    let n = lines.len() as u16;
    let area = if n < body.height {
        Rect::new(body.x, body.y + (body.height - n), body.width, n)
    } else {
        body
    };
    f.render_widget(Paragraph::new(lines), area);
}

/// ` title ` + faint rule to the edge - the standard block divider (regions
/// separate by whitespace and rules, not boxes). Consumes one row, returns
/// the body Rect below it.
pub fn rule_head(f: &mut Frame, area: Rect, title: &str) -> Rect {
    if area.height == 0 {
        return area;
    }
    let label = format!(" {title} ");
    let used = label.chars().count().min(area.width as usize);
    let rule_w = (area.width as usize).saturating_sub(used);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(label, Style::default().fg(MUTED)),
            Span::styled("\u{2500}".repeat(rule_w), Style::default().fg(FAINT)),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1))
}

/// A rounded FAINT-bordered card with an embedded ` title `, no filled
/// surface - the dock's single bordered zone. `keys` is the card's ONE MUTED
/// key line (rule N-9: a card's shortcuts live INSIDE it, max ~5 pairs),
/// drawn on the card's last interior row; pass `&[]` for a keyless card.
/// Returns the content Rect above the key line.
pub fn card(f: &mut Frame, area: Rect, title: &str, keys: &[(&str, &str)]) -> Rect {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(FAINT))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(BRIGHT),
        ));
    let inner = b.inner(area);
    f.render_widget(b, area);
    if keys.is_empty() || inner.height == 0 {
        return inner;
    }
    let key_row = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    f.render_widget(Paragraph::new(hints(keys)), key_row);
    Rect::new(inner.x, inner.y, inner.width, inner.height - 1)
}

// ============================================================================
// THEME DATA - swatch hues for the theme gallery/picker. Data lives here so
// screens still never touch raw `Color::Rgb`.
// ============================================================================

/// A brand theme's picker labels and swatch hues. This is the shared source
/// of truth for the theme picker and the later gallery previews.
pub struct ThemePreview {
    pub name: &'static str,
    pub descriptor: &'static str,
    pub tag: &'static str,
    pub swatches: [Color; 5],
}

/// The five supported brand themes, in the order shown by `/themes`.
pub fn theme_previews() -> &'static [ThemePreview] {
    &[
        ThemePreview {
            name: "dark",
            descriptor: "cortex default",
            tag: "warm",
            swatches: [CANVAS, TXT, ACCENT, OK, BAD],
        },
        ThemePreview {
            name: "light",
            descriptor: "daytime",
            tag: "cool",
            swatches: [
                Color::Rgb(250, 247, 242),
                Color::Rgb(51, 56, 63),
                Color::Rgb(192, 107, 68),
                Color::Rgb(79, 121, 66),
                Color::Rgb(181, 74, 85),
            ],
        },
        ThemePreview {
            name: "vscode-dark",
            descriptor: "familiar",
            tag: "neutral",
            swatches: [
                Color::Rgb(30, 30, 30),
                Color::Rgb(212, 212, 212),
                Color::Rgb(79, 193, 255),
                Color::Rgb(137, 190, 116),
                Color::Rgb(239, 83, 80),
            ],
        },
        ThemePreview {
            name: "monokai",
            descriptor: "vivid",
            tag: "high-contrast",
            swatches: [
                Color::Rgb(39, 40, 34),
                Color::Rgb(248, 248, 242),
                Color::Rgb(253, 151, 31),
                Color::Rgb(166, 226, 46),
                Color::Rgb(249, 38, 114),
            ],
        },
        ThemePreview {
            name: "ocean",
            descriptor: "cool blues",
            tag: "calm",
            swatches: [
                Color::Rgb(15, 34, 52),
                Color::Rgb(214, 225, 236),
                Color::Rgb(74, 164, 209),
                Color::Rgb(108, 190, 143),
                Color::Rgb(216, 106, 120),
            ],
        },
    ]
}

/// Render the brand theme rows with the normal picker selection treatment and
/// an explicit glyph-plus-color current indicator for the live theme.
pub fn theme_select_list(
    width: usize,
    themes: &[ThemePreview],
    selected: usize,
) -> Vec<Line<'static>> {
    const CURRENT_W: usize = 9;
    let name_w = themes
        .iter()
        .map(|theme| theme.name.chars().count())
        .max()
        .unwrap_or(0);
    let tag_w = themes
        .iter()
        .map(|theme| theme.tag.chars().count())
        .max()
        .unwrap_or(0);

    themes
        .iter()
        .enumerate()
        .map(|(i, theme)| {
            let selected_row = i == selected;
            let sty = |hue: Color| {
                let s = Style::default().fg(hue);
                if selected_row {
                    s.bg(SURFACE)
                } else {
                    s
                }
            };
            let marker = if selected_row {
                Span::styled(format!(" {SELECT_MARKER} "), sty(ACCENT))
            } else {
                Span::styled("   ".to_string(), sty(TXT))
            };
            let name_hue = if selected_row { BRIGHT } else { TXT };
            let detail_hue = if selected_row { TXT } else { MUTED };
            let name_cell = format!("{:<name_w$}  ", theme.name);
            let current = if selected_row { "● current" } else { "" };
            let current_cell = format!("{current:<CURRENT_W$}  ");
            let tag_cell = format!("{:<tag_w$}", theme.tag);
            let used = 3
                + name_cell.chars().count()
                + theme.descriptor.chars().count()
                + current_cell.chars().count()
                + tag_cell.chars().count();
            let pad = width.saturating_sub(used + 1).max(1);
            Line::from(vec![
                marker,
                Span::styled(name_cell, sty(name_hue)),
                Span::styled(theme.descriptor.to_string(), sty(detail_hue)),
                Span::styled(" ".repeat(pad), sty(detail_hue)),
                Span::styled(current_cell, sty(INFO)),
                Span::styled(tag_cell, sty(MUTED)),
                Span::styled(" ".to_string(), sty(MUTED)),
            ])
        })
        .collect()
}

/// The light-theme token table (used by theme previews / light-mode mocks).
pub fn light_palette() -> Vec<(&'static str, Color)> {
    vec![
        ("CANVAS", Color::Rgb(250, 247, 242)),
        ("ACCENT", Color::Rgb(192, 107, 68)),
        ("OK", Color::Rgb(79, 121, 66)),
        ("BAD", Color::Rgb(181, 74, 85)),
        ("WARN", Color::Rgb(168, 122, 46)),
        ("INFO", Color::Rgb(61, 111, 168)),
        ("BRIGHT", Color::Rgb(20, 23, 28)),
        ("TXT", Color::Rgb(51, 56, 63)),
        ("MUTED", Color::Rgb(107, 114, 128)),
        ("FAINT", Color::Rgb(195, 200, 209)),
        ("SURFACE", Color::Rgb(232, 228, 220)),
        ("ADD_BG", Color::Rgb(228, 239, 218)),
        ("DEL_BG", Color::Rgb(243, 224, 221)),
    ]
}
