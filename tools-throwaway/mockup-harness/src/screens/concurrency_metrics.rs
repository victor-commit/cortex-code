//! Screen 7 — concurrency metrics, framed as a mission-control VIEW a keybinding
//! opens over the session (no composer — you read the metrics, you don't type
//! into it). A per-wave conflict-rate ribbon, a dense rate table, then the
//! merge-back ledger and the quarantine detail side by side, over an event feed.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, m, t, ACCENT, BAD, DIM, INFO, MUTED, OK, TITLE, TXT, WARN,
};

// Fixed column widths keep the rate table a real grid, not ragged prose.
const WAVE_W: usize = 8;
const PAIRS_W: usize = 6;
const INTER_W: usize = 13;
const RATE_W: usize = 7;

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Concurrency Metrics",
        &[
            ("\u{2190}\u{2192}", "wave"),
            ("e", "expand event"),
            ("R", "reattach \u{2192} READY"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // conflict-rate ribbon
            Constraint::Length(1), // spacer
            Constraint::Length(1), // rate rule
            Constraint::Length(1), // table header
            Constraint::Length(3), // wave rows
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // merge-back + quarantine + feed
        ])
        .split(body);

    f.render_widget(Paragraph::new(ribbon()), rows[0]);
    chrome::rule_head(f, rows[2], "waves \u{00b7} conflict rate", TITLE);
    f.render_widget(Paragraph::new(table_header()), rows[3]);
    f.render_widget(Paragraph::new(wave_rows()), rows[4]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // merge-back + quarantine
            Constraint::Min(0),     // event feed (box)
        ])
        .split(rows[6]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(lower[0]);
    render_mergeback(f, split[0]);
    render_quarantine(f, split[1]);
    render_feed(f, lower[1]);
}

/// The three waves as one flow line — a rate dot + state each (clear / hot),
/// with the parallelism decision pinned on the right.
fn ribbon() -> Line<'static> {
    let seg = |name: &str, glyph: &str, state: &str, pct: &str, color: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(glyph.to_string(), Style::default().fg(color)),
            Span::styled(format!(" {name} "), Style::default().fg(DIM)),
            Span::styled(state.to_string(), Style::default().fg(color)),
            Span::styled(format!(" {pct}"), Style::default().fg(DIM)),
        ]
    };
    let arrow = || m("  \u{2192}  ");
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" waves  ", Style::default().fg(DIM))];
    spans.extend(seg("w1", "\u{25cf}", "clear", "0%", OK));
    spans.push(arrow());
    spans.extend(seg("w2", "\u{25cf}", "clear", "0%", OK));
    spans.push(arrow());
    spans.extend(seg("w3", "\u{25d0}", "hot", "16.7%", WARN));
    spans.push(m("      "));
    spans.push(Span::styled("decision ", Style::default().fg(DIM)));
    spans.push(Span::styled("hold @ N=3", Style::default().fg(TXT)));
    spans.push(m("      "));
    spans.push(Span::styled("workers ", Style::default().fg(DIM)));
    spans.push(Span::styled("3/3", Style::default().fg(TXT)));
    spans.push(m("      "));
    spans.push(Span::styled("quarantine ", Style::default().fg(DIM)));
    spans.push(Span::styled("1", Style::default().fg(TXT)));
    Line::from(spans)
}

fn table_header() -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(
            " {:<ww$}{:>pw$}  {:>iw$}  {:>rw$}  {}",
            "WAVE",
            "PAIRS",
            "INTERSECTING",
            "RATE",
            "DECISION",
            ww = WAVE_W,
            pw = PAIRS_W,
            iw = INTER_W,
            rw = RATE_W,
        ),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )])
}

fn wave_rows() -> Vec<Line<'static>> {
    vec![
        wave("wave 1", 1, 0, "WIDEN", OK),
        wave("wave 2", 3, 0, "WIDEN", OK),
        wave("wave 3", 6, 1, "HOLD", BAD),
    ]
}

fn wave(name: &str, pairs: u32, inter: u32, decision: &str, decision_color: Color) -> Line<'static> {
    let pct = if pairs == 0 { 0.0 } else { inter as f32 / pairs as f32 * 100.0 };
    let rate = format!("{pct:.1}%");
    Line::from(vec![
        Span::styled(format!(" {name:<w$}", w = WAVE_W), Style::default().fg(TXT)),
        t(&format!("{pairs:>PAIRS_W$}  ")),
        t(&format!("{inter:>INTER_W$}  ")),
        t(&format!("{rate:>RATE_W$}  ")),
        c(&format!("\u{25cf} {decision}"), decision_color),
    ])
}

fn render_mergeback(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "merge-back \u{00b7} integration", TITLE);
    let row = |feat: &str, glyph: &str, state: &str, color: Color, note: &str| -> Line<'static> {
        Line::from(vec![
            c(&format!("  {feat:<20}"), TITLE),
            c(&format!("{glyph} {state:<14}"), color),
            m(note),
        ])
    };
    let lines = vec![
        row("feat-db-schema", "\u{25cf}", "completed", OK, "wave 1"),
        row("feat-auth-login", "\u{25cf}", "completed", OK, "wave 2"),
        row("feat-api-routes", "\u{25cf}", "merge blocked", BAD, "wave 3 \u{00b7} routes.rs"),
        Line::from(""),
        Line::from(m("  worker branch \u{2192} integration branch")),
        Line::from(vec![
            m("  on conflict: "),
            Span::styled("merge blocked", Style::default().fg(BAD).add_modifier(Modifier::BOLD)),
            m(" \u{00b7} never force-resolves"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_quarantine(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "quarantine \u{00b7} held", TITLE);
    let lines = vec![
        Line::from(vec![c(&format!("  {:<12}", "ref"), MUTED), c("refs/workflow/quarantine/", INFO)]),
        Line::from(vec![c(&format!("  {:<12}", ""), MUTED), c("feat-api-routes", INFO)]),
        Line::from(""),
        Line::from(vec![t("  "), c("\u{25cf} merge conflict", BAD), m("  overlapping")]),
        Line::from(vec![c(&format!("  {:<12}", ""), MUTED), m("edits in routes.rs")]),
        Line::from(vec![t("  "), c("\u{25cf} test regression", WARN), m("  suite red")]),
        Line::from(""),
        Line::from(""),
        Line::from(vec![c(&format!("  {:<12}", "disposition"), MUTED), t("held for triage")]),
        Line::from(vec![c(&format!("  {:<12}", "reattach"), MUTED), m("re-post as READY once resolved")]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_feed(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "recent \u{00b7} concurrency events", TITLE);
    f.render_widget(Paragraph::new(feed_table()), inner);
}

fn cell(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(chrome::vsep());
    out
}

// Row width = 1 + Σw + 3n = 1 + 107 + 12 = 120, so the feed box spans the frame.
const COLS_FEED: [usize; 4] = [8, 13, 34, 52];

fn feed_header() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["TIME", "WHO", "EVENT", "NOTE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS_FEED) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn feed_row(time: &str, who: &str, who_color: Color, verb: &str, note: &str) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{time:<w$}", w = COLS_FEED[0]), DIM)]));
    spans.extend(cell(vec![c(
        &format!("{:<w$}", format!("\u{25cf} {who}"), w = COLS_FEED[1]),
        who_color,
    )]));
    spans.extend(cell(vec![t(&format!("{verb:<w$}", w = COLS_FEED[2]))]));
    spans.extend(cell(vec![m(&format!("{note:<w$}", w = COLS_FEED[3]))]));
    Line::from(spans)
}

fn feed_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS_FEED, "\u{250c}", "\u{252c}", "\u{2510}"),
        feed_header(),
        chrome::table_rule(&COLS_FEED, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        feed_row("14:07:12", "wave 3", BAD, "conflict on routes.rs \u{2192} HOLD", "16.7% \u{2265} 15% threshold"),
        feed_row("14:07:04", "core", ACCENT, "feat-api-routes \u{2192} merge blocked", "routed to quarantine"),
        feed_row("14:06:58", "core", ACCENT, "quarantine ref created", "held for triage"),
        feed_row("14:06:41", "wave 3", WARN, "intersecting pair detected", "routes.rs \u{00b7} handlers.rs"),
        feed_row("14:06:15", "core", ACCENT, "wave 3 dispatched \u{2192} 6 pairs", "N=3 workers live"),
        feed_row("14:05:31", "wave 2", OK, "feat-auth-login merged clean", "0 intersecting pairs"),
        feed_row("14:05:02", "wave 2", OK, "feat-session-store merged clean", "0 intersecting pairs"),
        feed_row("14:04:48", "core", ACCENT, "wave 2 dispatched \u{2192} 3 pairs", "parallelism widened"),
        feed_row("14:03:22", "wave 1", OK, "feat-db-schema merged clean", "0 intersecting pairs"),
        feed_row("14:02:55", "core", ACCENT, "integration branch synced", "fast-forward @ main"),
        feed_row("14:02:10", "core", ACCENT, "parallelism widened \u{2192} N=3", "wave 3 dispatched"),
        feed_row("14:01:37", "core", ACCENT, "wave 1 dispatched \u{2192} 1 pair", "cold start N=1"),
    ]);
    lines.push(chrome::table_rule(&COLS_FEED, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
