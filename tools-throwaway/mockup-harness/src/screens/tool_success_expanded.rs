//! Session screen - the tool-success state after `ctrl+o`: the same turn,
//! tool line and `⎿` verdict row (shared with `tool_success`), but the hit
//! list expands into surrounding code context per hit - `file:line` heads
//! stay in the collapsed state's ref column, FAINT line-number gutters and
//! MUTED context code indent under the elbow, the matched line itself in
//! bold TXT (one emphasis channel). No border around the expansion; the
//! summary row's key hint flips to `ctrl+o collapse`.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, cb, m, StatusLine, FAINT, TXT};

use super::tool_success;

/// One context row under a hit head: `  <num>  <code>`, gutter FAINT, the
/// matched line's code in bold TXT, surrounding lines in MUTED.
fn ctx(num: u32, code: &str, matched: bool) -> Vec<Span<'static>> {
    vec![
        c(&format!("  {num:>3}  "), FAINT),
        if matched { cb(code, TXT) } else { m(code) },
    ]
}

/// A hit head row: the `file:line` ref alone, in the collapsed state's
/// ref column and styling.
fn hit_head(path: &str) -> Vec<Span<'static>> {
    vec![c(path, TXT)]
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    let (rate_limit, login, api) = (
        tool_success::HITS[0],
        tool_success::HITS[1],
        tool_success::HITS[2],
    );
    let rows = vec![
        tool_success::summary_row(true),
        hit_head(rate_limit.0),
        ctx(40, "use crate::quota::{Quota, RateLimited};", false),
        ctx(41, "", false),
        ctx(42, rate_limit.1, true),
        ctx(43, "    let key = client_key(&request)?;", false),
        vec![],
        hit_head(login.0),
        ctx(87, "let attempt = LoginAttempt::from(&form);", false),
        ctx(88, login.1, true),
        vec![],
        hit_head(api.0),
        ctx(16, "// every api route shares one quota", false),
        ctx(17, api.1, true),
    ];

    let mut lines = tool_success::turn_open();
    lines.push(tool_success::tool_call());
    lines.extend(chrome::elbow(rows));
    lines.push(Line::from(""));
    lines.extend(tool_success::closing());

    chrome::transcript(f, body, lines);
}
