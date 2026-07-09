//! auth/login sign-in landing with a first-run neutral welcome.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, ListRow, StatusLine, FAINT};

const SIGN_IN_OPTIONS: &[ListRow] = &[
    ListRow::Section("Quick sign-in"),
    ListRow::Item {
        name: "Sign in with browser",
        detail: "authorize Cortex in your browser",
        meta: "",
    },
    ListRow::Item {
        name: "Use an API key",
        detail: "paste a provider API key",
        meta: "",
    },
    ListRow::Section("All providers"),
    ListRow::Item {
        name: "Browse provider catalogue",
        detail: "choose from every available provider",
        meta: "",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "sign in", &status);

    f.render_widget(
        Paragraph::new(chrome::wordmark("v0.3.0")),
        Rect::new(body.x, body.y + 2, body.width, 1),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Choose how to sign in"),
        ])),
        Rect::new(body.x, body.y + 4, body.width, 1),
    );

    let card_area = Rect::new(body.x, body.y + 10, body.width, 11);
    let inner = chrome::card(
        f,
        card_area,
        "Sign in",
        &[("enter", "continue"), ("↑↓", "move"), ("esc", "quit")],
    );
    let mut lines = chrome::sectioned_select_list(inner.width as usize, SIGN_IN_OPTIONS, 0, None);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        m("Stored locally in "),
        c("auth.json", FAINT),
        m(" (0600)"),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}
