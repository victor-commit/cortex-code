//! Full-screen `/review` findings with fixed severity and location columns.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, cb, m, ReviewFindingRow, ReviewSeverity, StatusLine, BAD, BRIGHT, FAINT, MUTED, WARN,
};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(
        f,
        "/review · 3 files · 5 findings · ↑↓ select · enter open file:line",
        &status,
    );

    let mut lines = vec![
        Line::from(vec![
            Span::raw("   "),
            m("reviewed the working tree against master"),
        ]),
        Line::from(""),
        columns(),
        chrome::rule_line(100),
    ];
    lines.extend(chrome::review_findings_list(
        body.width as usize,
        &[
            ReviewFindingRow {
                severity: ReviewSeverity::High,
                location: "buffer.rs:42",
                text: "unbounded growth when cap == 0",
            },
            ReviewFindingRow {
                severity: ReviewSeverity::Med,
                location: "mod.rs:18",
                text: "parse error dropped without context",
            },
            ReviewFindingRow {
                severity: ReviewSeverity::Med,
                location: "stream_processor.rs:39",
                text: "magic number 64*1024 - name it",
            },
            ReviewFindingRow {
                severity: ReviewSeverity::Low,
                location: "lib.rs:90",
                text: "redundant .clone() on &str",
            },
            ReviewFindingRow {
                severity: ReviewSeverity::Info,
                location: "tests/buffer.rs:1",
                text: "consider a proptest for the cap",
            },
        ],
        0,
    ));
    lines.extend([
        Line::from(""),
        chrome::rule_line(100),
        summary(),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn columns() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{:<10}", "severity")),
        m(&format!("{:<24}", "location")),
        m("  finding"),
    ])
}

fn summary() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("verdict  "),
        cb("changes requested", BRIGHT),
        m(" · 1 "),
        c("high", BAD),
        m(" · 2 "),
        c("med", WARN),
        m(" · 1 "),
        c("low", MUTED),
        m(" · 1 "),
        c("info", FAINT),
    ])
}
