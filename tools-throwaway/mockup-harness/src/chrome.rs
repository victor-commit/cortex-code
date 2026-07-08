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
        Span::styled(format!("({args})"), Style::default().fg(MUTED)),
    ];
    if let Some(secs) = elapsed {
        spans.push(Span::styled(
            format!(" {SEP} {}", timefmt(secs)),
            Style::default().fg(MUTED),
        ));
    }
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
// STATUSLINE (pattern 3) - dim gray, NO filled background (rule N-1).
// ============================================================================

/// Statusline content. Left: `path · branch · model · ctx N% · cost`; right:
/// an optional fixed-slot mode badge, then the state dot ALWAYS paired with a
/// one-word label (`● live`, `● busy`) - glyph + color, never color alone.
pub struct StatusLine {
    pub path: &'static str,
    pub branch: &'static str,
    pub model: &'static str,
    pub ctx_pct: u8,
    pub cost: &'static str,
    /// Mode badge before the dot: `("plan", INFO)` is the single INFO badge;
    /// any elevated/active mode badge is ACCENT.
    pub badge: Option<(&'static str, Color)>,
    /// `(hue, label)`: MUTED "idle" at rest, OK "live" when healthy.
    pub state: (Color, &'static str),
}

impl Default for StatusLine {
    fn default() -> Self {
        Self {
            path: "~/projects/cortex-code",
            branch: "master",
            model: "claude-opus",
            ctx_pct: 42,
            cost: "$0.14",
            badge: None,
            state: (MUTED, "idle"),
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
        sep(),
        seg(&format!("ctx {}%", s.ctx_pct)),
        sep(),
        seg(s.cost),
    ];
    f.render_widget(Paragraph::new(Line::from(left)), area);

    let mut right: Vec<Span<'static>> = Vec::new();
    if let Some((label, hue)) = s.badge {
        right.push(Span::styled(
            format!(" {label} "),
            Style::default().fg(CANVAS).bg(hue),
        ));
        right.push(Span::raw("  "));
    }
    right.push(Span::styled(
        format!("{DOT} "),
        Style::default().fg(s.state.0),
    ));
    right.push(Span::styled(
        format!("{} ", s.state.1),
        Style::default().fg(MUTED),
    ));
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        area,
    );
}

// ============================================================================
// FOOTER HINTS (pattern 4) - 3-6 keys max, progressive disclosure (`? more`).
// ============================================================================

/// ` key label · key label · ? more` - keys MUTED, labels FAINT, chords
/// lowercase (`ctrl+p`, `esc`, `enter`).
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
// FRAMES - the three screen shapes. Each paints CANVAS, draws the statusline
// and the hints row, and returns the content Rect(s). The composer (or the
// dock that replaces it) is the screen's ONE bordered zone.
// ============================================================================

/// Chat frame: transcript column + composer + statusline + hints. The
/// transcript body is 35 rows tall at the fixed 120x40 capture
/// (40 minus composer 3, statusline 1, hints 1).
pub fn cli_page(
    f: &mut Frame,
    composer_ghost: &str,
    status: &StatusLine,
    keys: &[(&str, &str)],
) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // transcript body
            Constraint::Length(3), // composer
            Constraint::Length(1), // statusline
            Constraint::Length(1), // hints
        ])
        .split(area);
    composer(f, rows[1], composer_ghost);
    statusbar(f, rows[2], status);
    f.render_widget(Paragraph::new(hints(keys)), rows[3]);
    rows[0]
}

/// Full-screen view frame (help / status / sidebar / gallery): a one-row
/// header (` title ` + faint rule), full-height body, statusline + hints.
/// No composer - you read these, you don't type.
pub fn cli_view(
    f: &mut Frame,
    title: &str,
    status: &StatusLine,
    keys: &[(&str, &str)],
) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header rule
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // body
            Constraint::Length(1), // statusline
            Constraint::Length(1), // hints
        ])
        .split(area);
    rule_head(f, rows[0], title);
    statusbar(f, rows[3], status);
    f.render_widget(Paragraph::new(hints(keys)), rows[4]);
    rows[2]
}

/// Dock frame: the composer slot is REPLACED by an action dock `dock_h` rows
/// tall (rule F-5 / N-7: decisions and pickers anchor where the user would
/// type, never a floating centered modal). Returns `(transcript, dock)`;
/// the caller draws a `card()` into the dock.
pub fn cli_action(
    f: &mut Frame,
    dock_h: u16,
    status: &StatusLine,
    keys: &[(&str, &str)],
) -> (Rect, Rect) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),         // transcript body
            Constraint::Length(dock_h), // action dock
            Constraint::Length(1),      // statusline
            Constraint::Length(1),      // hints
        ])
        .split(area);
    statusbar(f, rows[2], status);
    f.render_widget(Paragraph::new(hints(keys)), rows[3]);
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
/// the body height (35 rows in `cli_page`).
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
/// surface - the dock's single bordered zone. Hints belong in the global
/// footer row, never inside the card (rule F-5). Returns the content Rect.
pub fn card(f: &mut Frame, area: Rect, title: &str) -> Rect {
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
    inner
}

// ============================================================================
// THEME DATA - swatch hues for the theme gallery/picker. Data lives here so
// screens still never touch raw `Color::Rgb`.
// ============================================================================

/// `(name, [canvas, text, accent, ok, bad])` swatches per selectable theme.
pub fn theme_previews() -> Vec<(&'static str, [Color; 5])> {
    vec![
        ("cortex dark", [CANVAS, TXT, ACCENT, OK, BAD]),
        (
            "cortex light",
            [
                Color::Rgb(250, 247, 242),
                Color::Rgb(51, 56, 63),
                Color::Rgb(192, 107, 68),
                Color::Rgb(79, 121, 66),
                Color::Rgb(181, 74, 85),
            ],
        ),
        (
            "high contrast",
            [
                Color::Rgb(0, 0, 0),
                Color::Rgb(255, 255, 255),
                Color::Rgb(255, 158, 108),
                Color::Rgb(150, 210, 128),
                Color::Rgb(255, 132, 142),
            ],
        ),
        (
            "solarized dark",
            [
                Color::Rgb(0, 43, 54),
                Color::Rgb(131, 148, 150),
                Color::Rgb(203, 75, 22),
                Color::Rgb(133, 153, 0),
                Color::Rgb(220, 50, 47),
            ],
        ),
    ]
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
