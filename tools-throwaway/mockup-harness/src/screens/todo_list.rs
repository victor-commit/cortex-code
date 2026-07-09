//! Session screen for an in-progress plan. The shared step primitive keeps
//! done, current, and pending states aligned while CurrentDot supplies the
//! transcript plan marker without changing onboarding's selected-step marker.

use ratatui::{text::Line, Frame};

use crate::chrome::{self, m, StatusLine, StepState, MUTED};

const ROW_W: usize = 118;
const LABEL_W: usize = 34;
const STATUS_W: usize = 12;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "type a follow-up, it runs after this turn\u{2026}",
        &status,
    );

    let lines = vec![
        chrome::user_cmd("implement rate limiting on the login endpoint", Some("5m ago")),
        Line::from(""),
        Line::from(vec![
            ratatui::text::Span::raw(" "),
            chrome::dot(MUTED),
            m(" Updated plan"),
        ]),
        Line::from(""),
        chrome::step_row(
            ROW_W,
            StepState::Done,
            "Read the auth middleware",
            LABEL_W,
            "reviewed the login handler",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::Done,
            "Add a token-bucket limiter",
            LABEL_W,
            "created the request guard",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::Done,
            "Wire the POST /login route",
            LABEL_W,
            "attached the limiter",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::CurrentDot,
            "Add tests/login_ratelimit.rs",
            LABEL_W,
            "cover the fifth attempt",
            "in progress",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::Pending,
            "Run cargo test and cargo fmt",
            LABEL_W,
            "verify the full change",
            "pending",
            STATUS_W,
        ),
        Line::from(""),
        Line::from(vec![m("     3 of 5 done"), m(" · 1 in progress · 1 pending")]),
        Line::from(""),
        chrome::working_band("Writing tests", 22.0, Some("1 file")),
    ];

    chrome::transcript(f, body, lines);
}
