//! Running workflow monitor. The feature table is the single progress source.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, BAD, BRIGHT, FAINT, MUTED, OK};

const RUNNING_FEATURES: &[Feature] = &[
    Feature {
        id: "feat-api-routes",
        milestone: "api",
        verdict: Verdict::Partial,
        checks: "2/4",
        artifact: "rate-limit middleware",
        fulfills: "ACC-API-010",
    },
    Feature {
        id: "feat-web-ui",
        milestone: "api",
        verdict: Verdict::Pending,
        checks: "0/3",
        artifact: "login form and guard",
        fulfills: "ACC-WEB-003",
    },
    Feature {
        id: "feat-audit-log",
        milestone: "api",
        verdict: Verdict::Pending,
        checks: "0/2",
        artifact: "auth events audit sink",
        fulfills: "ACC-AUD-004",
    },
];

const FAILURE_FEATURES: &[Feature] = &[
    Feature {
        id: "feat-audit-log",
        milestone: "api",
        verdict: Verdict::Failed,
        checks: "0/2",
        artifact: "auth events audit sink",
        fulfills: "ACC-AUD-004",
    },
    Feature {
        id: "feat-api-routes",
        milestone: "api",
        verdict: Verdict::Partial,
        checks: "2/4",
        artifact: "rate-limit middleware",
        fulfills: "ACC-API-010",
    },
    Feature {
        id: "feat-web-ui",
        milestone: "api",
        verdict: Verdict::Pending,
        checks: "0/3",
        artifact: "login form and guard",
        fulfills: "ACC-WEB-003",
    },
];

struct Feature {
    id: &'static str,
    milestone: &'static str,
    verdict: Verdict,
    checks: &'static str,
    artifact: &'static str,
    fulfills: &'static str,
}

enum Verdict {
    Partial,
    Pending,
    Failed,
}

pub fn render(f: &mut Frame) {
    render_variant(f, MonitorVariant::Running);
}

pub(crate) fn render_failure(f: &mut Frame) {
    render_variant(f, MonitorVariant::Failure);
}

enum MonitorVariant {
    Running,
    Failure,
}

fn render_variant(f: &mut Frame, variant: MonitorVariant) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "workflow monitor · wf-42", &status);
    let (features, failed) = match variant {
        MonitorVariant::Running => (RUNNING_FEATURES, false),
        MonitorVariant::Failure => (FAILURE_FEATURES, true),
    };
    let feature_caption = if failed {
        "feature progress · failed feature pinned first"
    } else {
        "feature progress · filter shows unverified by default"
    };
    let mut lines = vec![
        Line::from(header_keys()),
        Line::from(""),
        workflow_summary(failed),
        phase_ribbon(),
        Line::from(vec![
            Span::raw("   "),
            m("runner drives features.json while the workflow is running"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m(feature_caption)]),
        chrome::rule_line(105),
        progress_head(),
    ];
    lines.extend(features.iter().map(progress_row));
    lines.extend([
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("runtime loop · D1-D8")]),
        chrome::rule_line(105),
    ]);
    lines.extend(loop_steps());
    lines.extend([
        Line::from(vec![
            Span::raw("   "),
            m("worker  "),
            t("ws-3f9a2c"),
            m("  ·  branch  "),
            t("workflow/wf-42/feat-api-routes"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m(&chrome::inline_live_state("Orchestrating", 252.0)),
            m(" · 3 workers"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("wave scheduling · parallel metrics")]),
        chrome::rule_line(105),
        Line::from(vec![
            Span::raw("   "),
            m("wave  "),
            t("2 of 3"),
            m("  ·  pairs  "),
            t("3"),
            m("  ·  worktrees  "),
            t("3"),
            m("  ·  dispatch  "),
            t("3/3"),
        ]),
        conflict_rate(),
        Line::from(vec![
            Span::raw("   "),
            m("end gate  "),
            c("○ ", MUTED),
            t("2 of 4 passed"),
            m("  ·  README present"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("recent workflow events")]),
        chrome::rule_line(105),
        event_head(),
        event("14:07", "runner", "checkpoint recorded", "worker result received"),
        event("14:06", "ws-3f9a2c", "D4 work", "routes.rs and handlers.rs"),
        event("14:05", "runner", "D2 worktree", "wave 2 dispatched"),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("f", "filter"));
    spans.push(c(" · ", FAINT));
    spans.push(m("unverified by default"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("enter", "inspect feature"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("j/k", "move"));
    spans
}

fn workflow_summary(failed: bool) -> Line<'static> {
    let mut spans = vec![
        Span::raw("   "),
        m("workflow wf-42  ·  "),
        c("verified 4/7", OK),
        m("  ·  "),
    ];
    if failed {
        spans.push(c("failed 1", BAD));
        spans.push(m("  ·  feat-audit-log"));
    } else {
        spans.push(m("current phase D"));
    }
    Line::from(spans)
}

fn phase_ribbon() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("● ", OK),
        m("A plan"),
        c("  →  ", FAINT),
        c("● ", OK),
        m("B design"),
        c("  →  ", FAINT),
        c("● ", OK),
        m("C build"),
        c("  →  ", FAINT),
        c("› ", ACCENT),
        c("D runtime", BRIGHT),
        c("  →  ", FAINT),
        c("○ ", MUTED),
        m("E validate"),
        c("  →  ", FAINT),
        c("○ ", MUTED),
        m("F gate"),
    ])
}

fn progress_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        m("FEATURE            "),
        m("MILESTONE  "),
        m("VERDICT     "),
        m("CHECKS  "),
        m("ARTIFACT                  "),
        Span::raw("  "),
        m("FULFILLS"),
    ])
}

fn progress_row(feature: &Feature) -> Line<'static> {
    let (glyph, hue, word) = match feature.verdict {
        Verdict::Partial => ("●", MUTED, "partial"),
        Verdict::Pending => ("○", MUTED, "pending"),
        Verdict::Failed => ("✗", BAD, "failed"),
    };
    let marker = if matches!(feature.verdict, Verdict::Failed) {
        c("│ ", BAD)
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![
        Span::raw("   "),
        marker,
        t(&format!("{:<19}", feature.id)),
        m(&format!("{:<11}", feature.milestone)),
        c(&format!("{glyph} "), hue),
        t(&format!("{word:<10}")),
        m(&format!("{:<8}", feature.checks)),
        t(&format!("{:<26}", feature.artifact)),
        Span::raw("  "),
        m(feature.fulfills),
    ];
    if matches!(feature.verdict, Verdict::Failed) {
        spans.push(c("  ·  ", FAINT));
        spans.extend(chrome::key_span("r", "retry feature"));
    }
    Line::from(spans)
}

fn loop_steps() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::raw("   "),
            c("● ", OK),
            m("D1 spawn"),
            c("  ·  ", FAINT),
            c("● ", OK),
            m("D2 worktree"),
            c("  ·  ", FAINT),
            c("● ", OK),
            m("D3 gate"),
            c("  ·  ", FAINT),
            c("› ", ACCENT),
            c("D4 work", BRIGHT),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("○ ", MUTED),
            m("D5 check"),
            c("  ·  ", FAINT),
            c("○ ", MUTED),
            m("D6 end feature"),
            c("  ·  ", FAINT),
            c("○ ", MUTED),
            m("D7 reduce"),
            c("  ·  ", FAINT),
            c("○ ", MUTED),
            m("D8 merge-back"),
        ]),
    ]
}

fn conflict_rate() -> Line<'static> {
    let mut spans = vec![Span::raw("   "), m("conflicts  ")];
    spans.extend(chrome::bar(0, 100, 22));
    spans.extend([
        Span::raw("  "),
        m("0.0%  ·  below 15% threshold"),
    ]);
    Line::from(spans)
}

fn event_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("TIME    "),
        m("WHO          "),
        m("EVENT                 "),
        m("NOTE"),
    ])
}

fn event(time: &str, who: &str, action: &str, note: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{time:<8}")),
        t(&format!("{who:<13}")),
        t(&format!("{action:<22}")),
        m(note),
    ])
}
