//! Session screen - first-run setup: the ASCII "Code" banner (rule F-6), a
//! title/version header, and the three setup steps as leader-dot rows. The
//! current step is marked by the `›` glyph plus a SURFACE wash and BRIGHT
//! label (never color alone); the continue/skip keys render on one MUTED
//! line adjacent to the steps (rule N-9: no global hint row).

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, StepState, FAINT};

/// Fixed columns shared by the three step rows.
const ROW_W: usize = 118;
const LABEL_W: usize = 23;
const STATUS_W: usize = 12;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "finish setup to start chatting\u{2026}", &status);

    let mut lines: Vec<Line> = chrome::banner();
    lines.extend(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Welcome to Cortex Code."),
            m("  v0.3.0"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Three quick steps and you're ready"),
            c(&format!(" {} ", chrome::SEP), FAINT),
            m("step 2 of 3"),
        ]),
        Line::from(""),
        chrome::step_row(
            ROW_W,
            StepState::Done,
            "1  Pick a theme",
            LABEL_W,
            "Cortex Dark, switch any time with /themes",
            "done",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::Current,
            "2  Choose a model",
            LABEL_W,
            "Claude Opus 4.8 is the default for planning and edits",
            "current step",
            STATUS_W,
        ),
        chrome::step_row(
            ROW_W,
            StepState::Pending,
            "3  Connect your account",
            LABEL_W,
            "sign in to your provider to run the agent",
            "pending",
            STATUS_W,
        ),
        Line::from(""),
        chrome::hints(&[("enter", "continue"), ("s", "skip for now")]),
        Line::from(""),
    ]);

    chrome::transcript(f, body, lines);
}
