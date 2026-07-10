//! auth/connect-provider catalogue with explicit connection states.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, ConnectionListRow, StatusLine};

const PROVIDERS: &[ConnectionListRow] = &[
    ConnectionListRow {
        name: "OpenRouter",
        detail: "unified gateway · many models",
        connected: true,
        active: true,
    },
    ConnectionListRow {
        name: "Anthropic",
        detail: "Claude Opus · Sonnet · Haiku",
        connected: true,
        active: false,
    },
    ConnectionListRow {
        name: "OpenAI",
        detail: "GPT-5 · coding and reasoning",
        connected: false,
        active: false,
    },
    ConnectionListRow {
        name: "Google",
        detail: "Gemini Pro · Flash",
        connected: false,
        active: false,
    },
    ConnectionListRow {
        name: "Azure OpenAI",
        detail: "enterprise OpenAI deployment",
        connected: false,
        active: false,
    },
    ConnectionListRow {
        name: "GitHub Copilot",
        detail: "OAuth · editor models",
        connected: false,
        active: false,
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action_view(f, "/connect · choose a provider", 10, &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Connect a provider to use its models in this session"),
        ])),
        Rect::new(body.x, body.y + 2, body.width, 1),
    );

    let inner = chrome::card(
        f,
        dock,
        "Connect a provider",
        &[
            ("↑↓", "select"),
            ("enter", "connect"),
            ("d", "disconnect selected OpenRouter"),
            ("esc", "close"),
        ],
    );
    let lines = chrome::connection_select_list(inner.width as usize, PROVIDERS, 0);
    f.render_widget(Paragraph::new(lines), inner);
}
