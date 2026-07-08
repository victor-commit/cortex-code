//! Session screen — the landing view after the first message: the brain mascot
//! and the MOTD (session facts + a tip line) stay pinned at the top of the
//! scrollback while the conversation flows beneath them, so the welcome header
//! remains in history instead of clearing the moment the user starts typing.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, ACCENT, BRIGHT, TITLE, TXT};

const MASCOT: [&str; 4] = [
    " \u{2584}\u{2588}\u{2580}\u{2580}\u{2580}\u{2580}\u{2588}\u{2584}  ",
    "\u{2588}\u{2588} \u{258c}  \u{2590} \u{2588}\u{2588} ",
    " \u{2588}\u{2584}\u{2584}\u{2584}\u{2584}\u{2584}\u{2584}\u{2588}  ",
    "  \u{2588}    \u{2588}   ",
];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Home",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("/", "commands"),
            ("\u{21e5}", "autocomplete"),
            ("@", "files"),
            ("?", "help"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::raw("   "),
        c(MASCOT[0], ACCENT),
        Span::raw("  "),
        Span::styled(
            "Cortex Code",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        m("   v0.1.0"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        c(MASCOT[1], ACCENT),
        Span::raw("  "),
        Span::styled(
            "Welcome back, root",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        c(MASCOT[2], ACCENT),
        Span::raw("  "),
        m("Your agentic coding terminal."),
    ]));
    lines.push(Line::from(vec![Span::raw("   "), c(MASCOT[3], ACCENT)]));
    lines.push(Line::from(""));

    lines.push(chrome::info("directory", "~/projects/cortex-code", TXT));
    lines.push(Line::from(vec![
        m("  model: "),
        c("claude-opus", BRIGHT),
        m("     mode: "),
        c("Agent", TITLE),
        m("     branch: "),
        t("main"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::user_cmd(
        "add a /healthz endpoint that returns build info",
        "14:24:10",
    ));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("On it. I'll add a "),
        c("/healthz", TITLE),
        t(" handler returning the commit, version and uptime as"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("JSON, then wire it into the router. Reviewing "),
        c("crates/server/src", TITLE),
        t(" first."),
    ]));

    chrome::transcript(f, body, lines);
}
