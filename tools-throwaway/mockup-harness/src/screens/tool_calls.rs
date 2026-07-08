//! Session screen - the canonical tool-call showcase: one transcript with a
//! succeeded, a failed and a running call, each a one-line tool call with its
//! result attached below via the chrome elbow. No working band (showcase
//! screens may omit it), so the running call carries its elapsed on the
//! line - elapsed renders exactly once per activity.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, m, t, StatusLine, ToolStatus};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut done = chrome::verdict(true, "214 lines");
    done.push(m(&format!(" {} {}", chrome::SEP, chrome::timefmt(0.3))));
    let mut failed = chrome::verdict(false, "2 of 38 tests failed");
    failed.push(m(&format!(
        " {} exit 101 {} {}",
        chrome::SEP,
        chrome::SEP,
        chrome::timefmt(41.0)
    )));

    let mut lines: Vec<Line> = vec![
        chrome::user_cmd(
            "find where the poll cadence is set and run the tui tests",
            Some("1m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Reading the app loop first, then running the suite."),
        ]),
        Line::from(""),
        chrome::tool_line(ToolStatus::Done, "Read", "crates/tui/src/app.rs"),
    ];
    lines.extend(chrome::elbow(vec![done]));
    lines.push(Line::from(""));

    lines.push(chrome::tool_line(
        ToolStatus::Failed,
        "Bash",
        "cargo test -p cortex-tui",
    ));
    lines.extend(chrome::elbow(vec![
        failed,
        vec![m(
            "poll::cadence_backs_off: assertion failed at crates/tui/tests/poll.rs:41",
        )],
        chrome::key_span("r", "retry"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::tool_line(
        ToolStatus::Running(Some(12.0)),
        "Search",
        "pattern: \"poll_cadence\"",
    ));
    lines.extend(chrome::elbow_note("118 of 240 files scanned"));
    lines.push(Line::from(""));

    chrome::transcript(f, body, lines);
}
