use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, cb, m, t, ACCENT, BRIGHT, GHOST, OK};

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
        "implement rate limiting on the login endpoint",
        "15:07:12",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::call_head(chrome::ACCENT, "Updated plan", vec![]));
    lines.push(todo(
        "\u{2713}",
        OK,
        vec![m("Read the auth middleware and login handler")],
    ));
    lines.push(todo(
        "\u{2713}",
        OK,
        vec![
            m("Add a token-bucket limiter in "),
            m("src/mw/ratelimit.rs"),
        ],
    ));
    lines.push(todo(
        "\u{2713}",
        OK,
        vec![m("Wire the limiter into the POST /login route")],
    ));
    lines.push(todo(
        "\u{25b6}",
        ACCENT,
        vec![
            cb("Add ", BRIGHT),
            cb("tests/login_ratelimit.rs", BRIGHT),
            cb(" \u{2014} 429 after 5 attempts", BRIGHT),
        ],
    ));
    lines.push(todo(
        "\u{25cb}",
        GHOST,
        vec![t("Run "), t("cargo test"), t(" and "), t("cargo fmt")],
    ));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("     "),
        c("3", BRIGHT),
        m(" of "),
        c("5", BRIGHT),
        m(" done   \u{00b7}   "),
        c("1", ACCENT),
        m(" in progress   \u{00b7}   "),
        c("1", GHOST),
        m(" pending"),
    ]));

    lines.push(Line::from(""));
    lines.push(chrome::call_active("Working", 22));
    lines.push(chrome::call_branch(
        vec![
            chrome::m("Writing "),
            chrome::m("tests/login_ratelimit.rs"),
        ],
        true,
    ));
    chrome::transcript(f, body, lines);
}

fn todo(marker: &str, mc: Color, text: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![
        Span::raw("     "),
        Span::styled(format!("{marker} "), Style::default().fg(mc)),
    ];
    out.extend(text);
    Line::from(out)
}
