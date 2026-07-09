//! Fixed-shape tool-response envelope with selectable outcome counters.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, OK, WARN};

const FIELD_W: usize = 14;
const TYPE_W: usize = 27;
const JSON_W: usize = 48;
const STATUS_W: usize = 13;
const ACTION_W: usize = 17;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "tool envelope · session", &status);
    let mut lines = vec![
        filter_counters(),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("response contract")]),
        chrome::rule_line(105),
        contract_head(),
        contract_row("status", "ok | error | refused", "read outcome first"),
        contract_row("payload", "object", "result or reason"),
        contract_row("truncated", "bool", "true when output was clipped"),
        Line::from(vec![
            Span::raw("   "),
            m("fixed shape  ·  every reply carries all three fields"),
        ]),
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("example payloads")]),
        chrome::rule_line(105),
        example_labels(),
    ];
    lines.extend(json_rows());
    lines.extend([
        Line::from(""),
        Line::from(vec![Span::raw("   "), m("recent tool calls")]),
        chrome::rule_line(105),
        contrast_line(),
        calls_head(),
        call_row("10:24", "✓", "ok", OK, "Read", "routes.rs:12-48"),
        call_row("10:23", "○", "refused", WARN, "Edit", "policy requires project read"),
        call_row("10:23", "✗", "error", BAD, "Web lookup", "request timed out after 10s"),
        call_row("10:22", "✓", "ok", OK, "Run shell", "48 checks passed · clipped"),
        call_row("10:22", "✓", "ok", OK, "Search", "rate limit · 3 matches"),
    ]);

    f.render_widget(Paragraph::new(lines), body);
}

fn filter_counters() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("filters  "),
        c("✓ [o] ok 41", OK),
        c("  ·  ", FAINT),
        c("✗ [e] error 2", BAD),
        c("  ·  ", FAINT),
        c("○ [r] refused 3", WARN),
    ])
}

fn contract_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{:<FIELD_W$}", "FIELD")),
        m(&format!("{:<TYPE_W$}", "TYPE")),
        m("DETAIL"),
    ])
}

fn contract_row(field: &str, ty: &str, detail: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        t(&format!("{field:<FIELD_W$}")),
        t(&format!("{ty:<TYPE_W$}")),
        m(detail),
    ])
}

fn example_labels() -> Line<'static> {
    let left_label = "✓ ok  completed call";
    let right_pad = " ".repeat(JSON_W.saturating_sub(left_label.chars().count()));
    Line::from(vec![
        Span::raw("   "),
        c("✓ ok", OK),
        m("  completed call"),
        Span::raw(right_pad),
        c("○ refused", WARN),
        m("  never ran"),
    ])
}

fn json_rows() -> Vec<Line<'static>> {
    [
        ("{", "{"),
        ("  \"status\": \"ok\",", "  \"status\": \"refused\","),
        ("  \"payload\": {", "  \"payload\": {"),
        ("    \"result\": \"3 matches\"", "    \"reason\": \"read the project first\""),
        ("  },", "  },"),
        ("  \"truncated\": false", "  \"truncated\": false"),
        ("}", "}"),
    ]
    .into_iter()
    .map(|(ok, refused)| json_pair(ok, refused))
    .collect()
}

fn json_pair(ok: &str, refused: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        t(&format!("{ok:<JSON_W$}")),
        t(refused),
    ])
}

fn contrast_line() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("error", BAD),
        m(" = ran and failed  ·  "),
        c("refused", WARN),
        m(" = never ran"),
    ])
}

fn calls_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m("TIME    "),
        m(&format!("{:<STATUS_W$}", "STATUS")),
        m(&format!("{:<ACTION_W$}", "ACTION")),
        m("DETAIL"),
    ])
}

fn call_row(
    time: &str,
    glyph: &str,
    outcome: &str,
    hue: ratatui::style::Color,
    action: &str,
    detail: &str,
) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{time:<8}")),
        c(&format!("{glyph} {outcome:<11}"), hue),
        t(&format!("{action:<ACTION_W$}")),
        m(detail),
    ])
}
