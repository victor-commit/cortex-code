//! auth/connect-oauth, device-code authorization waiting state.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::chrome::{self, BRIGHT, FAINT, MUTED, StatusLine, c, cb, dot, m, t};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action_view(f, "/connect · GitHub Copilot", 16, &status);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("   "),
            m("Authorize GitHub Copilot to use its models"),
        ])),
        Rect::new(body.x, body.y + 3, body.width, 1),
    );

    let inner = chrome::card(
        f,
        dock,
        "Authorize device",
        &[("o", "open"), ("y", "copy code"), ("esc", "cancel")],
    );
    let expires = chrome::timefmt(872.0);
    let lines = vec![
        Line::from(vec![Span::raw("  "), m("1  Open this URL in your browser")]),
        Line::from(vec![
            Span::raw("     "),
            m("https://github.com/login/device"),
            m("  [o] open"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("  "), m("2  Enter this one-time code")]),
        Line::from(vec![
            Span::raw("     "),
            cb("WDJB-MJHT", BRIGHT),
            m("  y copy"),
        ]),
        Line::from(vec![
            Span::raw("     "),
            m(&format!("Code expires in {expires}")),
        ]),
        Line::from(vec![
            Span::raw("     "),
            c(chrome::ELBOW, FAINT),
            m(" refreshing"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            dot(MUTED),
            t(" Waiting for authorization…"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
