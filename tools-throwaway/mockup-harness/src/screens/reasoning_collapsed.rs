//! Session screen - the same reasoning turn with the thinking trace
//! collapsed to exactly ONE line: `▸` disclosure glyph + a short summary +
//! the elapsed time + the key that expands it. No `│`-guttered trace lines
//! remain between the collapsed line and the answer; the full agent answer
//! follows below. Inline color convention per chat-turn: identifiers BRIGHT,
//! HTTP status codes INFO.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, INFO, MUTED};

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

    let mut collapsed: Vec<Span> = vec![
        Span::raw(" "),
        m(&format!(
            "{} Thinking {} traced HTTP 500 errors to a null token_version column {} {}",
            chrome::DISCLOSURE_CLOSED,
            chrome::SEP,
            chrome::SEP,
            chrome::timefmt(8.0)
        )),
        c(&format!(" {} ", chrome::SEP), FAINT),
    ];
    collapsed.extend(chrome::key_span("ctrl+o", "expand reasoning"));

    let lines: Vec<Line> = vec![
        chrome::user_cmd(
            "why is /login returning 500 for some users after the deploy?",
            Some("2m ago"),
        ),
        Line::from(""),
        Line::from(collapsed),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("The "),
            http("500"),
            t(" comes from stale session rows, not the login handler itself. The deploy added"),
        ]),
        para(vec![
            t("a non-null "),
            ident("token_version"),
            t(" column, but sessions created before the migration still hold null,"),
        ]),
        para(vec![
            t("so the token-refresh query fails for those users. Two fixes worth applying together:"),
        ]),
        Line::from(""),
        para(vec![
            m("1  "),
            c("Backfill", BRIGHT),
            t("   set "),
            ident("token_version"),
            t(" = 1 for every row where it is currently null"),
        ]),
        para(vec![
            m("2  "),
            c("Guard", BRIGHT),
            t("      treat a null "),
            ident("token_version"),
            t(" as version 0 instead of failing the refresh"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            chrome::dot(MUTED),
            c(" In short", BRIGHT),
            t("  backfill the null rows, then make the refresh path tolerate them."),
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
