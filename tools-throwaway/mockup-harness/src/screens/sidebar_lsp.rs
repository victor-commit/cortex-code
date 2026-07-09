//! Sidebar LSP tab: file-grouped diagnostics with explicit severity glyphs.

use ratatui::{
    style::Color,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, SidebarTab, StatusLine, BAD, MUTED, OK, WARN};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Lsp,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("diagnostics"), m(" · rust-analyzer")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        file_row(BAD, "buffer.rs", "1 error"),
        diagnostic(BAD, "E", "42", "capacity must be greater than zero"),
        file_row(WARN, "mod.rs", "2 warnings"),
        diagnostic(WARN, "W", "18", "unused import: Read"),
        diagnostic(WARN, "W", "90", "redundant clone()"),
        file_row(MUTED, "config.rs", "0 diagnostics"),
        Line::from(vec![Span::raw("      "), c("○ no problems", OK)]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("summary  "),
            c("1 error", BAD),
            m(" · "),
            c("2 warnings", WARN),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn file_row(color: Color, file: &str, count: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        chrome::dot(color),
        Span::raw(" "),
        t(&format!("{file:<17}")),
        c(count, color),
    ])
}

fn diagnostic(color: Color, level: &str, line: &str, message: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("      "),
        c(level, color),
        m(&format!(" {line:<4}")),
        t(message),
    ])
}
