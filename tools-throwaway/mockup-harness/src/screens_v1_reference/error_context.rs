//! Screen — states/error-context. The conversation has grown past the model's
//! context window: a plain-English message that it is too long to continue, the
//! token usage against the limit with a full progress bar, a short breakdown of
//! where the tokens went, and one clear recovery path — run /compact to shrink
//! the history (or /new to start fresh). The composer awaits that command.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, dot, m, t, ACCENT, BAD, DIM, FAINT, OK, TXT, WARN};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "keep refactoring the auth module",
        "run /compact to shrink the conversation \u{2014} or /new to start fresh\u{2026}",
        &[
            ("/compact", "compact history"),
            ("/new", "new session"),
            ("enter", "run"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("keep refactoring the auth module", "15:07:53"));
    lines.push(Line::from(""));

    // --- the limit, stated plainly ---
    lines.push(chrome::agent_head(
        "Turn stopped \u{2014} the conversation reached the model's limit",
    ));
    lines.push(Line::from(vec![
        Span::raw("   "),
        dot(BAD),
        c(" Context limit reached", BAD),
        m("  \u{2014}  "),
        t("this conversation is too long to continue"),
    ]));
    lines.push(Line::from(""));

    // --- usage vs limit, with a full bar ---
    lines.push(Line::from(vec![
        m("   tokens   "),
        c("198,842", WARN),
        m(" / "),
        t("200,000"),
        m("   "),
        c("99%", WARN),
        m("   \u{00b7}   1,158 left"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        chrome::c(&chrome::PROGRESS_FULL.repeat(40), BAD),
    ]));
    lines.push(Line::from(""));

    // --- where the tokens went ---
    lines.push(chrome::agent_head("Where the tokens are"));
    lines.push(usage("conversation history", 35, "176,400", TXT));
    lines.push(usage("system + instructions", 2, "9,800", DIM));
    lines.push(usage("tool definitions", 1, "4,600", DIM));
    lines.push(usage("this turn", 2, "8,042", DIM));
    lines.push(Line::from(""));

    // --- recovery paths ---
    lines.push(chrome::agent_head("How to keep going"));
    lines.push(recover(
        "/compact",
        OK,
        "summarize the history and keep working \u{2014} recovers about 150k tokens",
    ));
    lines.push(recover(
        "/new",
        ACCENT,
        "start a fresh session \u{2014} this conversation stays saved",
    ));
    lines.push(Line::from(""));

    // --- suggestion (hugs the composer) ---
    lines.push(chrome::agent_head(
        "Run /compact to shrink the conversation, then this turn can continue.",
    ));

    chrome::transcript(f, body, lines);
}

/// A usage row: `  label ……… mini-bar  count`, a compact per-source breakdown.
fn usage(label: &str, filled: usize, count: &str, color: ratatui::style::Color) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("   {label:<24}"),
        Style::default().fg(color),
    )];
    spans.extend(chrome::bar_spans(filled, 40));
    spans.push(Span::styled(
        format!("  {count:>7}"),
        Style::default().fg(FAINT),
    ));
    Line::from(spans)
}

/// A recovery row: `› /cmd   — what it does`, command in a status hue.
fn recover(cmd: &str, cmd_color: ratatui::style::Color, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "   \u{203a} ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{cmd:<10}"),
            Style::default().fg(cmd_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(DIM)),
    ])
}
