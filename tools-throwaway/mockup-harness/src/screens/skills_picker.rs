//! Session skills picker with explicit pending toggle state and apply control.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, StatusLine, ToggleListRow};

const SKILLS: &[ToggleListRow] = &[
    ToggleListRow {
        name: "commit-identity",
        detail: "Commit as the linked identity",
        source: "loaded",
        enabled: true,
    },
    ToggleListRow {
        name: "cortex-code-design",
        detail: "Apply Cortex Code interface rules",
        source: "loaded",
        enabled: true,
    },
    ToggleListRow {
        name: "screen-worker",
        detail: "Build mockup screens cleanly",
        source: "loaded",
        enabled: true,
    },
    ToggleListRow {
        name: "vision-opus",
        detail: "Review rendered interface images",
        source: "loaded",
        enabled: true,
    },
    ToggleListRow {
        name: "firecrawl",
        detail: "Search and extract web content",
        source: "available",
        enabled: false,
    },
    ToggleListRow {
        name: "security-review",
        detail: "Audit code for security risks",
        source: "available",
        enabled: false,
    },
    ToggleListRow {
        name: "use-figma",
        detail: "Translate a Figma design",
        source: "available",
        enabled: false,
    },
    ToggleListRow {
        name: "install-qa",
        detail: "Set up automated QA checks",
        source: "available",
        enabled: false,
    },
    ToggleListRow {
        name: "simplify",
        detail: "Reduce accidental complexity",
        source: "available",
        enabled: false,
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 15, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("prepare this mockup set for review", Some("2m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Four session skills are loaded. You can change the working set."),
            Line::from(""),
            chrome::user_cmd("/skills", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Skills",
        &[
            ("↑↓", "move"),
            ("space", "toggle"),
            ("enter", "apply"),
            ("esc", "cancel"),
        ],
    );
    let mut lines = chrome::toggle_select_list(inner.width as usize, SKILLS, 4, Some(""));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(
        inner.width as usize,
        "4 of 9 enabled",
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
