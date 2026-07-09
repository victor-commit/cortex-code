//! Session screen for the expanded findings of a completed delegated task.
//! Finding rows share a fixed reference column under the result elbow.

use ratatui::{
    style::Color,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, StatusLine, BAD, OK, WARN};

use super::tool_task;

const DETAIL_W: usize = 61;

/// One finding row with a colored severity glyph and a fixed `file:line`
/// column. Findings with blockers appear before review and confirmation rows.
fn finding(
    glyph: &str,
    hue: Color,
    severity: &str,
    detail: &str,
    reference: &str,
) -> Vec<Span<'static>> {
    vec![
        c(&format!("{glyph} "), hue),
        t(&format!("{severity:<10}")),
        t(&format!("{detail:<DETAIL_W$}")),
        t(reference),
    ]
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let rows = vec![
        tool_task::completed_summary(true),
        finding(
            "✗",
            BAD,
            "blocking",
            "Refresh tokens do not check a revoked session.",
            "crates/auth/src/refresh.rs:84",
        ),
        finding(
            "✗",
            BAD,
            "blocking",
            "The test helper creates a session without token state.",
            "tests/login_ratelimit.rs:31",
        ),
        finding(
            "●",
            WARN,
            "review",
            "Login retries share no per-user limit.",
            "crates/auth/src/login.rs:118",
        ),
        finding(
            "✓",
            OK,
            "confirmed",
            "Logout already invalidates the active token.",
            "crates/auth/src/logout.rs:57",
        ),
        finding(
            "✓",
            OK,
            "confirmed",
            "The migration keeps existing session rows valid.",
            "crates/db/src/migrate.rs:204",
        ),
        finding(
            "✓",
            OK,
            "confirmed",
            "Password reset clears stale login attempts.",
            "crates/auth/src/reset.rs:92",
        ),
    ];

    let mut lines: Vec<Line> = tool_task::turn_open();
    lines.push(tool_task::completed_task());
    lines.extend(chrome::elbow(rows));

    chrome::transcript(f, body, lines);
}
