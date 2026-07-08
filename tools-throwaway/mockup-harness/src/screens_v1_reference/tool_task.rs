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
            ("enter", "send"),
            ("ctrl+o", "expand"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "audit the auth module and add the rate-limit tests",
        "15:03:44",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_head(
        chrome::OK,
        "Ran",
        vec![
            chrome::c("subagent audit", chrome::TITLE),
            chrome::m(" \u{2014} auth module"),
        ],
    ));
    lines.push(chrome::call_branch(
        vec![
            chrome::c("6", chrome::ACCENT),
            chrome::m(" findings \u{00b7} 3m12s"),
        ],
        true,
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_active("Working", 64));
    lines.push(chrome::call_branch(
        vec![chrome::t("Running a subagent to add the rate-limit tests")],
        true,
    ));
    lines.push(chrome::call_branch(
        vec![
            chrome::c("2", chrome::ACCENT),
            chrome::m(" of "),
            chrome::c("3", chrome::ACCENT),
            chrome::m(" files written \u{00b7} tests/login_ratelimit.rs"),
        ],
        false,
    ));

    chrome::transcript(f, body, lines);
}
