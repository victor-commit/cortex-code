//! Session screen - one completed tool call (pattern 1): OK `●` on the tool
//! line, `✓` verdict on the `⎿` summary row, three actionable `file:line`
//! hits aligned in one fixed column with their open/expand keys inline on
//! the summary row (never a global footer), then a short agent paragraph
//! interpreting the result. Every figure appears exactly once, durations in
//! timefmt. The turn helpers are shared with `tool_success_expanded` so the
//! collapsed and expanded states stay byte-identical outside the elbow.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ToolStatus, BRIGHT, FAINT};

/// Code identifier styling (single treatment for the whole answer).
fn ident(s: &str) -> Span<'static> {
    c(s, BRIGHT)
}

/// The three search hits: `path:line` ref + the matched source line.
pub(crate) const HITS: [(&str, &str); 3] = [
    (
        "middleware/rate_limit.rs:42",
        "pub fn enforce(limit: Quota) -> Result<(), RateLimited> {",
    ),
    ("routes/login.rs:88", "enforce(login_quota())?;"),
    ("routes/api.rs:17", "enforce(api_quota())?;"),
];

/// Widest `path:line` ref; every hit row pads to it so refs and snippets
/// each sit in one fixed column.
pub(crate) const REF_W: usize = 27;

pub(crate) fn turn_open() -> Vec<Line<'static>> {
    vec![
        chrome::user_cmd("where do we enforce the rate limit?", Some("2m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![t(
            "Searching the server crate for every place the limit is enforced.",
        )]),
        Line::from(""),
    ]
}

pub(crate) fn tool_call() -> Line<'static> {
    chrome::tool_line(
        ToolStatus::Done,
        "Search",
        "pattern: \"rate_limit\", path: crates/server/src",
    )
}

/// The `⎿` summary row: verdict + duration + the keys that act on the hits.
pub(crate) fn summary_row(expanded: bool) -> Vec<Span<'static>> {
    let mut row = chrome::verdict(true, "3 matches across 400 files");
    row.push(m(&format!(
        " {} {}",
        chrome::SEP,
        chrome::timefmt(0.4)
    )));
    row.push(c(&format!(" {} ", chrome::SEP), FAINT));
    row.extend(chrome::key_span("enter", "open"));
    row.push(c(&format!(" {} ", chrome::SEP), FAINT));
    row.extend(chrome::key_span(
        "ctrl+o",
        if expanded { "collapse" } else { "expand" },
    ));
    row
}

pub(crate) fn closing() -> Vec<Line<'static>> {
    vec![
        para(vec![
            t("The limit is enforced in one place. "),
            ident("middleware/rate_limit.rs"),
            t(" owns "),
            ident("enforce()"),
            t(", and the login"),
        ]),
        para(vec![t(
            "and API routes both call it with their own quota. Want me to walk through it?",
        )]),
        Line::from(""),
    ]
}

/// A prose row indented to column 4, under the `agent_head` heading.
pub(crate) fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let mut rows = vec![summary_row(false)];
    for (path, snippet) in HITS {
        rows.push(vec![
            t(&format!("{path:<REF_W$}")),
            m(&format!("   {snippet}")),
        ]);
    }

    let mut lines = turn_open();
    lines.push(tool_call());
    lines.extend(chrome::elbow(rows));
    lines.push(Line::from(""));
    lines.extend(closing());

    chrome::transcript(f, body, lines);
}
