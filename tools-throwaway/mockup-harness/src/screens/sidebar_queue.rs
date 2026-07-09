use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, t, SidebarTab, StatusLine};

const ROW_W: usize = 50;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Queue,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("queued prompts"), m(" · 3 waiting")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        prompt_row(1, "Add regression coverage for refresh", "next"),
        prompt_row(2, "Trace retry backoff after timeout", "queued"),
        prompt_row(3, "Review session catalog duplicate rows", "queued"),
        Line::from(""),
        chrome::hints(&[
            ("ctrl+↑", "move"),
            ("ctrl+↓", "move"),
            ("d", "drop"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn prompt_row(position: usize, prompt: &str, meta: &str) -> Line<'static> {
    let prefix = format!("  {position} ");
    let gap = ROW_W
        .saturating_sub(prefix.chars().count() + prompt.chars().count() + meta.chars().count())
        .max(1);
    Line::from(vec![
        m(&prefix),
        t(prompt),
        Span::raw(" ".repeat(gap)),
        m(meta),
    ])
}
