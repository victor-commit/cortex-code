//! Screen 3 — workflow monitor, framed as a mission-control VIEW a keybinding
//! opens over the session (no composer — you read the run, you don't type into
//! it). A lifecycle-phase ribbon, a dense per-feature verdict table, then the
//! runtime loop and wave scheduling side by side, over a lifecycle event feed.

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

// Fixed column widths keep the feature table a real grid, not ragged prose.
// Row width = 1 + Σ(w+2) + n = 1 + 113 + 6 = 120, so the box spans the frame.
const COLS: [usize; 6] = [18, 10, 13, 8, 34, 18];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Workflow Monitor",
        &[
            ("enter", "inspect feature"),
            ("f", "filter"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // phase ribbon
            Constraint::Length(1),  // spacer
            Constraint::Length(1),  // features rule
            Constraint::Length(11), // feature table (box)
            Constraint::Length(1),  // spacer
            Constraint::Min(0),     // runtime + wave + feed
        ])
        .split(body);

    f.render_widget(Paragraph::new(ribbon()), rows[0]);
    chrome::rule_head(f, rows[2], "features \u{00b7} worker verdict \u{2192} expected artifact", TITLE);
    f.render_widget(Paragraph::new(feature_table()), rows[3]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // runtime loop + wave scheduling
            Constraint::Min(0),     // lifecycle event feed (box)
        ])
        .split(rows[5]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(lower[0]);
    render_runtime(f, split[0]);
    render_wave(f, split[1]);
    render_feed(f, lower[1]);
}

/// The six lifecycle phases A-F as one flow line — a state dot + short name each
/// (done / active / wait), with the verified-feature tally pinned on the right.
fn ribbon() -> Line<'static> {
    let seg = |letter: &str, name: &str, glyph: &str, color: Color, label: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(glyph.to_string(), Style::default().fg(color)),
            Span::styled(format!(" {letter} "), Style::default().fg(color)),
            Span::styled(name.to_string(), Style::default().fg(label)),
        ]
    };
    let arrow = || m("  \u{2192}  ");
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" phase  ", Style::default().fg(DIM))];
    spans.extend(seg("A", "plan", "\u{25cf}", OK, DIM));
    spans.push(arrow());
    spans.extend(seg("B", "design", "\u{25cf}", OK, DIM));
    spans.push(arrow());
    spans.extend(seg("C", "build", "\u{25cf}", OK, DIM));
    spans.push(arrow());
    spans.extend(seg("D", "runtime", "\u{25c9}", ACCENT, TXT));
    spans.push(arrow());
    spans.extend(seg("E", "validate", "\u{25cb}", MUTED, MUTED));
    spans.push(arrow());
    spans.extend(seg("F", "gate", "\u{25cb}", MUTED, MUTED));
    spans.push(m("      "));
    spans.push(Span::styled("verified ", Style::default().fg(DIM)));
    spans.push(Span::styled("4/7", Style::default().fg(TXT)));
    spans.push(m("   "));
    spans.push(Span::styled("failed ", Style::default().fg(DIM)));
    spans.push(Span::styled("1", Style::default().fg(TXT)));
    Line::from(spans)
}

fn feature_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        row("feat-db-schema", "m1-auth", "verified", OK, "4/4", "sessions table + schema migration", "ACC-AUTH-002"),
        row("feat-auth-login", "m1-auth", "verified", OK, "5/5", "POST /login issues JWT", "ACC-AUTH-001"),
        row("feat-session-mgr", "m1-auth", "verified", OK, "3/3", "revoke + refresh endpoints", "ACC-AUTH-003"),
        row("feat-api-routes", "m2-api", "partial", WARN, "2/4", "429 on 6th attempt \u{00b7} in loop", "ACC-API-010"),
        row("feat-web-ui", "m2-api", "pending", MUTED, "0/3", "login form + session guard", "ACC-WEB-003"),
        row("feat-audit-log", "m2-api", "failed", BAD, "0/2", "auth events \u{2192} audit sink", "ACC-AUD-004"),
        row("feat-rate-limit", "m2-api", "verified", OK, "3/3", "sliding-window limiter", "ACC-API-011"),
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
    let labels = ["FEATURE", "MILESTONE", "VERDICT", "CHECKS", "ARTIFACT", "FULFILLS"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

#[allow(clippy::too_many_arguments)]
fn row(
    feat: &str,
    ms: &str,
    verdict: &str,
    verdict_color: Color,
    checks: &str,
    artifact: &str,
    acc: &str,
) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![t(&format!("{feat:<w$}", w = COLS[0]))]));
    spans.extend(cell(vec![m(&format!("{ms:<w$}", w = COLS[1]))]));
    let glyph = if verdict_color == BAD { chrome::state_glyph(verdict_color) } else { "\u{25cf}" };
    spans.extend(cell(vec![c(&format!("{:<w$}", format!("{glyph} {verdict}"), w = COLS[2]), verdict_color)]));
    spans.extend(cell(vec![t(&format!("{checks:<w$}", w = COLS[3]))]));
    spans.extend(cell(vec![m(&format!("{artifact:<w$}", w = COLS[4]))]));
    spans.extend(cell(vec![m(&format!("{acc:<w$}", w = COLS[5]))]));
    Line::from(spans)
}

fn render_runtime(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "phase D \u{00b7} runtime loop", TITLE);
    let step = |glyph: &str, id: &str, name: &str, color: Color, label: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(format!("{glyph} "), Style::default().fg(color)),
            Span::styled(format!("{id} "), Style::default().fg(color)),
            Span::styled(format!("{name}   "), Style::default().fg(label)),
        ]
    };
    let mut top: Vec<Span<'static>> = vec![t("  ")];
    top.extend(step("\u{25cf}", "D1", "spawn", OK, DIM));
    top.extend(step("\u{25cf}", "D2", "worktree", OK, DIM));
    top.extend(step("\u{25c9}", "D4", "work", ACCENT, TXT));
    let mut bot: Vec<Span<'static>> = vec![t("  ")];
    bot.extend(step("\u{25cb}", "D6", "finish", MUTED, MUTED));
    bot.extend(step("\u{25cb}", "D7", "reduce", MUTED, MUTED));
    bot.extend(step("\u{25cb}", "D8", "merge-back", MUTED, MUTED));
    let lines = vec![
        Line::from(m("  run_workflow is blocking \u{00b7} runner drives features.json")),
        Line::from(""),
        Line::from(top),
        Line::from(bot),
        Line::from(""),
        Line::from(vec![c(&format!("  {:<10}", "worker"), MUTED), c("ws-3f9a2c", INFO)]),
        Line::from(vec![c(&format!("  {:<10}", "branch"), MUTED), t("workflow/wf-42/feat-api-routes")]),
        Line::from(""),
        Line::from(vec![c(&format!("  {:<10}", "elapsed"), MUTED), t("4m 12s \u{00b7} attempt 1/3")]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_wave(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "wave \u{00b7} scheduling", TITLE);
    let field = |k: &str, v: &str| -> Line<'static> {
        Line::from(vec![c(&format!("  {k:<13}"), MUTED), t(v)])
    };
    let lines = vec![
        field("wave", "2"),
        field("pairs", "3"),
        Line::from(vec![c(&format!("  {:<13}", "intersecting"), MUTED), c("0", OK)]),
        Line::from(vec![
            c(&format!("  {:<13}", "rate"), MUTED),
            c("0.0%", OK),
            m("  below "),
            c("15%", WARN),
            m(" threshold"),
        ]),
        field("worktrees", "3 \u{00b7} dispatched 3/3"),
        Line::from(""),
        Line::from(vec![t("  "), c("\u{25cf} expanding", OK), m("  parallelism widening")]),
        Line::from(vec![c(&format!("  {:<13}", "end gate"), MUTED), c("\u{25cf} 2/4 passed", WARN)]),
        Line::from(vec![c(&format!("  {:<13}", ""), MUTED), m("+ README present")]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_feed(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "recent \u{00b7} workflow events", TITLE);
    f.render_widget(Paragraph::new(feed_table()), inner);
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
        feed_row("14:07:19", "runner", ACCENT, "feat-api-routes \u{2192} partial", "2/4 assertions \u{00b7} reduce"),
        feed_row("14:06:41", "ws-3f9a2c", INFO, "D4 work on feat-api-routes", "routes.rs \u{00b7} handlers.rs"),
        feed_row("14:06:02", "runner", ACCENT, "D2 worktree \u{2192} feat-api-routes", "workflow/wf-42 branch"),
        feed_row("14:05:33", "runner", ACCENT, "D1 spawn \u{2192} wave 2 dispatched", "3 pairs \u{00b7} N=3"),
        feed_row("14:05:02", "runner", ACCENT, "feat-session-mgr \u{2192} completed", "verdict verified"),
    ]);
    lines.push(chrome::table_rule(&COLS_FEED, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
