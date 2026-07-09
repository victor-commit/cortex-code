//! One authoritative verification table, with held-out gates kept opaque.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, INFO, MUTED, OK};

const LEVEL_W: usize = 7;
const GATE_W: usize = 24;
const VISIBILITY_W: usize = 13;
const RESULT_W: usize = 11;

struct Rung {
    level: &'static str,
    gate: &'static str,
    visibility: &'static str,
    glyph: &'static str,
    result: &'static str,
    result_color: ratatui::style::Color,
    detail: &'static str,
}

const RUNGS: &[Rung] = &[
    Rung {
        level: "L0",
        gate: "Deterministic gates",
        visibility: "visible",
        glyph: "✓",
        result: "pass",
        result_color: OK,
        detail: "format, lint, type check, and build passed",
    },
    Rung {
        level: "L1",
        gate: "Agent-visible tests",
        visibility: "visible",
        glyph: "✓",
        result: "pass",
        result_color: OK,
        detail: "48 of 48 unit and integration tests passed",
    },
    Rung {
        level: "L2",
        gate: "Held-out checks",
        visibility: "masked",
        glyph: "✗",
        result: "fail",
        result_color: BAD,
        detail: "one held-out assertion needs repair",
    },
    Rung {
        level: "L3",
        gate: "Intent audit",
        visibility: "masked",
        glyph: "●",
        result: "review",
        result_color: INFO,
        detail: "3 questions remaining → enter",
    },
    Rung {
        level: "L4",
        gate: "Process check",
        visibility: "masked",
        glyph: "○",
        result: "pending",
        result_color: MUTED,
        detail: "evidence opens after review completes",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "verification ladder · gate 2/5", &status);
    let mut lines = vec![
        header_keys(),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("verification rungs")]),
        chrome::rule_line(105),
        rung_head(),
    ];
    lines.extend(RUNGS.iter().map(rung_row));
    lines.extend([
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("submission · feat-api-routes")]),
        chrome::rule_line(105),
        submission_row("worker", "ws-3f9a2c"),
        submission_row("diff", "6 files · +214 -37"),
        submission_row("branch", "workflow/wf-42/feat-api-routes"),
        submission_row("blocking", "acceptance, intent, and process"),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("why masked")]),
        chrome::rule_line(105),
        Line::from(vec![
            Span::raw("   "),
            m("Masked gates reveal their outcome, not private cases or reviewer prompts."),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("That keeps verification earned and keeps the next repair focused."),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("recent verification events")]),
        chrome::rule_line(105),
        event_head(),
        event("14:09", "reviewer", "opened intent questions", "3 answers still needed"),
        event("14:08", "validator", "withheld check completed", "one assertion needs repair"),
        event("14:07", "worker", "submitted feat-api-routes", "6 files · +214 -37"),
        event("14:06", "runner", "visible test suite passed", "48 of 48 checks"),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Line<'static> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("enter", "continue review"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("r", "re-run visible gates"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("j/k", "move"));
    Line::from(spans)
}

fn rung_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{:<LEVEL_W$}", "LEVEL")),
        m(&format!("{:<GATE_W$}", "GATE")),
        m(&format!("{:<VISIBILITY_W$}", "VISIBILITY")),
        m(&format!("{:<RESULT_W$}", "RESULT")),
        m("DETAIL"),
    ])
}

fn rung_row(rung: &Rung) -> Line<'static> {
    let result = format!("{} {}", rung.glyph, rung.result);
    Line::from(vec![
        Span::raw("   "),
        t(&format!("{:<LEVEL_W$}", rung.level)),
        t(&format!("{:<GATE_W$}", rung.gate)),
        c(&format!("{:<VISIBILITY_W$}", rung.visibility), INFO),
        c(&format!("{:<RESULT_W$}", result), rung.result_color),
        m(rung.detail),
    ])
}

fn submission_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{label:<12}")),
        t(value),
    ])
}

fn event_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("TIME    "),
        m("WHO        "),
        m("EVENT                         "),
        m("NOTE"),
    ])
}

fn event(time: &str, who: &str, action: &str, note: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{time:<8}")),
        t(&format!("{who:<11}")),
        t(&format!("{action:<30}")),
        m(note),
    ])
}
