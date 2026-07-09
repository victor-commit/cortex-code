//! Filtered model picker dock with a typed query and narrowed matches.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, ListRow, StatusLine};

const MATCHES: &[ListRow] = &[
    ListRow::Section("Anthropic"),
    ListRow::Item {
        name: "Claude Opus 4.8",
        detail: "slow · best quality · ● current",
        meta: "200K ctx · $15/M",
    },
    ListRow::Item {
        name: "Claude Sonnet 4.6",
        detail: "fast · strong coding",
        meta: "200K ctx · $3/M",
    },
    ListRow::Item {
        name: "Claude Haiku 4.5",
        detail: "fast · low cost",
        meta: "200K ctx · $1/M",
    },
    ListRow::Item {
        name: "Claude Sonnet 4.5",
        detail: "balanced · coding",
        meta: "200K ctx · $3/M",
    },
    ListRow::Item {
        name: "Claude Opus 4.6",
        detail: "slow · deep reasoning",
        meta: "200K ctx · $15/M",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 13, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd(
                "which model should handle this multi-file migration?",
                Some("1m ago"),
            ),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I can use the active model or switch before the next turn."),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Select model",
        &[
            ("↑↓", "move"),
            ("enter", "select"),
            ("esc", "cancel"),
            ("type", "filter"),
        ],
    );
    let mut lines = chrome::sectioned_select_list(inner.width as usize, MATCHES, 0, Some("claude"));
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(
        inner.width as usize,
        "5 of 15 models",
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
