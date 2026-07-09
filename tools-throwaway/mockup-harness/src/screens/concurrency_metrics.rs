//! Wave concurrency and conflict handling with a linked quarantine record.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, MUTED, OK};

const WAVE_W: usize = 10;
const PAIRS_W: usize = 8;
const INTERSECT_W: usize = 15;
const RATE_W: usize = 9;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "concurrency metrics · wf-42", &status);
    let mut lines = vec![
        header_keys(),
        Line::from(""),
        metrics_ribbon(),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("per-wave conflicts")]),
        chrome::rule_line(105),
        conflict_head(),
        conflict_row("wave 1", 1, 0, "0.0%", ("●", "WIDEN", MUTED), false),
        conflict_row("wave 2", 3, 0, "0.0%", ("●", "WIDEN", MUTED), false),
        conflict_row("wave 3", 6, 1, "16.7%", ("✗", "HOLD", BAD), true),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("merge-back · integration")]),
        chrome::rule_line(105),
        merge_head(),
        merge_row("feat-db-schema", "✓", "merged", OK, "wave 1"),
        merge_row("feat-auth-login", "✓", "merged", OK, "wave 2"),
        merge_row(
            "feat-api-routes",
            "✗",
            "merge blocked",
            BAD,
            "wave 3 · routes.rs",
        ),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("quarantine")]),
        chrome::rule_line(105),
    ];
    lines.extend(quarantine_rows());
    lines.extend([
        Line::from(""),
        legend_line(
            "merge conflicts block merge-back · HOLD starts at 15% intersecting pairs",
        ),
        legend_line("held work returns READY only after the named conflict is resolved"),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("recent concurrency events")]),
        chrome::rule_line(105),
        event_head(),
        event("14:07", "wave 3", "conflict → HOLD", "routes.rs · 16.7%"),
        event("14:06", "core", "merge blocked", "feat-api-routes quarantined"),
        event("14:05", "core", "workers widened", "3/3 workers · 1.8k tokens/s"),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Line<'static> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("enter", "inspect wave"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("e", "expand event"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("j/k", "move"));
    Line::from(spans)
}

fn metrics_ribbon() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("waves  "),
        c("✓ 1 clear", OK),
        c("  →  ", FAINT),
        c("✓ 2 clear", OK),
        c("  →  ", FAINT),
        c("✗ 3 HOLD", BAD),
        m("  ·  workers  "),
        t("3/3"),
        m("  ·  throughput  "),
        t("1.8k tokens/s"),
    ])
}

fn conflict_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        m(&format!("{:<WAVE_W$}", "WAVE")),
        m(&format!("{:>PAIRS_W$}  ", "PAIRS")),
        m(&format!("{:>INTERSECT_W$}  ", "INTERSECTING")),
        m(&format!("{:>RATE_W$}  ", "RATE")),
        m("DECISION"),
    ])
}

fn conflict_row(
    wave: &str,
    pairs: u32,
    intersecting: u32,
    rate: &str,
    outcome: (&str, &str, ratatui::style::Color),
    blocked: bool,
) -> Line<'static> {
    let (glyph, decision, hue) = outcome;
    let marker = if blocked {
        c("│ ", BAD)
    } else {
        Span::raw("  ")
    };
    Line::from(vec![
        Span::raw("   "),
        marker,
        t(&format!("{wave:<WAVE_W$}")),
        t(&format!("{pairs:>PAIRS_W$}  ")),
        t(&format!("{intersecting:>INTERSECT_W$}  ")),
        t(&format!("{rate:>RATE_W$}  ")),
        c(&format!("{glyph} {decision}"), hue),
    ])
}

fn merge_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        m("FEATURE                 "),
        m("OUTCOME             "),
        m("REFERENCE"),
    ])
}

fn merge_row(
    feature: &str,
    glyph: &str,
    outcome: &str,
    hue: ratatui::style::Color,
    reference: &str,
) -> Line<'static> {
    let marker = if hue == BAD {
        c("│ ", BAD)
    } else {
        Span::raw("  ")
    };
    Line::from(vec![
        Span::raw("   "),
        marker,
        t(&format!("{feature:<24}")),
        c(&format!("{glyph} {outcome:<18}"), hue),
        m(reference),
    ])
}

fn quarantine_rows() -> Vec<Line<'static>> {
    let mut action = vec![Span::raw("   "), m("  action       ")];
    action.extend(chrome::key_span("r", "reattach"));
    action.push(c(" → ", FAINT));
    action.push(t("READY"));
    vec![
        Line::from(vec![
            Span::raw("   "),
            c("│ ", BAD),
            t("feat-api-routes"),
            m("  ·  "),
            c("✗ held for triage", BAD),
            m("  ·  wave 3 · routes.rs"),
        ]),
        Line::from(action),
    ]
}

fn legend_line(text: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), m("legend  "), m(text)])
}

fn event_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("TIME    "),
        m("WHO        "),
        m("EVENT                 "),
        m("DETAIL"),
    ])
}

fn event(time: &str, who: &str, action: &str, detail: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{time:<8}")),
        t(&format!("{who:<11}")),
        t(&format!("{action:<22}")),
        m(detail),
    ])
}
