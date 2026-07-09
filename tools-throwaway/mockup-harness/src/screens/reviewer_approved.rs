//! Reviewer verdict after every L3 and L4 check passes.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, OK};

const NAME_W: usize = 24;
const FINDING_W: usize = 48;

const L3_CHECKS: &[Check] = &[
    Check {
        question: "Q1",
        name: "Scope fidelity",
        finding: "changes map to the stated request",
        reference: "review/scope.rs:34",
    },
    Check {
        question: "Q2",
        name: "Correctness",
        finding: "rate limit holds on the sixth call",
        reference: "routes/login.rs:119",
    },
    Check {
        question: "Q3",
        name: "Dead weight",
        finding: "no unused helpers remain",
        reference: "server/util.rs:88",
    },
    Check {
        question: "Q4",
        name: "Failure path",
        finding: "login errors reach the caller",
        reference: "routes.rs:142",
    },
    Check {
        question: "Q5",
        name: "Honest report",
        finding: "summary matches the submitted diff",
        reference: "review/report.rs:27",
    },
];

const L4_CHECKS: &[Check] = &[
    Check {
        question: "Q6",
        name: "Tests executed",
        finding: "42 workspace tests completed",
        reference: "tests/api.rs:44",
    },
    Check {
        question: "Q7",
        name: "Surface exercised",
        finding: "login rate limit was exercised",
        reference: "tests/login.rs:67",
    },
    Check {
        question: "Q8",
        name: "Evidence captured",
        finding: "audit run log is attached",
        reference: "artifacts/run-log.txt:1",
    },
    Check {
        question: "Q9",
        name: "Regression re-run",
        finding: "post-fix suite passed",
        reference: "tests/regression.rs:91",
    },
];

struct Check {
    question: &'static str,
    name: &'static str,
    finding: &'static str,
    reference: &'static str,
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "reviewer output · approved", &status);
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
            c("✓ APPROVED", OK),
            m("  ·  all reviewer and validator checks passed"),
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
            c("✓ merge proceeds", OK),
            m("  ·  accept the reviewed diff to continue to the next gate"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("a ", BRIGHT),
            c("accept merge", BRIGHT),
            c("  ·  ", FAINT),
            m("enter inspect the submitted diff"),
        ]),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn check_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("RESULT  ", FAINT),
        c("QUESTION  ", FAINT),
        c(&format!("{:<NAME_W$}", "CHECK"), FAINT),
        c(&format!("{:<FINDING_W$}", "EVIDENCE"), FAINT),
        c("FILE:LINE", FAINT),
    ])
}

fn check_row(check: &Check) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("✓ pass ", OK),
        m(&format!("{:<10}", check.question)),
        t(&format!("{:<NAME_W$}", check.name)),
        t(&format!("{:<FINDING_W$}", check.finding)),
        m(check.reference),
    ])
}
