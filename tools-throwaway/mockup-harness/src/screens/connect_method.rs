//! auth/connect-method pick for an OAuth-capable provider.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, StatusLine};

const METHODS: &[(&str, &str, &str)] = &[
    ("API key", "paste a provider API key", ""),
    ("OAuth", "authorize in your browser", ""),
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "/connect · GitHub Copilot", &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Sign in to GitHub Copilot"),
        ])),
        Rect::new(body.x, body.y + 2, body.width, 1),
    );

    let card_area = Rect::new(body.x, body.y + 15, body.width, 6);
    let inner = chrome::card(
        f,
        card_area,
        "Choose a sign-in method",
        &[("↑↓", "select"), ("enter", "choose"), ("esc", "back")],
    );
    let lines = chrome::select_list(inner.width as usize, METHODS, 0, None);
    f.render_widget(Paragraph::new(lines), inner);
}
