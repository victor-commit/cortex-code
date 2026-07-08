//! Session screen — a tool call in flight, laid out the way Codex renders it: an
//! `Exploring` header with the search shown on a `└` branch, then a live `Working`
//! status line (elapsed + interrupt) with its own branch of running detail.

use ratatui::{
    text::Line,
    Frame,
};

use crate::chrome;

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
        "where do we enforce the rate limit?",
        "14:24:02",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_head(chrome::WARN, "Exploring", vec![]));
    lines.push(chrome::call_branch(
        vec![
            chrome::t("Search "),
            chrome::c("\"rate limit\"", chrome::TITLE),
            chrome::m(" in "),
            chrome::m("crates/"),
        ],
        true,
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_active("Working", 4));
    lines.push(chrome::call_branch(
        vec![chrome::m(
            "Searching crates/ \u{00b7} 128 files scanned \u{00b7} 3 matches so far",
        )],
        true,
    ));

    chrome::transcript(f, body, lines);
}
