//! Screen 6 — the intent scorer, framed as a mission-control VIEW a keybinding
//! opens over the session (no composer — you read the report, you don't type
//! into it). A per-dimension score ribbon, a dense rubric table, then the run
//! verdict and the scoring rubric side by side, over a captured-evidence feed.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, bar, c, m, t, ACCENT, BAD, DIM, MUTED, OK, TITLE, TXT, WARN,
};

// Fixed column widths keep the rubric a real grid, not ragged prose.
// Row width = 1 + Σ(w+2) + n = 1 + 115 + 4 = 120, so the box spans the frame.
const COLS: [usize; 4] = [24, 12, 10, 61];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Intent Scorer",
        &[
            ("enter", "open dimension"),
            ("e", "show evidence"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // dimension ribbon
            Constraint::Length(1), // spacer
            Constraint::Length(1), // rubric rule
            Constraint::Length(9), // dimension table (box)
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // verdict + rubric + evidence
        ])
        .split(body);

    f.render_widget(Paragraph::new(ribbon()), rows[0]);
    chrome::rule_head(f, rows[2], "scores \u{00b7} 5 dimensions", TITLE);
    f.render_widget(Paragraph::new(score_table()), rows[3]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // verdict + rubric
            Constraint::Min(0),
        ])
        .split(rows[5]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(lower[0]);
    render_verdict(f, split[0]);
    render_rubric(f, split[1]);
}

/// The five dimensions as one flow line — a score dot each (full / partial /
/// miss) with the gated total pinned on the right.
fn ribbon() -> Line<'static> {
    let seg = |name: &str, glyph: &str, color: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(glyph.to_string(), Style::default().fg(color)),
            Span::styled(format!(" {name}"), Style::default().fg(DIM)),
        ]
    };
    let arrow = || m("  \u{2192}  ");
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" scorer  ", Style::default().fg(DIM))];
    spans.extend(seg("D1", "\u{25cf}", OK));
    spans.push(arrow());
    spans.extend(seg("D2", "\u{25cf}", OK));
    spans.push(arrow());
    spans.extend(seg("D3", "\u{25d0}", WARN));
    spans.push(arrow());
    spans.extend(seg("D4", "\u{25cf}", OK));
    spans.push(arrow());
    spans.extend(seg("D5", "\u{25d0}", WARN));
    spans.push(m("        "));
    spans.push(Span::styled("total ", Style::default().fg(DIM)));
    spans.push(Span::styled("8/10", Style::default().fg(TXT)));
    Line::from(spans)
}

fn score_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        row("D1 Intent fidelity", 2, true, "did exactly what was asked"),
        row("D2 Execution-verified", 2, false, "tests + curl surface captured"),
        row("D3 Scope discipline", 1, true, "one unrequested helper added"),
        row("D4 Report honesty", 2, true, "claims match the diff"),
        row("D5 Reliability", 1, false, "one flaky retry observed"),
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
    let labels = ["DIMENSION", "SCORE", "GATE", "EVIDENCE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn row(name: &str, score: u8, gate: bool, ev: &str) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{name:<w$}", w = COLS[0]), TITLE)]));
    spans.extend(cell(vec![score_cell(score)]));
    spans.extend(cell(vec![gate_cell(gate, score)]));
    spans.extend(cell(vec![m(&format!("{ev:<w$}", w = COLS[3]))]));
    Line::from(spans)
}

/// Score cell `● full` / `◐ partial` / `○ miss` in the status hue, padded to the
/// column width so the box separator always aligns.
fn score_cell(score: u8) -> Span<'static> {
    let (glyph, word, color) = match score {
        2 => ("\u{25cf}", "full", OK),
        1 => ("\u{25d0}", "partial", WARN),
        _ => ("\u{25cb}", "miss", BAD),
    };
    c(&format!("{:<w$}", format!("{glyph} {word}"), w = COLS[1]), color)
}

/// Gate cell: `● ok` when a gated dimension clears ≥1, `● BLOCK` when it misses,
/// `—` for ungated dimensions.
fn gate_cell(gate: bool, score: u8) -> Span<'static> {
    if !gate {
        return m(&format!("{:<w$}", "\u{2014}", w = COLS[2]));
    }
    if score >= 1 {
        c(&format!("{:<w$}", "\u{25cf} ok", w = COLS[2]), OK)
    } else {
        c(&format!("{:<w$}", "\u{25cf} BLOCK", w = COLS[2]), BAD)
    }
}

fn render_verdict(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "verdict \u{00b7} feat-api-routes", TITLE);
    let field = |k: &str, v: &str| -> Line<'static> {
        Line::from(vec![c(&format!("  {k:<16}"), MUTED), t(v)])
    };
    let lines = vec![
        field("subject", "feat-api-routes @ wf-42"),
        field("worker", "ws-77b1e0"),
        Line::from(""),
        Line::from(vec![
            c(&format!("  {:<16}", "total"), MUTED),
            Span::styled("8/10  ", Style::default().fg(OK).add_modifier(Modifier::BOLD)),
            c(&bar(8, 10), OK),
        ]),
        Line::from(vec![
            c(&format!("  {:<16}", "effectiveness"), MUTED),
            Span::styled("\u{25cf} PASS", Style::default().fg(OK)),
            m("     (PASS / FAIL / PARTIAL)"),
        ]),
        Line::from(vec![
            c(&format!("  {:<16}", "repo-readiness"), MUTED),
            Span::styled("\u{25cf} PASS", Style::default().fg(OK)),
            m("     (PASS / FAIL / N-A)"),
        ]),
        Line::from(vec![
            c(&format!("  {:<16}", "gates"), MUTED),
            Span::styled("\u{25cf} 3/3 met", Style::default().fg(OK)),
        ]),
        Line::from(""),
        Line::from(vec![
            c(&format!("  {:<16}", "note"), MUTED),
            m("D3 cleared the \u{2265}1 gate \u{00b7} trim dead weight next"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_rubric(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "rubric \u{00b7} scoring", TITLE);
    let lines = vec![
        Line::from(m("  each dimension is earned 0/1/2 and gate-checked")),
        Line::from(""),
        Line::from(vec![t("  "), c("\u{25cf} full", OK), m("     did it \u{00b7} scores 2")]),
        Line::from(vec![t("  "), c("\u{25d0} partial", WARN), m("  incomplete \u{00b7} scores 1")]),
        Line::from(vec![t("  "), c("\u{25cb} miss", BAD), m("     absent \u{00b7} scores 0")]),
        Line::from(""),
        rule_item("gated dims", "D1 \u{00b7} D3 \u{00b7} D4 must be \u{2265} 1"),
        rule_item("any miss", "a gated 0 fails the run"),
        rule_item("threshold", "\u{2265} 8/10 to pass"),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

/// A rubric-policy row: `● name  description`, dot in the accent hue (a static
/// rule, not a run status — hence ORANGE, never a status color).
fn rule_item(name: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        t("  "),
        chrome::dot(ACCENT),
        c(&format!(" {name:<12}"), ACCENT),
        m(desc),
    ])
}
