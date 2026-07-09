//! Full-screen MCP server inventory with an actionable failed connection.

use ratatui::{
    style::Color,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, BRIGHT, FAINT, OK, TXT};

const NAME_W: usize = 18;
const STATE_W: usize = 12;
const TRANSPORT_W: usize = 11;
const TOOLS_W: usize = 8;
const ATTEMPT_W: usize = 14;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "MCP · 4 configured · 3 connected · 1 failed", &status);

    let lines = vec![
        Line::from(header_keys()),
        Line::from(""),
        columns(),
        chrome::rule_line(96),
        server_row(
            true,
            "playwright",
            "✗ failed",
            BAD,
            "stdio",
            "0",
            "14:38",
            "browser automation",
        ),
        failure_note("playwright binary not found on PATH"),
        fix_command(),
        Line::from(""),
        reconnect_hint(),
        Line::from(""),
        server_row(
            false,
            "context7",
            "✓ connected",
            OK,
            "http",
            "2",
            "14:40",
            "resolve-library-id, query-docs",
        ),
        server_row(
            false,
            "github",
            "✓ connected",
            OK,
            "http",
            "5",
            "14:41",
            "issues, pull requests, checks",
        ),
        server_row(
            false,
            "filesystem",
            "✓ connected",
            OK,
            "stdio",
            "12",
            "14:42",
            "read, write, search",
        ),
        Line::from(""),
        chrome::rule_line(96),
        Line::from(vec![
            Span::raw("   "),
            m("total tools  "),
            t("19"),
            m(" available across connected servers"),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("enter", "details"));
    spans.push(c(" · ", FAINT));
    spans.extend(chrome::key_span("q", "quit"));
    spans
}

fn columns() -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        m(&format!("{:<NAME_W$}", "server")),
        m(&format!("{:<STATE_W$}", "state")),
        m(&format!("{:<TRANSPORT_W$}", "transport")),
        m(&format!("{:<TOOLS_W$}", "tools")),
        m(&format!("{:<ATTEMPT_W$}", "last attempt")),
        m("exposes"),
    ])
}

#[allow(clippy::too_many_arguments)]
fn server_row(
    failed: bool,
    name: &str,
    state: &str,
    state_color: Color,
    transport: &str,
    tools: &str,
    attempt: &str,
    exposes: &str,
) -> Line<'static> {
    let (glyph, state_word) = state
        .split_once(' ')
        .expect("state must contain a verdict glyph and word");
    let marker = if failed { c("  │ ", BAD) } else { t("    ") };
    Line::from(vec![
        marker,
        c(&format!("{name:<NAME_W$}"), if failed { BRIGHT } else { TXT }),
        c(glyph, state_color),
        t(&format!(
            " {state_word:<width$}",
            width = STATE_W - 2
        )),
        m(&format!("{transport:<TRANSPORT_W$}")),
        t(&format!("{tools:<TOOLS_W$}")),
        m(&format!("{attempt:<ATTEMPT_W$}")),
        m(exposes),
    ])
}

fn failure_note(note: &str) -> Line<'static> {
    Line::from(vec![c("  │   ✗ ", BAD), t(note)])
}

fn fix_command() -> Line<'static> {
    Line::from(vec![
        c("  │   ", BAD),
        m("fix: "),
        t("npm i -g @playwright/mcp"),
    ])
}

fn reconnect_hint() -> Line<'static> {
    let mut spans = vec![Span::raw("     ")];
    spans.extend(chrome::key_span("r", "reconnect"));
    Line::from(spans)
}

