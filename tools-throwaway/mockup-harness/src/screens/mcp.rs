//! input/mcp — the `/mcp` view as a CLI turn: the user asks which MCP servers
//! are wired in, the agent lists each server with a connection dot (connected
//! green, failed red), its transport, and how many tools it exposes. Mirrors
//! `crates/mcp/src/manager.rs` ConnectionState.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, BAD, BRIGHT, DIM, FAINT, OK, TXT, WARN};

const COLS: [usize; 5] = [11, 10, 9, 8, 30];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/mcp",
        "press r to reconnect a failed server \u{2014} or type a follow-up\u{2026}",
        &[("r", "reconnect"), ("enter", "details"), ("q", "quit")],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/mcp", "15:01:44"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head(
        "MCP servers \u{2014} 4 configured \u{00b7} 3 connected \u{00b7} 1 failed",
    ));
    lines.push(Line::from(""));

    lines.extend(server_table());

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        m("   total  "),
        c("23", TXT),
        m(" tools available     "),
        c("\u{2717} 1 server needs attention", WARN),
    ]));

    chrome::transcript(f, body, lines);
}

fn cell(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(chrome::vsep());
    out
}

fn header_row() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["STATE", "SERVER", "TRANSPORT", "TOOLS", "NOTE"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn server_row(
    color: ratatui::style::Color,
    state: &str,
    name: &str,
    transport: &str,
    tools: &str,
    note: &str,
) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(
        &format!("{:<w$}", format!("{} {state}", chrome::state_glyph(color)), w = COLS[0]),
        color,
    )]));
    spans.extend(cell(vec![Span::styled(
        format!("{name:<w$}", w = COLS[1]),
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
    )]));
    spans.extend(cell(vec![c(&format!("{transport:<w$}", w = COLS[2]), DIM)]));
    spans.extend(cell(vec![c(&format!("{tools:<w$}", w = COLS[3]), TXT)]));
    spans.extend(cell(vec![c(&format!("{note:<w$}", w = COLS[4]), FAINT)]));
    Line::from(spans)
}

fn server_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        server_row(OK, "connected", "filesystem", "stdio", "12 tools", ""),
        server_row(OK, "connected", "git", "stdio", "8 tools", ""),
        server_row(OK, "connected", "context7", "http", "3 tools", ""),
        server_row(
            BAD,
            "failed",
            "playwright",
            "stdio",
            "0 tools",
            "spawn error: command not found",
        ),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
