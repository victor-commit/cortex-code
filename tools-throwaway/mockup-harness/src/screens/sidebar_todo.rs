use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, m, t, SidebarTab, StatusLine, StepState};

const ROW_W: usize = 50;
const LABEL_W: usize = 22;
const STATUS_W: usize = 10;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let sidebar = chrome::sidebar_panel(
        f,
        SidebarTab::Todo,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let lines = vec![
        Line::from(vec![Span::raw("  "), m("plan"), m(" · "), t("3 of 5 done")]),
        chrome::rule_line(sidebar.panel.width as usize - 1),
        chrome::step_row(
            ROW_W,
            StepState::DoneDim,
            "Read auth middleware",
            LABEL_W,
            "reviewed",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::DoneDim,
            "Add token limiter",
            LABEL_W,
            "implemented",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::DoneDim,
            "Wire POST /login route",
            LABEL_W,
            "connected",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::CurrentDot,
            "Add rate-limit tests",
            LABEL_W,
            "fifth try",
            "current",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::PendingText,
            "Run cargo test",
            LABEL_W,
            "verify",
            "pending",
            STATUS_W,
        ),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            m("3 of 5 done · 1 in progress · 1 pending"),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), sidebar.panel);
}
