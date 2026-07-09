//! auth/connect-disconnect, confirmation before a stored credential is removed.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::chrome::{self, StatusLine, m, t};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "/connect · Anthropic", &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Review what will change before disconnecting"),
        ])),
        Rect::new(body.x, body.y + 4, body.width, 1),
    );

    let card_area = Rect::new(body.x, body.y + 12, body.width, 13);
    let inner = chrome::card(
        f,
        card_area,
        "Disconnect provider",
        &[("esc", "cancel, keep connected"), ("enter", "disconnect")],
    );
    let lines = vec![
        Line::from(vec![Span::raw("  "), t("Disconnect Anthropic?")]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("This removes the stored credential from auth.json"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            m("Its models will require reconnecting before you can use them"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), m("Nothing has been removed yet")]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
