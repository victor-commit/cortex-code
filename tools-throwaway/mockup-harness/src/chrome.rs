//! Shared visual chrome for every mockup screen.
//!
//! The palette, glyphs and chrome idioms below are taken directly from the real
//! Cortex Code TUI crates so the mockups render in the SAME design language as
//! the product: `frame::style` for colors, `tui/src/glyphs.rs` for the safe
//! glyph set, `tui/src/brand.rs` for the `Cortex Code` wordmark lockup. Only
//! the on-screen data is stub; the look is the shipped design base.

// This module mirrors the FULL design-base token + glyph inventory (as the real
// `tui/src/glyphs.rs` does), so not every constant is used by every screen.
#![allow(dead_code)]

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

// --- Palette: an exact mirror of `frame::style` (mono-orange on near-black).
// ORANGE is the only UI hue; green/red/blue are reserved for status dots.

/// Page background — near-black `#0d0d0d` (`frame::style::CANVAS`).
pub const CANVAS: Color = Color::Rgb(13, 13, 13);
/// Primary brand accent: prompts, brand glyphs, selection (`ORANGE`).
pub const ORANGE: Color = Color::Rgb(232, 146, 86);
/// Dim orange: borders and secondary marks (`ORANGE_D`).
pub const ORANGE_D: Color = Color::Rgb(150, 92, 54);
/// Lighter accent: titles, highlights, active marks (`AMBER`).
pub const AMBER: Color = Color::Rgb(242, 183, 120);
/// Body text (`TXT`).
pub const TXT: Color = Color::Rgb(208, 208, 206);
/// Emphasized / selected text (`BRIGHT`).
pub const BRIGHT: Color = Color::Rgb(242, 242, 240);
/// Secondary text (`DIM`).
pub const DIM: Color = Color::Rgb(140, 140, 140);
/// Tertiary text / placeholders / hints (`FAINT`).
pub const FAINT: Color = Color::Rgb(92, 92, 98);
/// Rules, dividers, empty progress track (`GHOST`).
pub const GHOST: Color = Color::Rgb(60, 60, 66);
/// Panel surface (`PANEL`).
pub const PANEL: Color = Color::Rgb(22, 22, 26);
/// Alternate panel surface (`PANEL2`).
pub const PANEL2: Color = Color::Rgb(28, 28, 33);
/// Status-bar / footer background (`BAR`).
pub const BAR: Color = Color::Rgb(20, 20, 24);
/// Warm selected-row background (`SELBG`).
pub const SELBG: Color = Color::Rgb(44, 35, 26);
/// Brand card / panel border (`BORDER_BRAND`).
pub const BORDER_BRAND: Color = Color::Rgb(92, 66, 46);
/// Success green — status dots only (`OKG`).
pub const OKG: Color = Color::Rgb(96, 204, 110);
/// Error red — status dots only (`ERR`).
pub const ERR: Color = Color::Rgb(224, 96, 96);
/// Info blue — status dots only (`INFO_DOT`).
pub const INFO_DOT: Color = Color::Rgb(96, 170, 224);
/// Added-line highlight tint — dark green wash behind `+` diff rows (`ADD_BG`).
pub const ADD_BG: Color = Color::Rgb(18, 40, 26);
/// Removed-line highlight tint — dark red wash behind `-` diff rows (`DEL_BG`).
pub const DEL_BG: Color = Color::Rgb(48, 22, 22);
pub const SOFT: Color = Color::Rgb(34, 28, 22);

pub const PETALS: [&str; 4] = ["\u{00b7}", "\u{2722}", "\u{273b}", "\u{273d}"];

// Semantic aliases used throughout the screens (map onto the real tokens above
// so existing screen code reads naturally without diverging from the base).
pub const ACCENT: Color = ORANGE;
pub const TITLE: Color = AMBER;
pub const OK: Color = OKG;
pub const WARN: Color = AMBER;
pub const BAD: Color = ERR;
pub const INFO: Color = INFO_DOT;
pub const MUTED: Color = DIM;
pub const LOCKED: Color = ORANGE_D;

// --- Canonical glyphs (subset of `tui/src/glyphs.rs`, all DejaVu-safe). ---

/// Selection / prompt marker (never the disclosure `▶`).
pub const SELECT_MARKER: &str = "›";
/// Faint inline separator.
pub const SEP: &str = "·";
/// Full progress cell.
pub const PROGRESS_FULL: &str = "█";
/// Empty progress cell (drawn in the GHOST tone).
pub const PROGRESS_EMPTY: &str = "░";

/// A filled ratio bar of `filled`/`total` cells using block glyphs.
pub fn bar(filled: usize, total: usize) -> String {
    let filled = filled.min(total);
    let mut s = String::new();
    for _ in 0..filled {
        s.push_str(PROGRESS_FULL);
    }
    for _ in filled..total {
        s.push_str(PROGRESS_EMPTY);
    }
    s
}

/// A ratio bar as two spans: filled cells in ORANGE, empty track in GHOST —
/// matching the `pbar` idiom from the real full-screen views.
pub fn bar_spans(filled: usize, total: usize) -> Vec<Span<'static>> {
    let filled = filled.min(total);
    vec![
        Span::styled(PROGRESS_FULL.repeat(filled), Style::default().fg(ORANGE)),
        Span::styled(
            PROGRESS_EMPTY.repeat(total - filled),
            Style::default().fg(GHOST),
        ),
    ]
}

pub fn fmt_elapsed(secs: u32) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let ss = secs % 60;
        format!("{m}m {ss:02}s")
    } else {
        let h = secs / 3600;
        let mm = (secs % 3600) / 60;
        let ss = secs % 60;
        format!("{h}h {mm:02}m {ss:02}s")
    }
}

/// Convenience: a plain (body-text) span.
pub fn t(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(TXT))
}

/// Convenience: a muted span.
pub fn m(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(DIM))
}

/// Convenience: a colored span.
pub fn c(s: &str, color: Color) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(color))
}

/// Convenience: a bold colored span.
pub fn cb(s: &str, color: Color) -> Span<'static> {
    Span::styled(
        s.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

// ============================================================================
// FLAT PAGE MANNER — mirrors crates/tui-components/src/{page_layout,card,
// key_hints,borders}.rs. Shipped command pages are FLAT (top rule, navbar with
// an inverted filled active tab, `label: value` rows on the bare canvas, a
// key-hints footer), NOT one outer box full of nested filled panels.
// ============================================================================

/// Product surfaces, stable order; each screen highlights the one it belongs to.
pub const NAV: [&str; 8] = [
    "Mode", "Workflow", "Team", "Review", "Intent", "Metrics", "Verify", "Tools",
];

/// Draw the flat page frame and return the body `Rect`.
pub fn page(f: &mut Frame, active_nav: &str, title: &str, hints: &[(&str, &str)]) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top rule
            Constraint::Length(1), // brand + title
            Constraint::Length(1), // nav tabs
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // body
            Constraint::Length(1), // session status line
            Constraint::Length(1), // footer rule
            Constraint::Length(1), // key hints
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(ORANGE_D),
        ))),
        rows[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Cortex Code",
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {SEP}  "), Style::default().fg(FAINT)),
            Span::styled(
                title.to_string(),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("mockup ", Style::default().fg(FAINT)),
            Span::styled(SEP, Style::default().fg(FAINT)),
            Span::styled(" stub data ", Style::default().fg(FAINT)),
        ]))
        .alignment(Alignment::Right),
        rows[1],
    );

    let mut nav: Vec<Span> = vec![Span::raw(" ")];
    for (i, item) in NAV.iter().enumerate() {
        if i > 0 {
            nav.push(Span::styled("   ", Style::default().fg(DIM)));
        }
        if *item == active_nav {
            nav.push(Span::styled(
                format!(" {item} "),
                Style::default()
                    .fg(CANVAS)
                    .bg(ORANGE)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            nav.push(Span::styled(item.to_string(), Style::default().fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(nav)), rows[2]);

    statusbar(f, rows[5], active_nav);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(GHOST),
        ))),
        rows[6],
    );
    f.render_widget(keyhints(hints, area.width), rows[7]);

    rows[4]
}

/// Persistent session status line: activity + model/ctx/cost, on the canvas.
fn statusbar(f: &mut Frame, area: Rect, active_nav: &str) {
    let sep = || Span::styled(format!("  {SEP}  "), Style::default().fg(FAINT));
    let seg = |label: &str, val: &str, color: Color| {
        vec![
            Span::styled(format!("{label} "), Style::default().fg(DIM)),
            Span::styled(val.to_string(), Style::default().fg(color)),
        ]
    };
    let mut left: Vec<Span> = vec![
        Span::styled(" \u{25cf} ", Style::default().fg(OKG)),
        Span::styled("live", Style::default().fg(OKG)),
        sep(),
        Span::styled(active_nav.to_string(), Style::default().fg(AMBER)),
    ];
    left.push(sep());
    left.extend(seg("model", "claude-opus", BRIGHT));
    left.push(sep());
    left.extend(seg("ctx", "47%", TXT));
    left.push(sep());
    left.extend(seg("cost", "$0.0184", TXT));
    f.render_widget(Paragraph::new(Line::from(left)), area);
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "~/projects/cortex-code ",
            Style::default().fg(FAINT),
        )]))
        .alignment(Alignment::Right),
        area,
    );
}

/// Split a Rect into two columns (left percentage) with a 2-col gutter.
pub fn two_col(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);
    (cols[0], cols[2])
}

/// Key-hints bar: `key desc` pairs, ` · ` separated, on the status-bar surface.
fn keyhints(hints: &[(&str, &str)], _width: u16) -> Paragraph<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    for (i, (k, d)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(
                format!(" {SEP} "),
                Style::default().fg(FAINT),
            ));
        }
        spans.push(Span::styled(
            k.to_string(),
            Style::default().fg(ORANGE),
        ));
        spans.push(Span::styled(
            format!(" {d}"),
            Style::default().fg(DIM),
        ));
    }
    Paragraph::new(Line::from(spans))
}

// ============================================================================
// REAL-CLI MANNER — mirrors how shipped coding-agent CLIs (Codex, Claude Code,
// Factory droid) actually lay out: a scrollback TRANSCRIPT body, a bottom INPUT
// COMPOSER (`›` prompt + ghost placeholder), a dim STATUS LINE (cwd · branch ·
// mode · model · ctx · tokens · cost), and a key-hints row. NO top nav tab bar.
// ============================================================================

/// Shared CLI header: top rule, `Cortex Code · <title>` on the left, and the
/// `session · mode` meta on the right — common to both CLI frame variants.
///
/// No full-width top rule: the harness reads as inline CLI output that keeps the
/// SSH scrollback (like Claude Code / Factory), not a full-screen TUI with pinned
/// top chrome — so the brand line is the first thing printed.
fn cli_header(f: &mut Frame, brand_row: Rect, title: &str) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " Cortex Code",
                Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {SEP}  "), Style::default().fg(FAINT)),
            Span::styled(
                title.to_string(),
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ),
        ])),
        brand_row,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("session ", Style::default().fg(FAINT)),
            Span::styled("wf-42", Style::default().fg(DIM)),
            Span::styled(format!("  {SEP}  "), Style::default().fg(FAINT)),
            Span::styled("agent mode", Style::default().fg(ORANGE)),
            Span::raw(" "),
        ]))
        .alignment(Alignment::Right),
        brand_row,
    );
}

/// Draw the real-CLI frame WITH an input composer and return the transcript
/// body `Rect`. For conversational / action screens where the user types next.
///
/// Rows: transcript(min) · composer(3) · status line · key hints. At the fixed
/// 120x40 capture the transcript is 35 rows tall, so screens fill the canvas
/// instead of leaving dead space.
pub fn cli_page(f: &mut Frame, _title: &str, composer_ghost: &str, hints: &[(&str, &str)]) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // transcript body
            Constraint::Length(3), // input composer
            Constraint::Length(1), // footer: hints + compact status
        ])
        .split(area);

    composer(f, rows[1], composer_ghost);
    footer(f, rows[2], hints);

    rows[0]
}

/// Draw the real-CLI frame WITHOUT a composer and return the full-height body
/// `Rect`. For monitoring / overlay views (mission-control, metrics, ladders)
/// that a keybinding opens over the session — you read them, you don't type.
///
/// Rows: body(min) · status line · key hints.
pub fn cli_view(f: &mut Frame, _title: &str, hints: &[(&str, &str)]) -> Rect {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // body
            Constraint::Length(1), // footer: hints + compact status
        ])
        .split(area);

    footer(f, rows[1], hints);

    rows[0]
}

/// Build a horizontal table border from per-column content widths, choosing the
/// left / junction / right glyphs (`┌ ┬ ┐`, `├ ┼ ┤`, or `└ ┴ ┘`). Each column
/// reserves `width + 2` for its one-space cell padding, so junctions land exactly
/// under the `│` separators of the cell rows built with `vsep`.
pub fn table_rule(widths: &[usize], left: &str, mid: &str, right: &str) -> Line<'static> {
    let mut s = String::from(left);
    for (i, w) in widths.iter().enumerate() {
        s.push_str(&"\u{2500}".repeat(w + 2));
        s.push_str(if i + 1 == widths.len() { right } else { mid });
    }
    Line::from(Span::styled(s, Style::default().fg(FAINT)))
}

/// A single vertical table separator `│`, faint so cell content stays dominant.
pub fn vsep() -> Span<'static> {
    Span::styled("\u{2502}".to_string(), Style::default().fg(FAINT))
}

/// Draw the real-CLI frame with the bottom input composer REPLACED by an action
/// dock `dock_h` rows tall, and return `(transcript_body, dock)` Rects.
///
/// This is the affordance for permission prompts and pickers (model / agent /
/// skills / session, the slash + command menus): the decision or selection lives
/// in place of the input line the user would otherwise type into, NOT as a
/// centered floating modal over a dimmed backdrop (see cortex-code-design N-7).
/// The caller draws a `card()` (rounded border, no filled surface) into `dock`.
///
/// Rows: transcript(min) · dock(dock_h) · status line · key hints.
pub fn cli_action(
    f: &mut Frame,
    _title: &str,
    dock_h: u16,
    hints: &[(&str, &str)],
) -> (Rect, Rect) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(CANVAS)), area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),         // transcript body
            Constraint::Length(dock_h), // action dock (replaces the composer)
            Constraint::Length(1),      // footer: hints + compact status
        ])
        .split(area);

    footer(f, rows[2], hints);

    (rows[0], rows[1])
}

/// Bottom input composer: a rounded box with a `›` prompt and ghost placeholder,
/// exactly the affordance a user types the next instruction into.
fn composer(f: &mut Frame, area: Rect, ghost: &str) {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ORANGE_D));
    let inner = b.inner(area);
    f.render_widget(b, area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{SELECT_MARKER} "),
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(ghost.to_string(), Style::default().fg(FAINT)),
        ])),
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(1), 1),
    );
}

/// Single-line footer: key hints left, compact model/ctx/cost/live status right.
fn footer(f: &mut Frame, area: Rect, hints: &[(&str, &str)]) {
    f.render_widget(keyhints(hints, area.width), area);
    let sep = || Span::styled(format!(" {SEP} "), Style::default().fg(FAINT));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("claude-opus", Style::default().fg(BRIGHT)),
            sep(),
            Span::styled("47%", Style::default().fg(TXT)),
            sep(),
            Span::styled("$0.0184", Style::default().fg(TXT)),
            sep(),
            Span::styled("\u{25cf} ", Style::default().fg(OKG)),
            Span::styled("live ", Style::default().fg(OKG)),
        ]))
        .alignment(Alignment::Right),
        area,
    );
}

/// Transcript: a user command turn — `› <cmd>` with a right-aligned timestamp.
pub fn user_cmd(cmd: &str, time: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {SELECT_MARKER} "),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            cmd.to_string(),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("    {SEP} {time}"), Style::default().fg(FAINT)),
    ])
}

/// Transcript: an agent turn header — the reply's section heading. Body lines
/// below indent to 3 columns, so the heading leads with the same 3-space pad.
pub fn agent_head(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled(
            title.to_string(),
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// Render transcript lines bottom-anchored in `body`: the newest turn hugs the
/// composer and any spare space sits at the top, exactly as real CLI scrollback
/// behaves (older turns scroll off the top, never a gap above the input).
pub fn transcript(f: &mut Frame, body: Rect, lines: Vec<Line<'static>>) {
    let n = lines.len() as u16;
    let area = if n < body.height {
        Rect::new(body.x, body.y + (body.height - n), body.width, n)
    } else {
        body
    };
    f.render_widget(Paragraph::new(lines), area);
}

/// Flat section: a bold title row, body directly below on the canvas (no box).
pub fn section(f: &mut Frame, area: Rect, title: &str, accent: Color) -> Rect {
    if area.height == 0 {
        return area;
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                title.to_string(),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1))
}

/// Section header that announces itself: ` title ` in the accent (bold) followed
/// by a faint horizontal rule to the panel edge, the standard CLI block divider.
/// Consumes one row and returns the body `Rect` below it.
pub fn rule_head(f: &mut Frame, area: Rect, title: &str, accent: Color) -> Rect {
    if area.height == 0 {
        return area;
    }
    let label = format!(" {title} ");
    let used = label.chars().count().min(area.width as usize);
    let rule_w = (area.width as usize).saturating_sub(used);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(label, Style::default().fg(accent).add_modifier(Modifier::BOLD)),
            Span::styled("\u{2500}".repeat(rule_w), Style::default().fg(GHOST)),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1))
}

/// Selective boxed block: rounded border, embedded ` title `, hints inside the
/// bottom border row, no filled surface. Returns the content `Rect`.
pub fn card(f: &mut Frame, area: Rect, title: &str, accent: Color, hints: &[(&str, &str)]) -> Rect {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ORANGE_D))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
    let mut inner = b.inner(area);
    f.render_widget(b, area);
    if !hints.is_empty() && inner.height > 1 {
        let hints_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
        f.render_widget(keyhints(hints, inner.width), hints_area);
        inner.height -= 1;
    }
    inner
}

/// Info row: `  label: value` with a dim label; empty label = continuation.
pub fn info(label: &str, value: &str, value_color: Color) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    if !label.is_empty() {
        spans.push(Span::styled(
            format!("{label}: "),
            Style::default().fg(DIM),
        ));
    }
    spans.push(Span::styled(value.to_string(), Style::default().fg(value_color)));
    Line::from(spans)
}

/// Inverted filled emphasis badge ` text ` (canvas-on-ORANGE). Emphasis only.
pub fn pill(text: &str) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(CANVAS)
            .bg(ORANGE)
            .add_modifier(Modifier::BOLD),
    )
}

/// Status indicator: colored dot + colored text (status hues only, never chrome).
pub fn status(state_color: Color, text: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("\u{25cf} ", Style::default().fg(state_color)),
        Span::styled(text.to_string(), Style::default().fg(state_color)),
    ]
}

/// Status dot glyph in the given hue.
pub fn dot(state_color: Color) -> Span<'static> {
    Span::styled("\u{25cf}", Style::default().fg(state_color))
}

pub fn state_glyph(color: Color) -> &'static str {
    match color {
        OKG => "\u{2713}",
        ERR => "\u{2717}",
        INFO_DOT => "\u{25cb}",
        _ => "\u{25cf}",
    }
}

/// Inline status chip `● text` in a status hue (never a filled box).
pub fn chip(text: &str, color: Color) -> Span<'static> {
    Span::styled(format!("\u{25cf} {text}"), Style::default().fg(color))
}

/// A collapsed function-call turn: `● <reason>   › <summary>`. The call is
/// stated as a natural-language reason (what the model asked for and why), the
/// dot carries the state hue, and the `›` chevron invites `ctrl+o` to expand.
pub fn call_collapsed(state: Color, reason: &str, summary: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        dot(state),
        Span::styled(format!(" {reason}"), Style::default().fg(TXT)),
        Span::styled(format!("   {SELECT_MARKER} "), Style::default().fg(ORANGE)),
        Span::styled(summary.to_string(), Style::default().fg(DIM)),
    ])
}

/// An expanded function-call head: `● <reason>` in bright, no trailing chevron.
/// The caller prints `call_detail` rows underneath to reveal the result.
pub fn call_expanded(state: Color, reason: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        dot(state),
        Span::styled(
            format!(" {reason}"),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// A detail row hanging under an expanded call: `     → <spans>` at 5-col indent.
pub fn call_detail(spans: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![
        Span::raw("     "),
        Span::styled("\u{2192} ".to_string(), Style::default().fg(DIM)),
    ];
    out.extend(spans);
    Line::from(out)
}

pub fn call_head(state: Color, title: &str, rest: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![
        Span::raw(" "),
        dot(state),
        Span::styled(
            format!(" {title}"),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ];
    if !rest.is_empty() {
        out.push(Span::raw(" "));
        out.extend(rest);
    }
    Line::from(out)
}

pub fn call_active(header: &str, elapsed: u32) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        dot(AMBER),
        Span::styled(
            format!(" {header}"),
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ({} \u{00b7} Esc to interrupt)", fmt_elapsed(elapsed)),
            Style::default().fg(FAINT),
        ),
    ])
}

pub fn call_branch(spans: Vec<Span<'static>>, first: bool) -> Line<'static> {
    let mut out = if first {
        vec![
            Span::raw("   "),
            Span::styled("\u{2514} ".to_string(), Style::default().fg(DIM)),
        ]
    } else {
        vec![Span::raw("     ")]
    };
    out.extend(spans);
    Line::from(out)
}

pub fn call_result(ok: bool, code: Option<i32>, detail: &str) -> Line<'static> {
    let (glyph, color) = if ok {
        ("\u{2713}", OKG)
    } else {
        ("\u{2717}", ERR)
    };
    let mut out = vec![
        Span::raw(" "),
        Span::styled(
            glyph.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(c) = code {
        out.push(Span::styled(format!(" ({c})"), Style::default().fg(color)));
    }
    out.push(Span::styled(
        format!(" \u{00b7} {detail}"),
        Style::default().fg(FAINT),
    ));
    Line::from(out)
}
