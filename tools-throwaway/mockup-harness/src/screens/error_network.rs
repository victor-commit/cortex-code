use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, FAINT, WARN};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "reply when ready…", &status);
    let backoff = format!(
        "{} → {} → {}",
        chrome::timefmt(2.0),
        chrome::timefmt(4.0),
        chrome::timefmt(8.0)
    );
    let mut recovery = chrome::key_span("r", "retry same request");
    recovery.push(c(&format!(" {} ", chrome::SEP), FAINT));
    recovery.extend(chrome::key_span("enter", "continue from saved draft"));

    let mut lines = vec![
        chrome::user_cmd("update the login retry path", None),
        Line::from(""),
        chrome::fail_head("Network unreachable"),
    ];
    lines.push(detail(fact("endpoint", "api.anthropic.com")));
    lines.push(detail(fact("model", "Claude Opus 4.8")));
    lines.push(detail(vec![
        m("attempts  "),
        c("3 of 3 · retries exhausted", WARN),
    ]));
    lines.push(detail(vec![m("backoff   "), t(&backoff)]));
    lines.push(detail(fact("reason", "Connection timed out after 8s")));
    lines.push(detail(vec![m("Your work is saved · nothing was lost")]));
    lines.push(detail(recovery));

    chrome::transcript(f, body, lines);
}

fn fact(label: &str, value: &str) -> Vec<Span<'static>> {
    vec![m(&format!("{label:<10}")), t(value)]
}

fn detail(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("     ")];
    row.append(&mut spans);
    Line::from(row)
}
