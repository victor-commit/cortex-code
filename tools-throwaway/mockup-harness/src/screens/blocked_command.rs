//! Session screen for a command denied by plan mode. The inline failure block
//! names the attempted command, explains the write risk in plain English, and
//! leaves a nearby key path to continue.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, INFO};

pub fn render(f: &mut Frame) {
    let status = StatusLine {
        badge: Some(("plan", INFO)),
        ..StatusLine::default()
    };
    let body = chrome::cli_page(
        f,
        "plan the next step\u{2026}",
        &status,
    );

    let mut recovery: Vec<Span> = chrome::key_span("m", "switch to agent mode");
    recovery.push(c(&format!(" {} ", chrome::SEP), FAINT));
    recovery.extend(chrome::key_span("enter", "continue planning"));

    let mut lines = vec![
        chrome::user_cmd("! cargo build --release", Some("now")),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("✗ ", BAD),
            t("Command blocked"),
        ]),
    ];
    lines.extend(chrome::fail_rows(vec![
        vec![t("Plan mode blocks file edits.")],
        vec![m("This command would write build files to disk.")],
        recovery,
    ]));

    chrome::transcript(f, body, lines);
}
