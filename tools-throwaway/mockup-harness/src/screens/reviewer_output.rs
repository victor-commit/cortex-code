//! Reviewer verdict with all blocking findings gathered at the merge gate.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, BAD, BRIGHT, FAINT, OK};

const NAME_W: usize = 24;
const FINDING_W: usize = 48;

const L3_CHECKS: &[Check] = &[
    Check {
        question: "Q1",
        name: "Scope fidelity",
        passed: true,
        finding: "changes map to the stated request",
        reference: "review/scope.rs:34",
    },
    Check {
        question: "Q2",
        name: "Correctness",
        passed: true,
        finding: "rate limit holds on the sixth call",
        reference: "routes/login.rs:119",
    },
    Check {
        question: "Q3",
        name: "Dead weight",
        passed: false,
        finding: "remove the unused build_stub helper",
        reference: "server/util.rs:88",
    },
    Check {
        question: "Q4",
        name: "Swallowed failure",
        passed: false,
        finding: "return the error instead of dropping it",
        reference: "routes.rs:142",
    },
    Check {
        question: "Q5",
        name: "Honest report",
        passed: true,
        finding: "summary matches the submitted diff",
        reference: "review/report.rs:27",
    },
];

const L4_CHECKS: &[Check] = &[
    Check {
        question: "Q6",
        name: "Tests executed",
        passed: true,
        finding: "42 workspace tests completed",
        reference: "tests/api.rs:44",
    },
    Check {
        question: "Q7",
        name: "Surface exercised",
        passed: true,
        finding: "login rate limit was exercised",
        reference: "tests/login.rs:67",
    },
    Check {
        question: "Q8",
        name: "Evidence captured",
        passed: false,
        finding: "attach the missing audit run log",
        reference: "artifacts/run-log.txt:1",
    },
    Check {
        question: "Q9",
        name: "Regression re-run",
        passed: false,
        finding: "run tests again after the fix",
        reference: "tests/regression.rs:91",
    },
];

struct Check {
    question: &'static str,
    name: &'static str,
    passed: bool,
    finding: &'static str,
    reference: &'static str,
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "reviewer output · changes requested", &status);
    let mut lines = vec![
        Line::from(vec![
            Span::raw("   "),
            m("diff  "),
            t("6 files · +214 -37"),
            m("  ·  worker  "),
            t("ws-3f9a2c"),
            m("  ·  branch  "),
            t("workflow/wf-42/feat-api-routes"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("✗ CHANGES REQUESTED", BAD),
            m("  ·  four findings block the merge"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("L3 Code Reviewer", BRIGHT),
            m("  ·  intent audit  ·  Q1-Q5"),
        ]),
        chrome::rule_line(105),
        check_head(),
    ];
    lines.extend(L3_CHECKS.iter().map(check_row));
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("L4 QA Validator", BRIGHT),
            m("  ·  process check  ·  Q6-Q9"),
        ]),
        chrome::rule_line(105),
        check_head(),
    ]);
    lines.extend(L4_CHECKS.iter().map(check_row));
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("✗ merge withheld", BAD),
            m("  ·  4 findings  ·  worker changes require re-review"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("re-post to "),
            t("ws-3f9a2c"),
            m(" with: "),
            c("Q3 Q4 Q8 Q9", BAD),
            m("  ·  fix, then submit the same gate"),
        ]),
        gate_keys(),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn check_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        c("RESULT  ", FAINT),
        c("QUESTION  ", FAINT),
        c(&format!("{:<NAME_W$}", "CHECK"), FAINT),
        c(&format!("{:<FINDING_W$}", "FINDING"), FAINT),
        c("FILE:LINE", FAINT),
    ])
}

fn check_row(check: &Check) -> Line<'static> {
    let (glyph, word, hue) = if check.passed {
        ("✓", "pass", OK)
    } else {
        ("✗", "fail", BAD)
    };
    Line::from(vec![
        Span::raw("   "),
        if check.passed {
            Span::raw("  ")
        } else {
            c("│ ", BAD)
        },
        c(&format!("{glyph} {word:<4} "), hue),
        m(&format!("{:<10}", check.question)),
        t(&format!("{:<NAME_W$}", check.name)),
        t(&format!("{:<FINDING_W$}", check.finding)),
        m(check.reference),
    ])
}

fn gate_keys() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("r ", ACCENT),
        c("re-post findings", BRIGHT),
        c("  ·  ", FAINT),
        c("a accept", FAINT),
    ])
}
