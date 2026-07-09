use ratatui::{text::Line, widgets::Paragraph, Frame};

use crate::chrome::{self, DiffRow, PermissionChoice, StatusLine, ToolStatus};

const CHOICES: &[PermissionChoice] = &[
    PermissionChoice {
        label: "Allow once",
        detail: "edit this file once",
    },
    PermissionChoice {
        label: "Allow always",
        detail: "this exact file, this session",
    },
    PermissionChoice {
        label: "Deny",
        detail: "leave the file unchanged",
    },
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 14, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("add token validation to the auth route", Some("1m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   I prepared the edit and need your approval."),
            Line::from(""),
            chrome::tool_line(
                ToolStatus::Running(None),
                "Edit",
                "crates/server/src/auth/routes.rs",
            ),
            chrome::elbow_note("3 lines changed · waiting for permission")[0].clone(),
        ],
    );

    let inner = chrome::card(
        f,
        dock,
        "Permission required",
        &[("1-3", "choose"), ("enter", "confirm"), ("esc", "deny")],
    );
    let mut lines = vec![
        Line::from(vec![
            chrome::t("  Allow Cortex to edit crates/server/src/auth/routes.rs?"),
            chrome::m("  +2 -1"),
        ]),
        Line::from(""),
    ];
    lines.extend(chrome::diff_lines(
        42,
        42,
        &[
            DiffRow::Ctx("fn route_for(request: Request) -> Response {"),
            DiffRow::Del("    let token = request.token();"),
            DiffRow::Add("    let token = request.auth_token()?;"),
            DiffRow::Ctx("    authorize(token)"),
            DiffRow::Ctx("}"),
        ],
    ));
    lines.push(Line::from(""));
    lines.extend(chrome::permission_choices(
        inner.width as usize,
        CHOICES,
        0,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
