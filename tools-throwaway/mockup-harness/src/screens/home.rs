//! Session screen - the landing view right after the first exchange: the
//! ASCII "Code" banner (rule F-6), version + greeting and one session-meta
//! row above a faint rule, the first turn beneath it. No global hint row
//! (rule N-9): the composer placeholder carries the screen's one inline hint.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, FAINT};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut lines: Vec<Line> = chrome::banner();
    lines.extend(vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Welcome back."),
            m("  v0.3.0"),
        ]),
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
    ]);

    chrome::transcript(f, body, lines);
}

/// One-line session meta panel at the column-4 text edge: muted labels,
/// body-text values, faint dots. Kept as-is per the M1 gate; the model is a
/// display name (rule N-10/F-7).
fn meta_row() -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("   ")];
    let pairs = [
        ("directory", "~/projects/cortex-code"),
        ("model", "Claude Opus 4.8"),
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
