//! Screen 9 — the tool response envelope, framed as a mission-control VIEW a
//! keybinding opens over the session (no composer — you read the contract, you
//! don't type into it). A status-distribution ribbon, the fixed-order field
//! table, then the ok and refused examples side by side, over a live call feed.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, m, t, ACCENT, BAD, DIM, MUTED, OK, TITLE, TXT, WARN,
};

// Fixed column widths keep the contract a real grid, not ragged prose.
// Row width = 1 + Σ(w+2) + n = 1 + 115 + 4 = 120, so the box spans the frame.
const COLS: [usize; 4] = [3, 12, 26, 66];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Tool Envelope",
        &[
            ("\u{2190}\u{2192}", "example"),
            ("e", "expand payload"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // status ribbon
            Constraint::Length(1), // spacer
            Constraint::Length(1), // contract rule
            Constraint::Length(7), // field table (box)
            Constraint::Length(1), // invariant caption
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // examples + call feed
        ])
        .split(body);

    f.render_widget(Paragraph::new(ribbon()), rows[0]);
    chrome::rule_head(f, rows[2], "envelope \u{00b7} contract", TITLE);
    f.render_widget(Paragraph::new(field_table()), rows[3]);
    f.render_widget(Paragraph::new(invariant_note()), rows[4]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // ok + refused examples (exact fit, no slack)
            Constraint::Min(0),     // recent call feed (box)
        ])
        .split(rows[6]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(lower[0]);
    render_ok(f, split[0]);
    render_refused(f, split[1]);
    render_feed(f, lower[1]);
}

/// The three outcomes as one flow line — a status dot + live count each, with
/// the contract shape pinned on the right.
fn ribbon() -> Line<'static> {
    let seg = |name: &str, n: &str, color: Color| -> Vec<Span<'static>> {
        vec![
            chrome::dot(color),
            Span::styled(format!(" {name} "), Style::default().fg(DIM)),
            Span::styled(n.to_string(), Style::default().fg(TXT)),
        ]
    };
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" envelope \u{00b7} session  ", Style::default().fg(DIM))];
    spans.extend(seg("ok", "41", OK));
    spans.push(m("    "));
    spans.extend(seg("error", "2", BAD));
    spans.push(m("    "));
    spans.extend(seg("refused", "3", WARN));
    spans.push(m("        "));
    spans.push(Span::styled("3 fields ", Style::default().fg(DIM)));
    spans.push(Span::styled("\u{00b7} order fixed \u{00b7} all 11 tools", Style::default().fg(DIM)));
    Line::from(spans)
}

fn field_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        row("1", "status", "ok | error | refused", "outcome discriminant, read first"),
        row("2", "payload", "object", "result, or reason on error / refused"),
        row("3", "truncated", "bool", "true when output was clipped"),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}

fn invariant_note() -> Line<'static> {
    Line::from(m(
        "  every reply carries all 3 \u{00b7} shape is fixed, values vary by status",
    ))
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
    let labels = ["#", "FIELD", "TYPE", "NOTE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn row(num: &str, name: &str, ty: &str, note: &str) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![Span::styled(
        format!("{num:<w$}", w = COLS[0]),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )]));
    spans.extend(cell(vec![c(&format!("{name:<w$}", w = COLS[1]), TITLE)]));
    spans.extend(cell(vec![t(&format!("{ty:<w$}", w = COLS[2]))]));
    spans.extend(cell(vec![m(&format!("{note:<w$}", w = COLS[3]))]));
    Line::from(spans)
}

fn render_ok(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "example \u{00b7} status = ok", TITLE);
    let lines = vec![
        Line::from(vec![t("  "), c("\u{25cf} ok", OK), m("   tool ran and returned")]),
        Line::from(m("  {")),
        kv("status", "\"ok\"", OK),
        Line::from(vec![c("    \"payload\"", TITLE), m(": {")]),
        Line::from(vec![c("      \"matches\"", TITLE), m(": "), t("3"), m(",")]),
        Line::from(vec![c("      \"file\"", TITLE), m(": "), t("\"routes.rs\"")]),
        Line::from(m("    },")),
        Line::from(vec![c("    \"truncated\"", TITLE), m(": "), c("false", MUTED)]),
        Line::from(m("  }")),
        Line::from(vec![m("  reader checks "), c("status", TITLE), m(", then "), c("truncated", TITLE)]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_refused(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "example \u{00b7} status = refused", TITLE);
    let lines = vec![
        Line::from(vec![t("  "), c("\u{25cf} refused", WARN), m("   policy stopped the call")]),
        Line::from(m("  {")),
        kv("status", "\"refused\"", WARN),
        Line::from(vec![c("    \"payload\"", TITLE), m(": {")]),
        Line::from(vec![c("      \"applied\"", TITLE), m(": "), t("false"), m(",")]),
        Line::from(vec![c("      \"reason\"", TITLE), m(": "), t("\"must explore first\"")]),
        Line::from(m("    },")),
        Line::from(vec![c("    \"truncated\"", TITLE), m(": "), c("false", MUTED)]),
        Line::from(m("  }")),
        Line::from(vec![
            m("  "),
            c("\u{25cf} error", BAD),
            m(" ran & failed \u{00b7} "),
            c("\u{25cf} refused", WARN),
            m(" never ran"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn kv(key: &str, val: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        c(&format!("    \"{key}\""), TITLE),
        m(": "),
        c(val, color),
        m(","),
    ])
}

fn render_feed(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "recent \u{00b7} tool calls", TITLE);
    f.render_widget(Paragraph::new(feed_table()), inner);
}

// Row width = 1 + Σw + 3n = 1 + 104 + 15 = 120, so the feed box spans the frame.
const COLS_FEED: [usize; 5] = [8, 10, 16, 24, 46];

fn feed_header() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["TIME", "STATUS", "ACTION", "DETAIL", "NOTE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS_FEED) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn feed_row(time: &str, status: &str, color: Color, tool: &str, detail: &str, note: &str) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{time:<w$}", w = COLS_FEED[0]), DIM)]));
    spans.extend(cell(vec![c(
        &format!("{:<w$}", format!("\u{25cf} {status}"), w = COLS_FEED[1]),
        color,
    )]));
    spans.extend(cell(vec![t(&format!("{tool:<w$}", w = COLS_FEED[2]))]));
    spans.extend(cell(vec![m(&format!("{detail:<w$}", w = COLS_FEED[3]))]));
    spans.extend(cell(vec![m(&format!("{note:<w$}", w = COLS_FEED[4]))]));
    Line::from(spans)
}

fn feed_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS_FEED, "\u{250c}", "\u{252c}", "\u{2510}"),
        feed_header(),
        chrome::table_rule(&COLS_FEED, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        feed_row("10:24:31", "ok", OK, "Read", "routes.rs:12\u{2013}48", "complete"),
        feed_row("10:24:29", "ok", OK, "Search", "\"rate limit\" \u{00b7} 3 hits", "complete"),
        feed_row("10:24:22", "refused", WARN, "Edit", "must explore first", "never ran"),
        feed_row("10:24:18", "ok", OK, "Run shell", "cargo test \u{00b7} 48 pass", "clipped"),
        feed_row("10:24:05", "error", BAD, "Web lookup", "timeout after 10s", "ran & failed"),
        feed_row("10:23:57", "ok", OK, "List tree", "crates/server/src", "complete"),
        feed_row("10:23:44", "ok", OK, "Read", "auth/mod.rs:1\u{2013}60", "complete"),
        feed_row("10:23:30", "ok", OK, "Plan note", "wave 2 \u{00b7} 3 tasks", "complete"),
    ]);
    lines.push(chrome::table_rule(&COLS_FEED, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
