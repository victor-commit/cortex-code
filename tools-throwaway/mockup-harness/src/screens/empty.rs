use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, StatusLine, ACCENT, BRIGHT};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "sessions", &status);
    let center_y = body.y + body.height.saturating_sub(4) / 2;

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "No sessions yet",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        Rect::new(body.x, center_y, body.width, 1),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            m("Press "),
            c("n", ACCENT),
            m(" to start your first session"),
        ]))
        .alignment(Alignment::Center),
        Rect::new(body.x, center_y + 2, body.width, 1),
    );
}
