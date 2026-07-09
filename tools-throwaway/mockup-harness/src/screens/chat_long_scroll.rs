//! Session screen - a long multi-turn history. Three user->agent exchanges
//! fill the transcript with the newest turn hugging the composer; a single
//! MUTED scroll affordance at the top says how many earlier turns are hidden
//! (the history never just cuts off), and one MUTED system-notice line (the
//! `notice` entry kind) sits between exchanges. Inline color convention as
//! established by chat-turn: code identifiers BRIGHT, HTTP status codes INFO.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, INFO, MUTED};

/// Code identifier styling (single treatment for the whole transcript).
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

    let mut scroll_hint: Vec<Span> = vec![
        Span::raw(" "),
        m("↑ 12 earlier turns"),
        c(&format!(" {} ", chrome::SEP), FAINT),
    ];
    scroll_hint.extend(chrome::key_span("ctrl+u", "to scroll"));

    let lines: Vec<Line> = vec![
        Line::from(scroll_hint),
        Line::from(""),
        chrome::user_cmd("where do refresh tokens get persisted?", Some("26m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("They live in the sessions table, one row per device. "),
            ident("TokenService"),
            t(" writes them through"),
        ]),
        para(vec![
            ident("store::persist()"),
            t(" at issue time and rotates the row in place on every refresh, so a device"),
        ]),
        para(vec![t("never holds two live tokens at once.")]),
        Line::from(""),
        chrome::user_cmd(
            "add an integration test that proves rotation invalidates the old token",
            Some("18m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("Added "),
            ident("rotation_invalidates_previous_token"),
            t(" to crates/server/tests/auth.rs. It signs in,"),
        ]),
        para(vec![
            t("refreshes once, then replays the pre-refresh token and asserts the reply is "),
            http("401"),
            t(" with"),
        ]),
        para(vec![
            t("a token-rotated reason in the body. The suite passes locally in 41s."),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw(" "),
            m(&format!("{} Exported session (Markdown) to ~/exports/auth-refactor.md", chrome::DOT)),
        ]),
        Line::from(""),
        chrome::user_cmd(
            "why keep the old token around for five minutes after rotation?",
            Some("9m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("The grace window absorbs racing requests: a client that fired two calls in parallel can"),
        ]),
        para(vec![
            t("still present the outgoing token a moment after the refresh landed. Within that window"),
        ]),
        para(vec![
            ident("enforce()"),
            t(" accepts the old token for reads but never for writes, which keeps the race"),
        ]),
        para(vec![t("harmless without weakening rotation.")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            chrome::dot(MUTED),
            c(" In short", BRIGHT),
            t("  rotation is immediate for writes; reads get a five-minute grace window."),
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
