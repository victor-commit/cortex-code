use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "reply when ready…", &status);

    let lines = vec![
        chrome::user_cmd("/undo", Some("1m ago")),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t("Undid last change · reverted 2 files"),
        ]),
        field("checkpoint", "auth-route cleanup · turn 14 restored"),
        Line::from(""),
        field("files", "crates/server/src/auth/mod.rs"),
        detail(vec![Span::raw("            "), t("crates/server/src/auth/routes.rs")]),
        Line::from(""),
        recovery("/redo", "re-apply the undone change"),
    ];

    chrome::transcript(f, body, lines);
}

fn detail(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("     ")];
    row.append(&mut spans);
    Line::from(row)
}

fn field(label: &str, value: &str) -> Line<'static> {
    detail(vec![m(&format!("{label:<12}")), t(value)])
}

fn recovery(key: &str, label: &str) -> Line<'static> {
    let mut spans = vec![Span::raw("     ")];
    spans.extend(chrome::key_span(key, label));
    Line::from(spans)
}
