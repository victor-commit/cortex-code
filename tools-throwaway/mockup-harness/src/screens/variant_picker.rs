//! Reasoning-variant dock for the four runtime effort modes.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, StatusLine};

const VARIANTS: &[(&str, &str, &str)] = &[
    ("default", "default reasoning effort", ""),
    ("fast", "fastest, minimal reasoning", ""),
    ("balanced", "balanced latency / quality", ""),
    ("quality", "maximum reasoning effort", ""),
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 10, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("keep this refactor responsive without losing coverage", Some("3m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Balanced is active for the next turn."),
            Line::from(""),
            chrome::user_cmd("/variant", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Reasoning variant",
        &[("↑↓", "move"), ("enter", "select"), ("esc", "close")],
    );
    let lines = chrome::select_list(inner.width as usize, VARIANTS, 2, None);
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
