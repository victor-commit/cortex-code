//! input/help — the `/help` view as a CLI turn: the user asks what they can do,
//! the agent prints the keybindings grouped by purpose with aligned `key  action`
//! columns. The leader is Ctrl+G (`crates/tui/src/keybind.rs`).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, AMBER, DIM, ORANGE, TXT};

const COLS: [usize; 3] = [8, 11, 27];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/help",
        "type a command with / \u{2014} or ask a question in plain English\u{2026}",
        &[("/", "commands"), ("ctrl+g", "leader"), ("q", "quit")],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/help", "15:04:19"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head(
        "Keybindings \u{2014} leader is ctrl+g, then a key",
    ));
    lines.push(Line::from(""));

    lines.extend(bind_table());

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
    let labels = ["SCOPE", "KEY", "ACTION"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn bind_row(scope: &str, key: &str, action: &str) -> Line<'static> {
    let scope_span = Span::styled(
        format!("{scope:<w$}", w = COLS[0]),
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
    );
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![scope_span]));
    spans.extend(cell(vec![c(&format!("{key:<w$}", w = COLS[1]), ORANGE)]));
    spans.extend(cell(vec![c(&format!("{action:<w$}", w = COLS[2]), TXT)]));
    Line::from(spans)
}

fn bind_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        bind_row("Session", "ctrl+g n", "start a new session"),
        bind_row("", "ctrl+g s", "open the session picker"),
        bind_row("", "ctrl+g f", "fork from the current turn"),
        bind_row("", "ctrl+c", "interrupt the running turn"),
        bind_row("Views", "ctrl+g m", "switch model"),
        bind_row("", "ctrl+g a", "switch agent"),
        bind_row("", "ctrl+g d", "show the pending diff"),
        bind_row("", "ctrl+g t", "show the session timeline"),
        bind_row("Composer", "/", "open the slash command menu"),
        bind_row("", "ctrl+p", "open the command palette"),
        bind_row("", "enter", "send the message"),
        bind_row("", "shift+enter", "insert a newline"),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
