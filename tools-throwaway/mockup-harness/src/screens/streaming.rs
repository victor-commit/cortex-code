//! Session screen - a reply streaming in. The paragraph stops mid-thought
//! with the single `█` stream cursor, and exactly ONE canonical working band
//! sits above the composer (ACCENT spinner + one verb + dim counters, per
//! patterns 5/10). The band is the only line naming the elapsed time and the
//! interrupt key - no second inline "streaming" status anywhere.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, StatusLine, BRIGHT, INFO};

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
        "type a follow-up, it runs after this turn\u{2026}",
        &status,
    );

    let lines: Vec<Line> = vec![
        chrome::user_cmd(
            "summarize what changed in the auth refactor and why",
            Some("1m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("The refactor pulls token handling out of the login handler and into a dedicated"),
        ]),
        para(vec![
            ident("TokenService"),
            t(", so login, refresh and logout now go through one place. That removes the"),
        ]),
        para(vec![
            t("duplicated expiry checks that had drifted apart and were the root cause of the"),
        ]),
        para(vec![t("intermittent HTTP "), http("401"), t(" errors.")]),
        Line::from(""),
        para(vec![
            t("The new service also validates the token signature before it touches the session"),
        ]),
        para(vec![
            t("store, so a forged token is rejected before"),
            c("█", BRIGHT),
        ]),
        Line::from(""),
        chrome::working_band("Composing", 24.0, Some("2.4k tokens")),
    ];

    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 4, under the `agent_head` heading.
fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
