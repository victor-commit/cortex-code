//! Session screen — a tool call that finished cleanly: `grep_code` returned, so
//! the card shows the call, an OK status dot and a concise summary of what came
//! back. The green dot (not a check glyph) carries the success state; the agent
//! then reads the result back in plain prose above the composer.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, cb, m, t, BRIGHT, OK, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("ctrl+o", "collapse"),
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

    lines.push(Line::from(vec![
        Span::raw("   "),
        c(chrome::state_glyph(OK), OK),
        cb(
            " Searched the codebase for where the rate limit is enforced",
            BRIGHT,
        ),
    ]));
    lines.push(chrome::call_detail(vec![
        m("pattern "),
        c("\"rate_limit\"", TITLE),
        m(" in "),
        t("crates/server/src"),
    ]));
    lines.push(chrome::call_detail(vec![
        c("3", BRIGHT),
        m(" matches across "),
        c("221", BRIGHT),
        m(" files \u{00b7} scanned in 0.4s"),
    ]));
    lines.push(chrome::call_detail(vec![
        c("middleware/rate_limit.rs", TITLE),
        m(":42   "),
        t("pub fn enforce(limit: Quota)"),
    ]));
    lines.push(chrome::call_detail(vec![
        c("routes/login.rs", TITLE),
        m(":88   "),
        t("enforce(login_quota())"),
    ]));
    lines.push(chrome::call_detail(vec![
        c("routes/api.rs", TITLE),
        m(":17   "),
        t("enforce(api_quota())"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("The limit is enforced in one place \u{2014} "),
        c("middleware/rate_limit.rs", TITLE),
        t(", and the login and"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("API routes both call "),
        c("enforce()", TITLE),
        t(" with their own quota. Want me to walk through it?"),
    ]));

    chrome::transcript(f, body, lines);
}
