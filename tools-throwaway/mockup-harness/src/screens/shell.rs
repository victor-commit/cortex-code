//! Session screen — the agent runs shell commands inline the way Codex does: a
//! `Ran`/`Running` header carrying the command, its stdout hanging off `└` output
//! branches, and a terse `✓ exit · timing` result line. A completed run sits above
//! a second command still streaming, capped by a live `Working` status.

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
            ("ctrl+o", "collapse"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "check the branch, then run the tests",
        "15:02:44",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_head(
        chrome::OK,
        "Ran",
        vec![chrome::c("git status --short --branch", chrome::TITLE)],
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("## main...origin/main [ahead 1]")],
        true,
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t(" M crates/server/src/models/session.rs")],
        false,
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("?? crates/server/tests/login_regression.rs")],
        false,
    ));
    lines.push(chrome::call_result(true, None, "0.2s"));
    lines.push(Line::from(""));

    lines.push(chrome::call_head(
        chrome::WARN,
        "Running",
        vec![chrome::c("cargo test --workspace", chrome::TITLE)],
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("Compiling cortex-server v0.1.0")],
        true,
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("Finished test profile in 3.1s")],
        false,
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("test models::session::token_version_defaults ... ok")],
        false,
    ));
    lines.push(chrome::call_branch(
        vec![chrome::t("test routes::login::rate_limited ... ok")],
        false,
    ));
    lines.push(chrome::call_branch(
        vec![
            chrome::t("test routes::api::quota_enforced ... "),
            chrome::c("\u{2588}", chrome::ACCENT),
        ],
        false,
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_active("Working", 4));
    chrome::transcript(f, body, lines);
}
