//! Builtin agent picker dock with current-agent and grant details.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, ListRow, StatusLine};

const AGENTS: &[ListRow] = &[
    ListRow::Item {
        name: "build",
        detail: "Default coding agent that implements requested changes · ● current",
        meta: "primary",
    },
    ListRow::Item {
        name: "plan",
        detail: "Planning agent that investigates and returns an implementation plan",
        meta: "subagent",
    },
    ListRow::Item {
        name: "general",
        detail: "General-purpose assistant for broad questions and guidance",
        meta: "subagent",
    },
    ListRow::Item {
        name: "explore",
        detail: "Read-only exploration agent for inspecting and reporting findings",
        meta: "subagent",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("inspect the production incident before changing anything", Some("2m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I can switch to a read-only agent for this investigation."),
            Line::from(""),
            chrome::user_cmd("/agents", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Select agent",
        &[
            ("↑↓", "move"),
            ("enter", "select"),
            ("esc", "cancel"),
            ("type", "filter"),
        ],
    );
    let mut lines = chrome::sectioned_select_list(inner.width as usize, AGENTS, 1, Some(""));
    lines.push(Line::from(""));
    lines.push(chrome::info(
        "plan grants",
        "read, glob, grep, and lsp · no shell, no writes",
        chrome::MUTED,
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
