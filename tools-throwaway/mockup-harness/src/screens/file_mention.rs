//! File mention picker attached to the composer.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, FuzzyFileRow, StatusLine};

const FILES: &[FuzzyFileRow] = &[
    FuzzyFileRow {
        before: "crates/providers/src/open",
        matched: "rou",
        after: "ter.rs",
        meta: "providers · 24.6k",
    },
    FuzzyFileRow {
        before: "crates/server/tests/config_provider_auth_",
        matched: "rou",
        after: "tes.rs",
        meta: "server/tests · 18.2k",
    },
    FuzzyFileRow {
        before: "crates/server/tests/session_",
        matched: "rou",
        after: "tes.rs",
        meta: "server/tests · 12.4k",
    },
    FuzzyFileRow {
        before: "crates/server/tests/file_find_catalog_",
        matched: "rou",
        after: "tes.rs",
        meta: "server/tests · 9.8k",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 11, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd(
                "trace how provider metadata reaches the model picker",
                Some("2m ago"),
            ),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I found the catalog boundary. Which file should I inspect next?"),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "@ files",
        &[("tab", "insert"), ("↑↓", "move"), ("esc", "dismiss")],
    );
    let mut lines = vec![chrome::list_counter(
        inner.width as usize,
        "4 matching project files",
    )];
    lines.extend(chrome::fuzzy_file_list(inner.width as usize, FILES, 0));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        ratatui::text::Span::raw("  "),
        chrome::c("› ", chrome::ACCENT),
        chrome::t("explain @rou"),
    ]));
    f.render_widget(ratatui::widgets::Paragraph::new(lines), inner);
}
