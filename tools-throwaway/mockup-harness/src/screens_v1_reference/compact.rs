//! Session screen — the result of `/compact`: Cortex folds the older turns into
//! a short retained summary so the context window frees up. The kept gist hangs
//! inline beneath a faint gutter — no card, no box — and a terse stat line
//! reports the reclaimed budget, the way shipped coding CLIs render it.

use ratatui::{
    style::Style,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, BRIGHT, FAINT, OK, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "pick up where you left off\u{2026}",
        &[
            ("enter", "send"),
            ("/", "commands"),
            ("ctrl+o", "expand transcript"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/compact", "15:18:03"));
    lines.push(Line::from(""));

    lines.push(chrome::call_expanded(OK, "Conversation compacted"));
    lines.push(gut(vec![
        m("\u{2192} "),
        t("Traced the /login 500s to null "),
        c("token_version", TITLE),
        t(" on pre-deploy sessions."),
    ]));
    lines.push(gut(vec![
        m("\u{2192} "),
        t("Applied a backfill and defaulted new sessions to "),
        c("Some(1)", TITLE),
        t("."),
    ]));
    lines.push(gut(vec![
        m("\u{2192} "),
        t("Added "),
        c("tests/login_regression.rs", TITLE),
        t(" to pin the fix."),
    ]));
    lines.push(gut(vec![
        m("\u{2192} "),
        t("Open: run "),
        c("cargo fmt", TITLE),
        t(" and push the branch for review."),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("     "),
        c("18", BRIGHT),
        m(" turns collapsed   \u{00b7}   "),
        c("31,400", BRIGHT),
        m(" tokens reclaimed   \u{00b7}   context now "),
        c("12%", OK),
        m(" of window"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("     "),
        m("The full transcript is still saved to the session log \u{2014} nothing was lost."),
    ]));

    chrome::transcript(f, body, lines);
}

fn gut(spans: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![
        Span::raw("     "),
        Span::styled("\u{2502} ", Style::default().fg(FAINT)),
    ];
    out.extend(spans);
    Line::from(out)
}
