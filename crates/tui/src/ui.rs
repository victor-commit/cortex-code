//! Rendering for the Cortex Code TUI scaffold: an empty/home transcript area, a
//! bottom prompt box, and a status footer. Layout is recomputed every frame so the
//! screen reflows on resize.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use crate::app::App;

/// Maximum number of visible prompt text rows before the box stops growing.
const MAX_PROMPT_ROWS: usize = 8;

/// Renders the whole frame for the current [`App`] state.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let prompt_rows = prompt_rows(app, area.height);
    let chunks = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(prompt_rows + 2),
        Constraint::Length(1),
    ])
    .split(area);

    draw_home(frame, chunks[0]);
    draw_prompt(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);
}

/// Number of text rows the prompt box should show (clamped to available height).
fn prompt_rows(app: &App, total_height: u16) -> u16 {
    let logical = app.input.line_count().max(1);
    // Leave room for at least one home row, the prompt borders, and the footer.
    let budget = total_height.saturating_sub(1 + 2 + 1).max(1) as usize;
    logical.min(MAX_PROMPT_ROWS).min(budget).max(1) as u16
}

fn draw_home(frame: &mut Frame, area: Rect) {
    let heading_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);

    let body = Text::from(vec![
        Line::from(Span::styled("Cortex Code", heading_style)),
        Line::from(""),
        Line::from("Your terminal coding agent — start a new conversation."),
        Line::from(""),
        Line::from(Span::styled(
            "Type a message and press Enter to begin.",
            dim,
        )),
        Line::from(Span::styled(
            "Shift+Enter inserts a newline · /exit or Ctrl+C to quit.",
            dim,
        )),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Cortex Code ");

    let paragraph = Paragraph::new(body)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, area);
}

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Prompt ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let value = app.input_value();
    if value.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            "Type a message…  (/ for commands)",
            Style::default().add_modifier(Modifier::DIM),
        ));
        frame.render_widget(placeholder, inner);
    } else {
        let lines: Vec<Line> = app
            .input
            .lines()
            .iter()
            .map(|l| Line::from(l.clone()))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    if inner.width > 0 && inner.height > 0 {
        let cursor = app.cursor();
        let cx = inner.x + (cursor.col as u16).min(inner.width - 1);
        let cy = inner.y + (cursor.row as u16).min(inner.height - 1);
        frame.set_cursor_position((cx, cy));
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let label = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let value = Style::default().fg(Color::Gray);
    let sep = Span::styled(" · ", Style::default().fg(Color::DarkGray));

    let line = Line::from(vec![
        Span::styled("model ", label),
        Span::styled(app.status.model.clone(), value),
        sep.clone(),
        Span::styled("agent ", label),
        Span::styled(app.status.agent.clone(), value),
        sep.clone(),
        Span::styled("mode ", label),
        Span::styled(app.status.mode.clone(), value),
        sep,
        Span::styled("session ", label),
        Span::styled(app.status.session.clone(), value),
    ]);

    let footer = Paragraph::new(line).style(Style::default().bg(Color::Indexed(236)));
    frame.render_widget(footer, area);
}
