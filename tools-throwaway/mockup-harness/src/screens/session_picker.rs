//! Saved-session picker with query match, fixed age, and message-count columns.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, SessionListRow, StatusLine};

const SESSIONS: &[SessionListRow] = &[
    SessionListRow {
        title: "Refactor auth middleware",
        age: "2h ago",
        messages: "48 msgs",
        matches_query: false,
    },
    SessionListRow {
        title: "Fix flaky rate-limit test",
        age: "5h ago",
        messages: "22 msgs",
        matches_query: false,
    },
    SessionListRow {
        title: "Review yesterday's release",
        age: "yesterday",
        messages: "91 msgs",
        matches_query: true,
    },
    SessionListRow {
        title: "Migrate sessions table",
        age: "4d ago",
        messages: "37 msgs",
        matches_query: false,
    },
    SessionListRow {
        title: "Wire revocation endpoint",
        age: "9d ago",
        messages: "63 msgs",
        matches_query: false,
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("resume yesterday's release review", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I found the saved project sessions that match your query."),
            Line::from(""),
            chrome::user_cmd("/sessions yesterday", None),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Sessions",
        &[
            ("↑↓", "move"),
            ("enter", "open"),
            ("d", "delete"),
            ("esc", "close"),
        ],
    );
    let mut lines =
        chrome::session_select_list(inner.width as usize, SESSIONS, 2, "yesterday");
    lines.push(Line::from(""));
    lines.push(chrome::list_counter(
        inner.width as usize,
        "1 of 5 sessions matches",
    ));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
