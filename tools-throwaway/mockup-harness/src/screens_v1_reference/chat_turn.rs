//! Session screen — one completed exchange: the user asks how the rate limiter
//! rejects requests, and the agent's finished answer is shown in full (a short
//! lead, a couple of explanatory paragraphs, and a compact takeaway). The turn
//! is done — no spinner — and the composer waits for the follow-up.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, ACCENT, BRIGHT, OK, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("\u{2191}", "edit last"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "how does the rate limiter decide when to reject a request?",
        "14:23:41",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));

    para(&mut lines, vec![
        t("The limiter keeps one "),
        c("token bucket", TITLE),
        t(" per client key. Each bucket holds up to "),
        c("60", BRIGHT),
        t(" tokens and refills"),
    ]);
    para(&mut lines, vec![
        t("at "),
        c("1 token/second", BRIGHT),
        t(", so a caller can burst briefly but settles to ~1 request per second over time."),
    ]);
    lines.push(Line::from(""));

    para(&mut lines, vec![
        t("On every request the middleware removes one token. If the bucket still has a token, the call is"),
    ]);
    para(&mut lines, vec![
        t("admitted; if it is empty the request is rejected with "),
        c("429 Too Many Requests", ACCENT),
        t(" and a "),
        c("Retry-After", TITLE),
    ]);
    para(&mut lines, vec![
        t("header telling the client how many seconds until the next token is available."),
    ]);
    lines.push(Line::from(""));

    para(&mut lines, vec![
        t("The client key is the authenticated user id when present, and falls back to the source IP for"),
    ]);
    para(&mut lines, vec![
        t("anonymous traffic, so one noisy caller cannot spend everyone else's budget."),
    ]);
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("   "),
        chrome::dot(OK),
        c(" In short", OK),
        t("  60-request burst, 1 request/second sustained, per user or per IP."),
    ]));

    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 3 so it aligns under the `agent_head` heading.
fn para(lines: &mut Vec<Line<'static>>, mut spans: Vec<Span<'static>>) {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    lines.push(Line::from(row));
}
