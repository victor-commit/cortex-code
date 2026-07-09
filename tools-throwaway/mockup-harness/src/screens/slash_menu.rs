//! Input dock for the builtin slash-command autocomplete menu.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, ListRow, StatusLine};

const ROWS: &[ListRow] = &[
    ListRow::Section("session"),
    ListRow::Item {
        name: "/help",
        detail: "Show the command reference and keybindings",
        meta: "",
    },
    ListRow::ItemWithMutedMeta {
        name: "/new",
        detail: "Start a fresh session",
        meta: "ctrl+g n",
    },
    ListRow::ItemWithMutedMeta {
        name: "/sessions",
        detail: "Open the session picker",
        meta: "ctrl+g s",
    },
    ListRow::Section("pickers"),
    ListRow::ItemWithMutedMeta {
        name: "/models",
        detail: "Open the model picker",
        meta: "ctrl+g m",
    },
    ListRow::ItemWithMutedMeta {
        name: "/agents",
        detail: "Open the agent picker",
        meta: "ctrl+g a",
    },
    ListRow::ItemWithMutedMeta {
        name: "/skills",
        detail: "Open the skill picker",
        meta: "ctrl+g 5",
    },
    ListRow::Section("views"),
    ListRow::Item {
        name: "/status",
        detail: "Open the status view",
        meta: "",
    },
    ListRow::Item {
        name: "/timeline",
        detail: "Open the session timeline",
        meta: "",
    },
    ListRow::Item {
        name: "/review",
        detail: "Run a code review",
        meta: "",
    },
    ListRow::Item {
        name: "/diff",
        detail: "Open the diff viewer",
        meta: "",
    },
    ListRow::Section("project"),
    ListRow::Item {
        name: "/compact",
        detail: "Compact the conversation",
        meta: "",
    },
    ListRow::Item {
        name: "/mcp",
        detail: "Open the MCP status view",
        meta: "",
    },
    ListRow::Item {
        name: "/init",
        detail: "Initialize the project guide",
        meta: "",
    },
    ListRow::Item {
        name: "/export",
        detail: "Export the current session",
        meta: "",
    },
    ListRow::Item {
        name: "/exit",
        detail: "Exit Cortex Code",
        meta: "",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 23, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd(
                "refactor the auth middleware to add rate limiting",
                Some("4m ago"),
            ),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from(
                "   The route now rejects repeated failed attempts and keeps the response generic.",
            ),
            Line::from(""),
            chrome::user_cmd("/", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Commands",
        &[
            ("↑↓", "move"),
            ("enter", "run"),
            ("esc", "cancel"),
            ("type", "filter"),
        ],
    );
    let lines = chrome::sectioned_select_list(inner.width as usize, ROWS, 0, Some(""));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
