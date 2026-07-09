//! Session export format picker dock for `/export`.

use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, AnnotatedListRow, StatusLine};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 10, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("save this investigation for the release review", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Choose the format that best fits where you will use the session."),
            Line::from(""),
            chrome::user_cmd("/export", None),
        ],
    );

    let rows = [
        AnnotatedListRow {
            name: "Markdown",
            detail: "readable .md · headings + code",
            meta: "default".to_owned(),
            meta_color: chrome::INFO,
            disabled: false,
        },
        AnnotatedListRow {
            name: "JSON",
            detail: "structured messages (.json)",
            meta: String::new(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "HTML",
            detail: "styled, shareable (.html)",
            meta: String::new(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "Patch",
            detail: "git-applyable diff (.patch)",
            meta: String::new(),
            meta_color: chrome::MUTED,
            disabled: false,
        },
    ];
    let inner = chrome::card(
        f,
        dock,
        "Export session",
        &[("↑↓", "move"), ("enter", "export"), ("esc", "close")],
    );
    let lines = chrome::annotated_select_list(inner.width as usize, &rows, 0, None);
    f.render_widget(Paragraph::new(lines), inner);
}
