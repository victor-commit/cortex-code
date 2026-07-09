//! Input dock for the root command palette.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, StatusLine};

const ACTIONS: &[(&str, &str, &str)] = &[
    (
        "Recall an older prompt",
        "place an earlier prompt in the draft",
        "ctrl+g h",
    ),
    (
        "Move toward the stashed draft",
        "step forward through prompt history",
        "ctrl+g l",
    ),
    (
        "Stash the current draft",
        "save this draft without sending it",
        "ctrl+g g",
    ),
    (
        "Restore the stashed draft",
        "put the saved draft back in the composer",
        "ctrl+g f",
    ),
    (
        "Queued prompts…",
        "manage queued prompt backlog",
        "ctrl+g 8 ▸",
    ),
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 11, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("add a rate limiter to the login route", Some("4m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I can add a small per-client limit and cover the expiry boundary."),
            Line::from(""),
            chrome::user_cmd("draft: add tests for the rejected login path", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Command palette",
        &[
            ("↑↓", "move"),
            ("enter", "run"),
            ("esc", "cancel"),
            ("type", "filter"),
        ],
    );
    let mut lines = vec![chrome::list_filter("›", "prompt", "search actions")];
    lines.extend(chrome::select_list(inner.width as usize, ACTIONS, 0, None));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(
        inner.width as usize,
        "5 of 28 actions",
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
