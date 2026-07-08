//! auth/connect-apikey — paste a provider API key. A centered panel: the target
//! provider, a masked key field (showing the masked value with a block cursor),
//! a validation hint line, then Save / Cancel actions with Save emphasized as a
//! filled pill. Over the shared CLI chrome.

use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, ACCENT, AMBER, DIM, FAINT, OK, TXT};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "auth \u{00b7} connect \u{00b7} api key",
        9,
        &[
            ("enter", "save"),
            ("esc", "cancel"),
            ("paste", "insert key"),
        ],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Add API key", AMBER, &[]);
    let masked = "sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}1234";
    let lines: Vec<Line> = vec![
        Line::from(vec![
            chrome::c("Provider  ", FAINT),
            chrome::c("Anthropic", DIM),
        ]),
        Line::from(""),
        Line::from(vec![
            chrome::c(masked, TXT),
            chrome::c("\u{2588}", ACCENT),
        ]),
        Line::from(""),
        Line::from(vec![
            chrome::dot(OK),
            chrome::c(" Format looks valid \u{2014} 108 characters", DIM),
        ]),
        Line::from(""),
        Line::from(vec![
            chrome::pill("Save"),
            m("   "),
            chrome::c("Cancel", DIM),
            m("   "),
            chrome::c("stored locally in ", FAINT),
            chrome::c("auth.json", AMBER),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        chrome::user_cmd("/connect anthropic", "14:22:07"),
        Line::from(""),
        dim("   Paste your Anthropic API key to connect"),
    ]
}
