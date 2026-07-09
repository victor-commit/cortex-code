//! Timeline-backed fork picker dock for `/fork`.

use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, StatusLine, TimelineListRow};

const TURNS: &[TimelineListRow] = &[
    TimelineListRow {
        key: "14:22  you",
        excerpt: "trace the token refresh path before editing",
    },
    TimelineListRow {
        key: "14:23  cortex",
        excerpt: "I found the cache boundary and its callers.",
    },
    TimelineListRow {
        key: "14:26  you",
        excerpt: "keep the existing invalidation behavior",
    },
    TimelineListRow {
        key: "14:27  cortex",
        excerpt: "The change can stay within the provider cache.",
    },
    TimelineListRow {
        key: "14:31  you",
        excerpt: "show the smallest safe patch before applying it",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("branch from the earlier cache investigation", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Pick a message to use as the starting point for the new branch."),
            Line::from(""),
            chrome::user_cmd("/fork", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Fork from message",
        &[("↑↓", "move"), ("enter", "fork"), ("esc", "close")],
    );
    let mut lines = chrome::timeline_select_list(inner.width as usize, TURNS, TURNS.len() - 1);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        chrome::m("  enter fork "),
        chrome::c("→", chrome::FAINT),
        chrome::m(" new session from this turn"),
    ]));
    f.render_widget(Paragraph::new(lines), inner);
}
