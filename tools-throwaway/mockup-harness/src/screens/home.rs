//! Session screen - the landing view right after the first exchange: brand
//! wordmark, greeting and one session-meta row above a faint rule, the first
//! turn beneath it. Keyboard hints live ONLY in the footer row (never
//! repeated in the welcome block).

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, FAINT};

pub fn render(f: &mut Frame) {
    let status = StatusLine {
        ctx_pct: 6,
        cost: "$0.02",
        ..Default::default()
    };
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
        &[
            ("enter", "send"),
            ("/", "commands"),
            ("@", "files"),
            ("?", "help"),
        ],
    );

    let lines: Vec<Line> = vec![
        chrome::wordmark("v0.3.0"),
        Line::from(vec![Span::raw("   "), t("Welcome back.")]),
        Line::from(""),
        meta_row(),
        Line::from(""),
        chrome::rule_line(118),
        Line::from(""),
        chrome::user_cmd("summarize what changed since yesterday", Some("2m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para("Two commits landed overnight: the poll cadence fix and a models.dev catalogue refresh."),
        para("Tests are green on master; nothing needs your attention."),
        Line::from(""),
    ];

    chrome::transcript(f, body, lines);
}

/// One-line session meta panel at the column-4 text edge: muted labels,
/// body-text values, faint dots.
fn meta_row() -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("   ")];
    let pairs = [
        ("directory", "~/projects/cortex-code"),
        ("model", "claude-opus"),
        ("mode", "agent"),
        ("branch", "master"),
    ];
    for (i, (label, value)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(c(&format!(" {} ", chrome::SEP), FAINT));
        }
        spans.push(m(&format!("{label} ")));
        spans.push(t(value));
    }
    Line::from(spans)
}

/// Agent prose row indented to column 4, under the `agent_head` heading.
fn para(text: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), t(text)])
}
