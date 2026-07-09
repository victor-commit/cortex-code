//! Session screen for a failed model turn. The visible partial answer remains
//! above an inline failure block, so retrying has a clear local meaning.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, m, t, StatusLine};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply when ready\u{2026}",
        &status,
    );

    let mut lines = vec![
        chrome::user_cmd("explain how the login rate limit behaves under load", Some("2m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![t(
            "The limiter allows a short burst, then refills each account bucket at a steady rate.",
        )]),
        para(vec![t(
            "Once the bucket is empty, later requests wait until capacity returns, so\u{2026}",
        )]),
        Line::from(""),
        chrome::fail_head("Reply interrupted"),
    ];
    lines.extend(chrome::fail_rows(vec![
        vec![t(
            "The provider closed the connection before the reply finished.",
        )],
        vec![m("Retry sends the same request again.")],
        chrome::key_span("r", "retry"),
    ]));

    chrome::transcript(f, body, lines);
}

fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
