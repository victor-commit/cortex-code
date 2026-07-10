use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, WARN};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "reply from where it stopped…", &status);
    let elapsed = chrome::timefmt(12.0);

    let lines = vec![
        chrome::user_cmd("explain how the rate limiter works", Some("2m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![t(
            "The limiter tracks each caller in a one-minute window and counts requests as they arrive.",
        )]),
        para(vec![t(
            "When a caller crosses the cap, the middleware returns 429 with a Retry-After header.",
        )]),
        para(vec![
            t("The counter resets when the window rolls over, so the next request can "),
            c("█", BRIGHT),
        ]),
        Line::from(""),
        chrome::rule_line(102),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("● ", WARN),
            t("Response interrupted"),
            m(" · "),
            t(&format!("you stopped the reply {elapsed} in")),
        ]),
        para(vec![m("Partial answer kept · saved in this session")]),
        Line::from(""),
        chrome::agent_head("What you can do"),
        recovery("enter", "continue", "finish the reply from where it stopped"),
        recovery("r", "rephrase", "ask the question a different way"),
    ];

    chrome::transcript(f, body, lines);
}

fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}

fn recovery(key: &str, label: &str, detail: &str) -> Line<'static> {
    let mut spans = vec![Span::raw("     ")];
    spans.extend(chrome::key_span(key, label));
    spans.push(c(&format!(" · {detail}"), FAINT));
    Line::from(spans)
}
