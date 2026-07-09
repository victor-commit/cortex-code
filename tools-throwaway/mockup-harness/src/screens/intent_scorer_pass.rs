//! Intent-scoring scorecard variants share one D1-D5 table anatomy.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, OK, WARN};

const DIMENSION_W: usize = 27;
const GATE_W: usize = 10;

struct Dimension {
    name: &'static str,
    score: u8,
    gated: bool,
    evidence: &'static str,
}

const PASS_DIMENSIONS: &[Dimension] = &[
    Dimension {
        name: "D1 Intent fidelity",
        score: 2,
        gated: true,
        evidence: "requested outcome is present",
    },
    Dimension {
        name: "D2 Execution evidence",
        score: 2,
        gated: false,
        evidence: "checks and surface evidence captured",
    },
    Dimension {
        name: "D3 Scope discipline",
        score: 1,
        gated: true,
        evidence: "one cleanup noted for follow-up",
    },
    Dimension {
        name: "D4 Report honesty",
        score: 2,
        gated: true,
        evidence: "summary matches the submitted diff",
    },
    Dimension {
        name: "D5 Reliability",
        score: 1,
        gated: false,
        evidence: "one retry observed, result held",
    },
];

const FAIL_DIMENSIONS: &[Dimension] = &[
    Dimension {
        name: "D1 Intent fidelity",
        score: 2,
        gated: true,
        evidence: "requested outcome is present",
    },
    Dimension {
        name: "D2 Execution evidence",
        score: 2,
        gated: false,
        evidence: "checks and surface evidence captured",
    },
    Dimension {
        name: "D3 Scope discipline",
        score: 0,
        gated: true,
        evidence: "unrequested helper changes the scope",
    },
    Dimension {
        name: "D4 Report honesty",
        score: 2,
        gated: true,
        evidence: "summary matches the submitted diff",
    },
    Dimension {
        name: "D5 Reliability",
        score: 2,
        gated: false,
        evidence: "repeat run held the same result",
    },
];

enum Variant {
    Pass,
    Fail,
}

pub fn render(f: &mut Frame) {
    render_variant(f, Variant::Pass);
}

pub(crate) fn render_fail(f: &mut Frame) {
    render_variant(f, Variant::Fail);
}

fn render_variant(f: &mut Frame, variant: Variant) {
    let status = StatusLine::default();
    let title = match variant {
        Variant::Pass => "intent scorer · report",
        Variant::Fail => "intent scorer · gate failed",
    };
    let body = chrome::cli_view(f, title, &status);
    let dimensions = match variant {
        Variant::Pass => PASS_DIMENSIONS,
        Variant::Fail => FAIL_DIMENSIONS,
    };

    let mut lines = vec![
        header_keys(),
        Line::from(""),
        metadata_row("subject", "feat-api-routes · wf-42"),
        metadata_row("worker", "ws-77b1e0"),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("scorecard")]),
        chrome::rule_line(105),
        score_head(),
    ];
    lines.extend(dimensions.iter().map(score_row));
    lines.extend([
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("verdict")]),
        chrome::rule_line(105),
    ]);
    lines.extend(verdict_rows(variant));
    lines.extend([
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("scoring rule")]),
        chrome::rule_line(105),
        score_scale(),
        Line::from(vec![
            Span::raw("   "),
            m("Pass requires total at least 8 and every gated dimension at least 1."),
        ]),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Line<'static> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("enter", "inspect dimension"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("e", "show evidence"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("j/k", "move"));
    Line::from(spans)
}

fn metadata_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{label:<10}")),
        t(value),
    ])
}

fn score_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        m(&format!("{:<DIMENSION_W$}", "DIMENSION")),
        m("SCORE / CONF.      "),
        m(&format!("{:<GATE_W$}", "GATE")),
        m("EVIDENCE"),
    ])
}

fn score_row(dimension: &Dimension) -> Line<'static> {
    let (glyph, word, hue) = score_cell(dimension.score);
    let failed = dimension.gated && dimension.score == 0;
    let marker = if failed {
        c("│ ", BAD)
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![
        Span::raw("   "),
        marker,
        t(&format!("{:<DIMENSION_W$}", dimension.name)),
        c(&format!("{glyph} {word:<7}"), hue),
        Span::raw("  "),
    ];
    spans.extend(chrome::bar_in(dimension.score as usize, 2, 6, hue));
    spans.extend([
        Span::raw("   "),
        m(&format!(
            "{:<GATE_W$}",
            if dimension.gated { "gated" } else { "n/a" }
        )),
        t(dimension.evidence),
    ]);
    Line::from(spans)
}

fn score_cell(score: u8) -> (&'static str, &'static str, ratatui::style::Color) {
    match score {
        2 => ("✓", "full", OK),
        1 => ("●", "partial", WARN),
        _ => ("○", "miss", BAD),
    }
}

fn verdict_rows(variant: Variant) -> Vec<Line<'static>> {
    match variant {
        Variant::Pass => {
            let mut total = vec![Span::raw("   "), m("total           "), c("8/10  ", OK)];
            total.extend(chrome::bar_in(8, 10, 10, OK));
            vec![
                Line::from(total),
                Line::from(vec![
                    Span::raw("   "),
                    m("effectiveness   "),
                    c("✓ PASS", OK),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    m("repo-readiness  "),
                    c("✓ PASS", OK),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    c("✓ PASS", OK),
                    m("  ·  all gated dimensions cleared the score floor"),
                ]),
            ]
        }
        Variant::Fail => {
            let mut total = vec![Span::raw("   "), m("total           "), t("8/10  ")];
            total.extend(chrome::bar(8, 10, 10));
            let mut recovery = vec![Span::raw("   ")];
            recovery.extend(chrome::key_span("r", "re-score after fix"));
            vec![
                Line::from(total),
                Line::from(vec![
                    Span::raw("   "),
                    c("✗ FAIL", BAD),
                    m("  ·  gated D3 scored 0"),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    m("rule            "),
                    t("A gated 0 fails the run regardless of total."),
                ]),
                Line::from(vec![
                    Span::raw("   "),
                    m("fix             "),
                    t("Remove the unrelated helper, then re-score the scoped change."),
                ]),
                Line::from(recovery),
            ]
        }
    }
}

fn score_scale() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("○ miss"),
        m("  0  ·  "),
        m("● partial"),
        m("  1  ·  "),
        m("✓ full"),
        m("  2"),
    ])
}
