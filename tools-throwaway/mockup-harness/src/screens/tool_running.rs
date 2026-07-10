//! Session screen - one in-flight tool call (pattern 1 + 5): a MUTED `●`
//! Search line with NO elapsed of its own, a `⎿` elbow owning every progress
//! figure exactly once (scan fraction + matches so far), and the single
//! working band above the composer owning the elapsed time and the
//! interrupt hint. One activity, one time source, zero repeated metrics.

use ratatui::{
    Frame,
    text::{Line, Span},
};

use crate::chrome::{self, LiveActivityInterrupt, StatusLine, ToolStatus, m, t};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "type a follow-up, it runs after this turn\u{2026}",
        &status,
    );

    let progress = vec![m(&format!(
        "128 of 400 files {} 3 matches so far",
        chrome::SEP
    ))];

    let mut lines: Vec<Line> = vec![
        chrome::user_cmd("where do we enforce the rate limit?", Some("1m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Searching the server crate for every place the limit is enforced."),
        ]),
        Line::from(""),
        chrome::tool_line(
            ToolStatus::Running(None, LiveActivityInterrupt::Band),
            "Search",
            "pattern: \"rate_limit\", path: crates/server/src",
        ),
    ];
    lines.extend(chrome::elbow(vec![progress]));
    lines.push(Line::from(""));
    lines.push(chrome::working_band("Scouring", 4.0, None));

    chrome::transcript(f, body, lines);
}
