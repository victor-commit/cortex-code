//! Session screen — a tool call that failed: `run_shell` exited non-zero, so the
//! card shows the call, a red status dot and the error in plain English (never a
//! raw error code alone). The agent then explains the failure and offers a fix,
//! so the user is never left staring at a stack trace.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, ACCENT, BAD, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("r", "retry"),
            ("ctrl+o", "expand"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("run the test suite", "14:41:55"));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("   "),
        c(chrome::state_glyph(BAD), BAD),
        t(" Ran the workspace test suite"),
        c("   \u{203a} ", ACCENT),
        m("exit 1"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("The suite never ran \u{2014} the build failed first. "),
        c("api.rs", TITLE),
        t(" uses "),
        c("serde_json", TITLE),
        t(","),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("but that crate is not listed in "),
        c("crates/server/Cargo.toml", TITLE),
        t(". I can add the"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("dependency and re-run the suite \u{2014} say the word and I'll apply the fix."),
    ]));

    chrome::transcript(f, body, lines);
}
