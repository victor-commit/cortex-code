use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, StatusLine, ACCENT, BAD, FAINT};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 9, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("run the full test suite and report the result", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I stopped before running the requested command."),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Permission denied",
        &[("enter", "send"), ("esc", "skip")],
    );
    let lines = vec![
        Line::from(vec![
            chrome::m("  $ "),
            chrome::t("cargo test --all"),
            chrome::m("  "),
            chrome::c("✗ denied", BAD),
        ]),
        Line::from(""),
        Line::from("  Tell Cortex what would make this safe."),
        Line::from(""),
        Line::from(vec![
            chrome::m("  "),
            chrome::c("› ", ACCENT),
            chrome::c("why? sent to the agent", FAINT),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
