//! Screen 8 — the verification ladder, framed as a mission-control VIEW a
//! keybinding opens over the session (no composer — you read the ladder, you
//! don't type into it). A one-line L0→L4 progression ribbon, a dense gate
//! table, then the live submission and the held-out masking policy side by side.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, m, t, ACCENT, DIM, INFO, MUTED, OK, TITLE, TXT, WARN,
};

// Fixed column widths keep the ladder a real grid, not ragged prose.
const LVL_W: usize = 4;
const NAME_W: usize = 22;
const VIS_W: usize = 11;
const RESULT_W: usize = 12;

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Verification Ladder",
        &[
            ("enter", "open level"),
            ("r", "re-run gate"),
            ("j/k", "move"),
            ("q", "close view"),
        ],
    );

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // progression ribbon
            Constraint::Length(1), // spacer
            Constraint::Length(1), // ladder rule
            Constraint::Length(1), // table header
            Constraint::Length(5), // level rows
            Constraint::Length(1), // spacer
            Constraint::Min(0),    // submission + policy + activity
        ])
        .split(body);

    f.render_widget(Paragraph::new(progression()), rows[0]);
    chrome::rule_head(f, rows[2], "ladder \u{00b7} gates L0\u{2013}L4", TITLE);
    f.render_widget(Paragraph::new(table_header()), rows[3]);
    f.render_widget(Paragraph::new(level_rows()), rows[4]);

    let lower = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // submission + policy
            Constraint::Length(1),  // spacer
            Constraint::Min(0),     // activity feed
        ])
        .split(rows[6]);

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(54), Constraint::Percentage(46)])
        .split(lower[0]);
    render_submission(f, split[0]);
    render_policy(f, split[1]);
    render_activity(f, lower[2]);
}

/// The five levels as one flow line — a status dot per level (pass / review /
/// pending) with the running gate tally pinned on the right.
fn progression() -> Line<'static> {
    let seg = |name: &str, glyph: &str, state: &str, color: Color| -> Vec<Span<'static>> {
        vec![
            Span::styled(glyph.to_string(), Style::default().fg(color)),
            Span::styled(format!(" {name} "), Style::default().fg(DIM)),
            Span::styled(state.to_string(), Style::default().fg(color)),
        ]
    };
    let arrow = || m("  \u{2192}  ");
    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ladder  ", Style::default().fg(DIM))];
    spans.extend(seg("L0", "\u{25cf}", "pass", OK));
    spans.push(arrow());
    spans.extend(seg("L1", "\u{25cf}", "pass", OK));
    spans.push(arrow());
    spans.extend(seg("L2", "\u{25cf}", "pass", OK));
    spans.push(arrow());
    spans.extend(seg("L3", "\u{25d0}", "review", WARN));
    spans.push(arrow());
    spans.extend(seg("L4", "\u{25cb}", "pending", MUTED));
    spans.push(m("      "));
    spans.push(Span::styled("gate ", Style::default().fg(DIM)));
    spans.push(Span::styled("3/5", Style::default().fg(TXT)));
    Line::from(spans)
}

fn table_header() -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(
            " {:<lw$}{:<nw$}{:<vw$}{:<rw$}{}",
            "LVL",
            "GATE",
            "VISIBILITY",
            "RESULT",
            "DETAIL",
            lw = LVL_W,
            nw = NAME_W,
            vw = VIS_W,
            rw = RESULT_W,
        ),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    )])
}

fn level_rows() -> Vec<Line<'static>> {
    vec![
        level("L0", "Deterministic gates", "visible", DIM, "\u{25cf}", "PASS", OK, "fmt \u{00b7} lint \u{00b7} type-check \u{00b7} build"),
        level("L1", "Agent-visible tests", "visible", DIM, "\u{25cf}", "PASS", OK, "48/48 unit + integration"),
        level("L2", "Held-out checks", "masked", INFO, "\u{25cf}", "PASS", OK, "12/12 hidden acceptance suite"),
        level("L3", "Intent audit", "masked", INFO, "\u{25d0}", "REVIEW", WARN, "reviewer's 5 questions over diff"),
        level("L4", "Process check", "masked", INFO, "\u{25cb}", "PENDING", MUTED, "evidence not yet collected"),
    ]
}

#[allow(clippy::too_many_arguments)]
fn level(
    id: &str,
    name: &str,
    vis: &str,
    vis_color: Color,
    result_glyph: &str,
    result: &str,
    result_color: Color,
    detail: &str,
) -> Line<'static> {
    // Result cell: `● RESULT` in the status hue, padded to a fixed width so the
    // DETAIL column always aligns regardless of the result word's length.
    let result_cell = format!("{result_glyph} {result}");
    let result_pad = RESULT_W.saturating_sub(result_cell.chars().count());
    Line::from(vec![
        Span::styled(
            format!(" {id:<w$}", w = LVL_W),
            Style::default().fg(TXT).add_modifier(Modifier::BOLD),
        ),
        t(&format!("{name:<NAME_W$}")),
        c(&format!("{vis:<VIS_W$}"), vis_color),
        c(&result_cell, result_color),
        t(&" ".repeat(result_pad)),
        m(detail),
    ])
}

fn render_submission(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "submission \u{00b7} feat-api-routes", TITLE);
    let field = |k: &str, v: &str| -> Line<'static> {
        Line::from(vec![c(&format!("  {k:<16}"), MUTED), t(v)])
    };
    let lines = vec![
        field("worker", "ws-77b1e0"),
        field("diff", "+142 \u{2212}38  \u{00b7}  6 files"),
        field("branch", "workflow/wf-42/feat-api-routes"),
        Line::from(""),
        field("blocking", "L3 intent \u{00b7} L4 process"),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_policy(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "held-out \u{00b7} why masked", TITLE);
    let lines = vec![
        Line::from(m("  worker sees L0/L1 to self-correct \u{00b7} L2\u{2013}L4 stay")),
        Line::from(m("  hidden so scores are earned, never gamed")),
        Line::from(""),
        rule_item("L2 cases", "live outside the worktree"),
        rule_item("L3 diff", "audited after submit, not before"),
        rule_item("L4 evidence", "\u{201c}un-run is not done\u{201d}"),
        Line::from(""),
        Line::from(vec![
            t("  "),
            c("\u{25c9} visible", DIM),
            m("   worker-facing context"),
        ]),
        Line::from(vec![
            t("  "),
            c("\u{25a0} masked", INFO),
            m("    held-out from the worker"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

/// A masking-policy row: `● name  description`, dot in the accent hue (a static
/// rule, not a run status — hence ORANGE, never a status color).
fn rule_item(name: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        t("  "),
        chrome::dot(ACCENT),
        c(&format!(" {name:<14}"), ACCENT),
        m(desc),
    ])
}

fn render_activity(f: &mut Frame, area: Rect) {
    let inner = chrome::rule_head(f, area, "recent \u{00b7} verification events", TITLE);
    f.render_widget(Paragraph::new(activity_table()), inner);
}

fn cell(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(chrome::vsep());
    out
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
        activity_row("14:03:22", "L3 intent", WARN, "review opened \u{2192} 2/5 answered", "reviewer ws-audit"),
        activity_row("14:03:05", "L2 held-out", OK, "12/12 acceptance passed", "core-run \u{00b7} 3.4s"),
        activity_row("14:02:40", "L1 tests", OK, "48/48 passing", "TDD loop closed"),
        activity_row("14:02:18", "L0 gates", OK, "fmt \u{00b7} lint \u{00b7} type \u{00b7} build", "0.8s"),
        activity_row("14:01:50", "core", ACCENT, "feat-api-routes submitted", "ws-77b1e0 \u{2192} core"),
        activity_row("13:59:31", "core", ACCENT, "feat-token-refresh accepted", "prior submission \u{2192} CLOSED"),
    ]);
    lines.push(chrome::table_rule(&COLS_ACT, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
