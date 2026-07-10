//! auth/connect-apikey, masked API-key entry.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::chrome::{self, ACCENT, FAINT, MUTED, StatusLine, TXT, c, dot, m, t};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action_view(f, "/connect · Anthropic", 13, &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Add an API key to use Anthropic models"),
        ])),
        Rect::new(body.x, body.y + 3, body.width, 1),
    );

    let inner = chrome::card(
        f,
        dock,
        "Add API key",
        &[
            ("ctrl+v", "paste key from clipboard"),
            ("enter", "connect"),
            ("esc", "back"),
        ],
    );
    let lines = vec![
        Line::from(vec![m("  Provider: "), c("Anthropic · locked", FAINT)]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            c("sk-ant-", TXT),
            c("············", MUTED),
            t("1234"),
            c("█", ACCENT),
        ]),
        Line::from(vec![
            Span::raw("  "),
            dot(MUTED),
            m(" Format looks valid · 108 characters"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("Will be stored to auth.json (0600) · never shown"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            m("Back to chat once verified · usable in /models immediately"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
