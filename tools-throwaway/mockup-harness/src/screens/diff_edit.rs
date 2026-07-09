//! Session screen for a proposed edit awaiting a key-driven decision.
//! The dock is a choice prompt, not a free-text composer.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, DiffRow, StatusLine, ToolStatus, MUTED};

/// A prose row aligned with the transcript's primary text edge.
fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 5, &status);

    let mut lines = vec![
        chrome::user_cmd("give login rows a default token_version", Some("4m ago")),
        Line::from(""),
        chrome::tool_line(
            ToolStatus::Done,
            "Proposed an edit to crates/server/src/models/session.rs",
            "",
        ),
    ];
    lines.extend(chrome::elbow(vec![vec![m("+3 -1 \u{00b7} 1 hunk")]]));
    lines.push(Line::from(""));
    lines.extend(chrome::diff_lines(
        18,
        18,
        &[
            DiffRow::Ctx("pub fn new(user: UserId) -> Self {"),
            DiffRow::Ctx("    Self {"),
            DiffRow::Ctx("        user,"),
            DiffRow::Del("        token_version: None,"),
            DiffRow::Add("        token_version: Some(1),"),
            DiffRow::Add("        created_at: Clock::now(),"),
            DiffRow::Add("        refresh_after: now + TOKEN_TTL,"),
            DiffRow::Ctx("    }"),
            DiffRow::Ctx("}"),
        ],
    ));
    lines.push(Line::from(""));
    lines.push(para(vec![
        m("Rationale "),
        t("token_version: None "),
        c("\u{2192}", MUTED),
        t(" Some(1)"),
    ]));
    lines.push(para(vec![t(
        "New session rows start with a version, so refresh checks can compare it safely.",
    )]));

    chrome::transcript(f, body, lines);

    let inner = chrome::card(
        f,
        dock,
        "Proposed edit",
        &[("y", "apply"), ("n", "skip"), ("e", "edit")],
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![m("[y/n/e] key prompt")]),
            Line::from(vec![m("○ Pending decision \u{00b7} no files changed")]),
        ]),
        inner,
    );
}
