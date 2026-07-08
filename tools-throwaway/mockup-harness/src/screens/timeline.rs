//! input/timeline — the `/timeline` view as a CLI turn: the user asks for the
//! session history, the agent prints each turn oldest-first with a status dot,
//! the actor, a one-line summary and a timestamp. Newest turn sits at the bottom.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, BAD, BRIGHT, DIM, OK, TXT, WARN};

const COLS: [usize; 3] = [5, 8, 43];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/timeline",
        "press f to fork from a turn \u{2014} or type a follow-up\u{2026}",
        &[("f", "fork"), ("\u{2191}\u{2193}", "select"), ("q", "quit")],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/timeline", "14:58:02"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head("Session timeline \u{2014} 6 turns \u{00b7} wf-42"));
    lines.push(Line::from(""));

    lines.extend(turn_table());

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
    let labels = ["WHEN", "ACTOR", "SUMMARY"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn turn_row(
    color: ratatui::style::Color,
    actor: &str,
    summary: &str,
    time: &str,
    current: bool,
) -> Line<'static> {
    let actor_style = if current {
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TXT)
    };
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{time:<w$}", w = COLS[0]), DIM)]));
    spans.extend(cell(vec![
        c(&format!("{} ", chrome::state_glyph(color)), color),
        Span::styled(format!("{actor:<w$}", w = COLS[1] - 2), actor_style),
    ]));
    spans.extend(cell(vec![c(
        &format!("{summary:<w$}", w = COLS[2]),
        if current { TXT } else { DIM },
    )]));
    Line::from(spans)
}

fn turn_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        turn_row(OK, "you", "asked to add per-IP rate limiting to /login", "14:02", false),
        turn_row(OK, "Build", "planned the change and edited routes.rs", "14:05", false),
        turn_row(OK, "you", "approved the shell command", "14:41", false),
        turn_row(OK, "Build", "ran the suite \u{2014} 42 passed, 0 failed", "14:47", false),
        turn_row(BAD, "Review", "changes requested \u{2014} 4 findings", "14:52", false),
        turn_row(WARN, "Build", "addressing the reviewer findings now", "14:58", true),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
