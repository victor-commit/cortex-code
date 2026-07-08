//! Screen — states/empty. The session list with nothing in it yet. No table, no
//! chrome noise — just a calm, centered message that there are no sessions, and
//! a single line on how to begin. A read-only view (no composer): the empty
//! surface a user meets the first time they open the session picker.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, ACCENT, BRIGHT, DIM, GHOST};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Sessions",
        &[
            ("n", "new session"),
            ("enter", "open"),
            ("q", "close view"),
        ],
    );

    // Center a small block vertically: spacer · message block · spacer.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(38),
            Constraint::Length(6),
            Constraint::Min(0),
        ])
        .split(body);

    let block = rows[1];
    let mut line = |y: u16, spans: Vec<Span<'static>>| {
        f.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
            Rect::new(block.x, block.y + y, block.width, 1),
        );
    };

    // Headline — calm, plain, centered.
    line(
        0,
        vec![Span::styled(
            "No sessions yet",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        )],
    );
    // A short faint rule under the headline for a settled, composed feel.
    line(
        1,
        vec![Span::styled(
            "\u{2500}".repeat(17),
            Style::default().fg(GHOST),
        )],
    );
    // One-line hint on how to start.
    line(
        3,
        vec![
            Span::styled("Press ", Style::default().fg(DIM)),
            Span::styled("n", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" to start your first session.", Style::default().fg(DIM)),
        ],
    );
}
