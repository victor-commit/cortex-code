//! Session screen - an expanded thinking trace between the user prompt and
//! the agent answer. The trace header carries the `▾` disclosure glyph, the
//! elapsed time and the hide hotkey; the trace body sits behind a FAINT `│`
//! gutter in readable MUTED (never FAINT), and a dim `▾ +N more lines`
//! affordance marks the truncation point so the trace never just stops.
//! Inline color convention per chat-turn: identifiers BRIGHT, HTTP status
//! codes INFO (the uniformly MUTED trace is exempt: it is secondary as a
//! whole).

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

/// One trace row: FAINT `│` gutter, MUTED thought text.
fn trace(s: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), c("│ ", FAINT), m(s)])
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut head: Vec<Span> = vec![
        Span::raw(" "),
        m(&format!(
            "{} Thinking {} {}",
            chrome::DISCLOSURE_OPEN,
            chrome::SEP,
            chrome::timefmt(8.0)
        )),
        c(&format!(" {} ", chrome::SEP), FAINT),
    ];
    head.extend(chrome::key_span("ctrl+o", "hide reasoning"));

    let lines: Vec<Line> = vec![
        chrome::user_cmd(
            "why is /login returning 500 for some users after the deploy?",
            Some("2m ago"),
        ),
        Line::from(""),
        Line::from(head),
        trace("HTTP 500 errors started after the deploy, so a migration or config change is the suspect."),
        trace("The new sessions table added a not-null token_version column with no default value."),
        trace("Existing rows were backfilled, but sessions created before the deploy still hold null."),
        trace("When a returning user hits /login, the token refresh reads token_version and fails."),
        trace("That matches the pattern: only returning users fail, brand-new sign-ins succeed."),
        Line::from(vec![
            Span::raw("   "),
            m(&format!("{} +6 more lines", chrome::DISCLOSURE_OPEN)),
        ]),
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
