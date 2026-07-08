//! Screen 4 — the task board, framed as a mission-control VIEW a keybinding
//! opens over the session (no composer — you read the board, you don't type
//! into it). A one-line state-flow ribbon, a dense scrollback-style task table,
//! then the focused contract and the coordinator's assignment rules side by side.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, cb, m, t, ACCENT, BAD, DIM, INFO, MUTED, OK, TITLE, TXT, WARN,
};

// Fixed column widths keep the table a real grid, not ragged prose.
// Row width = 1 + Σ(w+2) + n = 1 + 104 + 15 = 120, so the box spans the frame.
const COLS: [usize; 5] = [6, 11, 38, 9, 40];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Team Board",
        &[
            ("enter", "open item"),
            ("c", "claim READY"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // state-flow ribbon
            Constraint::Length(1),  // spacer
            Constraint::Length(1),  // board rule
            Constraint::Length(13), // task table (box)
            Constraint::Length(1),  // spacer
            Constraint::Min(0),     // focused item + coordinator + activity
        ])
        .split(body);

    f.render_widget(Paragraph::new(flow_ribbon()), rows[0]);
    chrome::rule_head(f, rows[2], "board \u{00b7} tasks", TITLE);
    f.render_widget(Paragraph::new(task_table()), rows[3]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // focused item + coordinator
            Constraint::Min(0),     // recent activity feed (box)
        ])
        .split(rows[5]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(lower[0]);
    render_focused(f, bottom[0]);
    render_coordinator(f, bottom[1]);
    render_activity(f, lower[1]);
}

/// The six lifecycle states as a single flow line, each in its status hue, with
/// live counts and the blocked tally pinned on the right.
fn flow_ribbon() -> Line<'static> {
    let seg = |name: &str, n: &str, color: Color| -> Vec<Span<'static>> {
        vec![
            chrome::dot(color),
            Span::styled(format!(" {name} "), Style::default().fg(DIM)),
            Span::styled(n.to_string(), Style::default().fg(TXT)),
        ]
    };
    let arrow = || m("  \u{2192}  ");
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" flow   ", Style::default().fg(DIM))];
    spans.extend(seg("OPEN", "1", MUTED));
    spans.push(arrow());
    spans.extend(seg("READY", "2", INFO));
    spans.push(arrow());
    spans.extend(seg("CLAIMED", "1", ACCENT));
    spans.push(arrow());
    spans.extend(seg("ACTIVE", "1", WARN));
    spans.push(arrow());
    spans.extend(seg("SUBMITTED", "1", TITLE));
    spans.push(arrow());
    spans.extend(seg("CLOSED", "2", OK));
    spans.push(m("      "));
    spans.extend(seg("BLOCKED", "1", BAD));
    Line::from(spans)
}

fn task_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        row("T-102", "ACTIVE", WARN, "add rate-limit middleware to /login", Some("ws-77b1e0"), "in progress \u{00b7} test pending", TITLE),
        row("T-103", "CLAIMED", ACCENT, "extract token-refresh into a service", Some("ws-3f9a2c"), "T-100", DIM),
        row("T-101", "SUBMITTED", TITLE, "wire /session revocation endpoint", Some("ws-2c4d10"), "awaiting accept", DIM),
        row("T-104", "READY", INFO, "add audit-log sink for auth events", None, "T-102", DIM),
        row("T-105", "READY", INFO, "rotate signing keys on deploy", None, "T-100", DIM),
        row("T-107", "OPEN", MUTED, "backfill legacy password hashes", None, "\u{2014}", DIM),
        row("T-106", "BLOCKED", BAD, "migrate sessions table to v3", None, "failed dep \u{00b7} missing contract field", BAD),
        row("T-100", "CLOSED", OK, "create sessions table + schema", Some("ws-3f9a2c"), "\u{2713} accepted", OK),
        row("T-099", "CLOSED", OK, "scaffold auth crate + wiring", Some("ws-2c4d10"), "\u{2713} accepted", OK),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}

fn cell(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(chrome::vsep());
    out
}

fn header_row() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["ID", "STATE", "TASK", "OWNER", "DEPENDS / ARTIFACT"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

#[allow(clippy::too_many_arguments)]
fn row(
    id: &str,
    state: &str,
    state_color: Color,
    task: &str,
    owner: Option<&str>,
    extra: &str,
    extra_color: Color,
) -> Line<'static> {
    let owner_span = match owner {
        Some(o) => c(&format!("{o:<w$}", w = COLS[3]), INFO),
        None => m(&format!("{:<w$}", "\u{2014}", w = COLS[3])),
    };
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{id:<w$}", w = COLS[0]), TXT)]));
    let glyph = if state_color == BAD { chrome::state_glyph(state_color) } else { "\u{25cf}" };
    spans.extend(cell(vec![c(&format!("{:<w$}", format!("{glyph} {state}"), w = COLS[1]), state_color)]));
    spans.extend(cell(vec![t(&format!("{task:<w$}", w = COLS[2]))]));
    spans.extend(cell(vec![owner_span]));
    spans.extend(cell(vec![c(&format!("{extra:<w$}", w = COLS[4]), extra_color)]));
    Line::from(spans)
}

fn render_focused(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "focused \u{00b7} item T-102 \u{00b7} contract", TITLE);
    let field = |k: &str, v: &str| -> Line<'static> {
        Line::from(vec![c(&format!("  {k:<18}"), MUTED), t(v)])
    };
    let lines = vec![
        field("id", "T-102"),
        Line::from(vec![c(&format!("  {:<18}", "state"), MUTED), cb("\u{25cf} ACTIVE", WARN)]),
        field("task", "add rate-limit middleware to /login"),
        field("expected", "429 on 6th attempt; test green"),
        field("tools", "read_span, edit_span, run"),
        field("forbidden", "crates/db/**, prod config"),
        field("context", "crates/server/src/auth/*"),
        field("depends on", "T-100 (CLOSED)"),
        Line::from(vec![c(&format!("  {:<18}", "owner"), MUTED), c("ws-77b1e0", INFO)]),
        field("result", "status + artifact ref + error"),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_coordinator(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "coordinator \u{00b7} assignments", TITLE);
    let lines = vec![
        Line::from(m("  posts contracts \u{00b7} workers claim \u{00b7} core moves")),
        Line::from(""),
        Line::from(vec![
            cb("  post", ACCENT),
            m(" \u{2192} "),
            cb("claim", ACCENT),
            m(" \u{2192} "),
            cb("submit", ACCENT),
            m(" \u{2192} "),
            cb("accept", ACCENT),
        ]),
        Line::from(""),
        invariant("single ownership", "one per CLAIMED/ACTIVE"),
        invariant("no self-closing", "never closes own item"),
        invariant("reconcile", "vs expected \u{2192} re-post"),
        Line::from(""),
        Line::from(vec![
            m("  assignments   "),
            c("ws-3f9a2c", INFO),
            m("\u{2192}T-103   "),
            c("ws-77b1e0", INFO),
            m("\u{2192}T-102"),
        ]),
        Line::from(vec![
            m("                "),
            c("ws-2c4d10", INFO),
            m("\u{2192}T-101 (reconciling)"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

/// A coordinator invariant row: `● name  description`, dot in the accent hue.
fn invariant(name: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        t("  "),
        chrome::dot(ACCENT),
        c(&format!(" {name:<18}"), ACCENT),
        m(desc),
    ])
}

fn render_activity(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "recent \u{00b7} board activity", TITLE);
    f.render_widget(Paragraph::new(activity_table()), inner);
}

// Row width = 1 + Σw + 3n = 1 + 107 + 12 = 120, so the feed box spans the frame.
const COLS_ACT: [usize; 4] = [8, 13, 34, 52];

fn activity_header() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["TIME", "WHO", "EVENT", "NOTE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS_ACT) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn activity_row(time: &str, who: &str, who_color: Color, verb: &str, note: &str) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{time:<w$}", w = COLS_ACT[0]), DIM)]));
    spans.extend(cell(vec![c(
        &format!("{:<w$}", format!("\u{25cf} {who}"), w = COLS_ACT[1]),
        who_color,
    )]));
    spans.extend(cell(vec![t(&format!("{verb:<w$}", w = COLS_ACT[2]))]));
    spans.extend(cell(vec![m(&format!("{note:<w$}", w = COLS_ACT[3]))]));
    Line::from(spans)
}

fn activity_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS_ACT, "\u{250c}", "\u{252c}", "\u{2510}"),
        activity_header(),
        chrome::table_rule(&COLS_ACT, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        activity_row("14:02:11", "ws-77b1e0", INFO, "claimed T-102 \u{2192} ACTIVE", "rate-limit middleware"),
        activity_row("14:01:47", "core", ACCENT, "posted T-104, T-105 \u{2192} READY", "wave 2 unblocked by T-102"),
        activity_row("14:00:32", "ws-2c4d10", INFO, "submitted T-101 \u{2192} review", "awaiting coordinator accept"),
    ]);
    lines.push(chrome::table_rule(&COLS_ACT, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
