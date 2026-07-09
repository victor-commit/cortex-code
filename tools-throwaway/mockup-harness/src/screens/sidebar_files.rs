//! Sidebar files tab: touched workspace tree with explicit diff change badges.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, c, m, t, SidebarTab, StatusLine, BAD, DISCLOSURE_CLOSED, DISCLOSURE_OPEN, FAINT, OK,
    WARN,
};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Files,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("touched files"), m(" · 3 modified")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        directory(0, DISCLOSURE_OPEN, "crates"),
        directory(1, DISCLOSURE_OPEN, "tui"),
        changed_file(2, "app.rs", "+12", "-4"),
        changed_file(2, "ui.rs", "+7", "-3"),
        unchanged_file(2, "mod.rs"),
        directory(1, DISCLOSURE_CLOSED, "server"),
        changed_file(0, "Cargo.toml", "+1", "-1"),
        unchanged_file(0, "README.md"),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("3 modified"),
            m(" · "),
            t("2 unchanged"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn directory(depth: usize, disclosure: &str, name: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{}{}", "  ".repeat(depth + 1), disclosure)),
        Span::raw(" "),
        c(name, FAINT),
    ])
}

fn changed_file(depth: usize, name: &str, added: &str, removed: &str) -> Line<'static> {
    let indent = "  ".repeat(depth + 1);
    let name_w = 19usize.saturating_sub(depth * 2);
    Line::from(vec![
        Span::raw(indent),
        c("● ", WARN),
        t(&format!("{name:<name_w$}")),
        c(added, OK),
        m(" "),
        c(removed, BAD),
        m(" modified"),
    ])
}

fn unchanged_file(depth: usize, name: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  ".repeat(depth + 1)),
        t(name),
        m("  unchanged"),
    ])
}
