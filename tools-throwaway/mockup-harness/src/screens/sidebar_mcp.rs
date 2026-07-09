use ratatui::{
    style::Color,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, cb, m, t, SidebarTab, StatusLine, BRIGHT, OK, WARN};

const ROW_W: usize = 50;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Mcp,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("MCP servers"), m(" · 3 configured")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        server_row("✓", "context7", "connected", OK, 2, false),
        server_row("✓", "github", "connected", OK, 5, false),
        server_row("●", "playwright", "needs auth", WARN, 0, true),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("2 connected · 1 needs attention"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn server_row(
    marker: &str,
    name: &str,
    state: &str,
    state_color: Color,
    tools: usize,
    attention: bool,
) -> Line<'static> {
    let tools = format!("{tools} tools");
    let used = 2 + 2 + name.chars().count() + 1 + state.chars().count() + tools.chars().count();
    let gap = ROW_W.saturating_sub(used).max(1);
    Line::from(vec![
        Span::raw("  "),
        c(&format!("{marker} "), state_color),
        t(name),
        Span::raw(" "),
        if attention { cb(state, BRIGHT) } else { t(state) },
        Span::raw(" ".repeat(gap)),
        m(&tools),
    ])
}
