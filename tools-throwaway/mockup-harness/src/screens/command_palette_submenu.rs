//! Input dock for the queued-prompts command-palette scope.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, ListRow, StatusLine};

const QUEUED_PROMPTS: &[ListRow] = &[
    ListRow::Item {
        name: "Add buffer cap tests",
        detail: "run this prompt next",
        meta: "queued 1",
    },
    ListRow::Item {
        name: "Trace the retry backoff",
        detail: "drop this queued prompt",
        meta: "queued 2",
    },
    ListRow::Item {
        name: "Document the rate-limit path",
        detail: "run after the current turn",
        meta: "queued 3",
    },
    ListRow::Section("actions"),
    ListRow::Action {
        name: "Clear queued prompts",
        detail: "c clear · drop every queued prompt",
        meta: "",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("add a rate limiter to the login route", Some("4m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I am checking the expiry boundary before I update the handler."),
            Line::from(""),
            Line::from("   3 prompts will run after this turn."),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Palette › Queued prompts",
        &[
            ("↑↓", "move"),
            ("enter", "run"),
            ("d", "drop"),
            ("c", "clear"),
            ("esc", "back"),
        ],
    );
    let mut lines = vec![chrome::list_filter("›", "", "filter queued prompts")];
    lines.extend(chrome::sectioned_select_list(
        inner.width as usize,
        QUEUED_PROMPTS,
        0,
        None,
    ));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(inner.width as usize, "3 queued"));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
