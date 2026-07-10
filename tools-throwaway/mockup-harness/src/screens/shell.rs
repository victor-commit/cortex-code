//! Session screen for `!` shell mode. A completed command and a live command
//! keep the shared shell status column aligned, while their output stays
//! attached through the standard elbow primitive.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, LiveActivityInterrupt, ShellStatus, StatusLine, m, t};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut lines = vec![
        chrome::user_cmd("check the branch, then run the tests", Some("2m ago")),
        Line::from(""),
        chrome::shell_line(
            ShellStatus::Done(0.2),
            "git status --short --branch",
        ),
    ];
    lines.extend(chrome::elbow(vec![
        vec![m("## master...origin/master [ahead 1]")],
        vec![t("M crates/server/src/models/session.rs")],
        vec![t("?? crates/server/tests/login_regression.rs")],
    ]));
    lines.push(Line::from(""));
    lines.push(chrome::shell_line(
        ShellStatus::Running(None, LiveActivityInterrupt::Band),
        "cargo test --workspace",
    ));
    lines.extend(chrome::elbow(vec![
        vec![m("Compiling cortex-server v0.1.0")],
        vec![m("Finished test profile in 3.1s")],
        vec![t("test routes::login::rate_limited ... ok")],
        vec![
            t("test routes::api::quota_enforced ... "),
            t("█"),
        ],
    ]));
    lines.push(Line::from(""));
    lines.push(chrome::working_band("Testing", 4.0, None));

    chrome::transcript(f, body, lines);
}
