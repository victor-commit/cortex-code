//! auth/connect-connected, confirmation after a provider is connected.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::chrome::{self, OK, StatusLine, c, m};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action_view(f, "/connect · Anthropic", 10, &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Your provider is ready to use"),
        ])),
        Rect::new(body.x, body.y + 4, body.width, 1),
    );

    let inner = chrome::card(
        f,
        dock,
        "Connected",
        &[("enter", "done, return to chat"), ("esc", "close")],
    );
    let lines = vec![
        Line::from(vec![Span::raw("  "), c("✓ connected Anthropic", OK)]),
        Line::from(vec![Span::raw("  "), m("Authentication method: API key")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("Usable now · open /models to pick a model"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
