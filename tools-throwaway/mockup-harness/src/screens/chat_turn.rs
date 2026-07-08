//! Session screen - one completed user->agent exchange. The answer follows
//! the inline color convention: code identifiers all share one styling
//! (BRIGHT), HTTP status codes share one distinct styling (INFO); every
//! occurrence in the body uses its category's styling.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, StatusLine, BRIGHT, INFO, MUTED};

/// Code identifier styling (single treatment for the whole answer).
fn ident(s: &str) -> Span<'static> {
    c(s, BRIGHT)
}

/// HTTP status-code styling (single, distinct treatment).
fn http(s: &str) -> Span<'static> {
    c(s, INFO)
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines: Vec<Line> = vec![
        chrome::user_cmd(
            "why are stale sessions rejected after a password change?",
            Some("5m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("Every session token embeds the "),
            ident("token_version"),
            t(" that was current when it was issued."),
        ]),
        para(vec![t(
            "Changing the password bumps the stored version, so the next request presents a stale token",
        )]),
        para(vec![t("that no longer matches.")]),
        Line::from(""),
        para(vec![
            t("The comparison happens in "),
            ident("enforce()"),
            t(", which runs before any handler. A version mismatch is"),
        ]),
        para(vec![
            t("rejected with "),
            http("401"),
            t(" before rate limiting is even consulted; genuine rate-limit rejections use"),
        ]),
        para(vec![
            http("429"),
            t(" instead, so clients can tell the two cases apart."),
        ]),
        Line::from(""),
        para(vec![
            t("Treat that rejection as a signal to re-authenticate rather than retry: "),
            ident("enforce()"),
            t(" keeps"),
        ]),
        para(vec![t(
            "rejecting the old token until a fresh sign-in issues a new one.",
        )]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            chrome::dot(MUTED),
            c(" In short", BRIGHT),
            t("  a password change bumps "),
            ident("token_version"),
            t("; every older token then gets "),
            http("401"),
            t("."),
        ]),
        Line::from(""),
    ];

    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 4, under the `agent_head` heading.
fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
