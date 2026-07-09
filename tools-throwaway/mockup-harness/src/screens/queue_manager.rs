//! Queue manager dock with the next prompt and reorder controls.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, NumberedListRow, StatusLine};

const QUEUED_PROMPTS: &[NumberedListRow] = &[
    NumberedListRow {
        prompt: "Add regression coverage for the token refresh boundary",
        meta: "next",
    },
    NumberedListRow {
        prompt: "Trace the retry backoff after a provider timeout",
        meta: "queued",
    },
    NumberedListRow {
        prompt: "Review the new session catalog for duplicate rows",
        meta: "queued",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("stabilize the provider retry path", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I am checking the failed request boundary before I update the policy."),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Queued prompts",
        &[
            ("ctrl+↑", "move up"),
            ("ctrl+↓", "move down"),
            ("d", "drop"),
            ("esc", "close"),
        ],
    );
    let mut lines = vec![Line::from(vec![
        ratatui::text::Span::raw("  "),
        chrome::m("The queue runs automatically after the current turn finishes."),
    ])];
    lines.push(Line::from(""));
    lines.extend(chrome::numbered_select_list(
        inner.width as usize,
        QUEUED_PROMPTS,
        0,
    ));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(
        inner.width as usize,
        "3 prompts queued",
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
