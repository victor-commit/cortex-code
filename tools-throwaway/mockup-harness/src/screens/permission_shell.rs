use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, PermissionChoice, StatusLine, ToolStatus, WARN};

const CHOICES: &[PermissionChoice] = &[
    PermissionChoice {
        label: "Allow once",
        detail: "run this command once",
    },
    PermissionChoice {
        label: "Allow always",
        detail: "this exact command, this session",
    },
    PermissionChoice {
        label: "Deny",
        detail: "do not run it",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 12, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("run the full test suite and report the result", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I prepared the test command and need your approval."),
            Line::from(""),
            chrome::tool_line(ToolStatus::Waiting, "Bash", "cargo test --all"),
            chrome::elbow_note("waiting for permission")[0].clone(),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Permission required",
        &[("1-3", "choose"), ("enter", "confirm"), ("esc", "deny")],
    );
    let mut lines = vec![
        Line::from("  Allow Cortex to run this shell command?"),
        Line::from(""),
        Line::from(vec![
            chrome::m("  $ "),
            chrome::t("cargo test "),
            chrome::c("--all", WARN),
            chrome::m("    in "),
            chrome::m("~/projects/cortex-code"),
        ]),
        Line::from(""),
    ];
    lines.extend(chrome::permission_choices(
        inner.width as usize,
        CHOICES,
        0,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
