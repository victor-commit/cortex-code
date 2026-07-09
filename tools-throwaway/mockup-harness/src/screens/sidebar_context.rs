//! Sidebar context tab: files in the active prompt context with token usage.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, t, SidebarTab, StatusLine};

const NAME_W: usize = 30;
const TOKENS_W: usize = 7;
const GAP_W: usize = 12;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Context,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("files in context"), m(" · 4 files")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        context_row("app.rs", "8.1k"),
        context_row("ui.rs", "6.4k"),
        context_row("view.rs", "3.8k"),
        context_row("sidebar.rs", "2.6k"),
        Line::from(""),
        total_row(),
        Line::from(vec![
            Span::raw("  "),
            m("@ "),
            m("add files to context"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn context_row(name: &str, tokens: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        t(&format!("{name:<NAME_W$}")),
        Span::raw(" ".repeat(GAP_W)),
        m(&format!("{tokens:>TOKENS_W$}")),
    ])
}

fn total_row() -> Line<'static> {
    let mut spans = vec![
        Span::raw("  "),
        m("context  "),
    ];
    spans.extend(chrome::bar(21, 100, 20));
    spans.extend([
        Span::raw("  "),
        t("21%"),
        m(" · 42.0k / 200k"),
    ]);
    Line::from(spans)
}
