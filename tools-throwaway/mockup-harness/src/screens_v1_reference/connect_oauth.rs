//! auth/connect-oauth — device-code authorization. A centered panel with the
//! verification URL, the user code shown large and spaced, a live `●` waiting
//! status, and a cancel hint. Over the shared CLI chrome.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, cb, ACCENT, AMBER, BRIGHT, DIM, FAINT, INFO, TXT};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "auth \u{00b7} connect \u{00b7} oauth",
        10,
        &[("o", "open browser"), ("esc", "cancel")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Authorize device", AMBER, &[]);
    let lines: Vec<Line> = vec![
        Line::from(vec![
            cb("1  ", ACCENT),
            chrome::c("Visit this URL in your browser", TXT),
        ]),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                "https://github.com/login/device",
                Style::default().fg(INFO).add_modifier(Modifier::UNDERLINED),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            cb("2  ", ACCENT),
            chrome::c("Enter this one-time code", TXT),
        ]),
        Line::from(vec![
            Span::raw("   "),
            cb("WDJB \u{2014} MJHT", BRIGHT),
        ]),
        Line::from(""),
        Line::from(vec![
            chrome::dot(ACCENT),
            chrome::c(" Waiting for authorization\u{2026}", AMBER),
        ]),
        Line::from(vec![
            chrome::c("Code valid until 14:37 \u{2014} press ", FAINT),
            chrome::c("esc", DIM),
            chrome::c(" to cancel", FAINT),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        chrome::user_cmd("/login", "14:22:07"),
        Line::from(""),
        dim("   Authorizing this device with GitHub"),
    ]
}
