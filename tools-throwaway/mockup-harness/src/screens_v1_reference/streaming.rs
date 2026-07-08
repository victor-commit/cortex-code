//! Session screen — an answer mid-stream: the agent's reply is still being
//! written, so the prose stops mid-sentence with a caret and a full-width
//! Working band (a breathing petal spinner, what it's doing, elapsed seconds and
//! how to interrupt) sits just above the composer.

use ratatui::{
    style::Style,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, ACCENT, ORANGE, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("esc", "interrupt"),
            ("ctrl+o", "expand"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "summarize what changed in the auth refactor and why",
        "14:38:52",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(prose(vec![t(
        "The refactor pulls token handling out of the login handler and into a dedicated",
    )]));
    lines.push(prose(vec![
        c("TokenService", TITLE),
        t(", so every entry point \u{2014} login, refresh and logout \u{2014} now goes through one"),
    ]));
    lines.push(prose(vec![t(
        "place. That removes the duplicated expiry checks that had drifted apart and were the",
    )]));
    lines.push(prose(vec![
        t("root cause of the intermittent "),
        c("401s", ACCENT),
        t(". The new service also validates the signature before"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("it touches the store, so a forged token is rejected"),
        Span::styled("\u{258f}", Style::default().fg(ORANGE)),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::call_active("Working", 6));
    lines.push(chrome::call_branch(
        vec![chrome::m("Writing the summary")],
        true,
    ));
    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 3 so it aligns under the `agent_head` heading.
fn prose(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
