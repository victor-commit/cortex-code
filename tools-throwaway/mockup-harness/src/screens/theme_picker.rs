//! Brand theme picker with live-preview and cancel-restore semantics.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, StatusLine};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 11, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("make the review comfortable in daylight", Some("2m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Move through the themes to preview each palette before applying."),
            Line::from(""),
            chrome::user_cmd("/themes", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Themes",
        &[
            ("↑↓", "preview"),
            ("enter", "apply"),
            ("esc", "restores previous theme"),
        ],
    );
    let lines = chrome::theme_select_list(inner.width as usize, chrome::theme_previews(), 0);
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
