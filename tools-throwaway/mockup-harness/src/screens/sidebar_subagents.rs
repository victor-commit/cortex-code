use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, SidebarTab, StatusLine, BRIGHT, FAINT, MUTED, OK};

const ROW_W: usize = 50;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Subs,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("subagents"), m(" · 3 agents")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        agent_row(
            "explore",
            &format!("● running {}", chrome::timefmt(12.0)),
            MUTED,
        ),
        task_row("find existing auth middleware"),
        Line::from(""),
        agent_row(
            "general",
            &format!("✓ done {}", chrome::timefmt(64.0)),
            OK,
        ),
        task_row("summarize redaction audit"),
        Line::from(""),
        agent_row("plan", "○ queued", FAINT),
        task_row("design token-refresh flow"),
        Line::from(""),
        Line::from(vec![Span::raw("  "), m("1 running · 1 done · 1 queued")]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}

fn agent_row(name: &str, state: &str, state_color: ratatui::style::Color) -> Line<'static> {
    let gap = ROW_W
        .saturating_sub(2 + name.chars().count() + state.chars().count())
        .max(1);
    Line::from(vec![
        Span::raw("  "),
        c(name, BRIGHT),
        Span::raw(" ".repeat(gap)),
        c(state, state_color),
    ])
}

fn task_row(task: &str) -> Line<'static> {
    Line::from(vec![Span::raw("    "), m(task)])
}
