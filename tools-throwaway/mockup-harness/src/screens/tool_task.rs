//! Session screen for delegated subagent tasks. One task is complete and one
//! is still running, so elapsed time belongs on the running task line only.

use ratatui::{
    Frame,
    text::{Line, Span},
};

use crate::chrome::{self, FAINT, LiveActivityInterrupt, StatusLine, ToolStatus, c, m, t};

/// The delegated turn shared by the collapsed and expanded task states.
pub(crate) fn turn_open() -> Vec<Line<'static>> {
    vec![
        chrome::user_cmd(
            "audit the auth module and add the rate-limit tests",
            Some("5m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![t(
            "I split the audit from the test work so the findings can guide the patch.",
        )]),
        Line::from(""),
    ]
}

/// The completed task remains a standard tool line with the delegated goal in
/// its argument string.
pub(crate) fn completed_task() -> Line<'static> {
    chrome::tool_line(ToolStatus::Done, "Task", "explore: audit the auth module")
}

/// The completed task's summary row. Its duration comes from `timefmt`, so a
/// future value cannot drift into the invalid bare-seconds form.
pub(crate) fn completed_summary(expanded: bool) -> Vec<Span<'static>> {
    let mut row = chrome::verdict(true, "6 findings");
    row.push(m(&format!(" {} {}", chrome::SEP, chrome::timefmt(192.0))));
    row.push(c(&format!(" {} ", chrome::SEP), FAINT));
    row.extend(chrome::key_span(
        "ctrl+o",
        if expanded { "collapse" } else { "expand" },
    ));
    row
}

/// A prose row aligned to the transcript text edge.
pub(crate) fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "type a follow-up, it runs after this turn\u{2026}",
        &status,
    );

    let mut lines = turn_open();
    lines.push(completed_task());
    lines.extend(chrome::elbow(vec![completed_summary(false)]));
    lines.push(Line::from(""));
    lines.push(chrome::tool_line(
        ToolStatus::Running(Some(48.0), LiveActivityInterrupt::Inline),
        "Task",
        "build: add rate-limit tests",
    ));
    lines.extend(chrome::elbow(vec![vec![m(
        "2 of 3 files \u{00b7} tests/login_ratelimit.rs \u{00b7} esc to interrupt",
    )]]));

    chrome::transcript(f, body, lines);
}
