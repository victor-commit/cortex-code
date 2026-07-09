//! Session screen for `/compact`. The retained recap separates prose from
//! reclaimed-context metrics and keeps the full transcript's expand target
//! explicit beside the summary.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, MUTED, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "resume from the compacted summary\u{2026}",
        &status,
    );

    let mut lines = vec![
        chrome::user_cmd("/compact", Some("1m ago")),
        Line::from(""),
        Line::from(vec![
            Span::raw(" "),
            chrome::dot(OK),
            t(" Conversation compacted"),
        ]),
        Line::from(""),
    ];
    lines.extend(recap("Traced the /login 500s to a missing token version on older sessions."));
    lines.extend(recap("Backfilled affected rows and defaulted new sessions to version 1."));
    lines.extend(recap("Added a regression test for the refreshed login path."));
    lines.extend(recap("Next: run cargo fmt, then open the change for review."));
    lines.push(Line::from(""));
    lines.push(chrome::rule_line(100));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("18 turns collapsed"),
        m(" · "),
        t("31,400 tokens reclaimed"),
        m(" · context now "),
        t("12%"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        m("The full transcript is preserved in this session."),
    ]));
    lines.push(Line::from(""));
    lines.push(chrome::hints(&[("ctrl+o", "expand full transcript")]));

    chrome::transcript(f, body, lines);
}

fn recap(text: &str) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::raw("   "),
        c("→ ", MUTED),
        t(text),
    ])]
}
