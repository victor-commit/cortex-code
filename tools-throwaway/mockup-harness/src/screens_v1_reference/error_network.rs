//! Screen — states/error-network. A live CLI turn that failed to reach the
//! model: the user asks for a change, the agent starts the turn, and the call
//! to the API never connects. A single plain-English error line, the small set
//! of facts a user actually needs (endpoint, attempts, elapsed), and a clear
//! retry path. The composer awaits — retry, or fix the connection and continue.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, dot, m, t, BAD, DIM, INFO, MUTED, TITLE, TXT};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "implement /login rate limiter",
        "press r to retry the turn \u{2014} or check your connection and continue\u{2026}",
        &[
            ("r", "retry turn"),
            ("c", "copy error"),
            ("enter", "continue"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("implement /login rate limiter", "14:32:08"));
    lines.push(Line::from(""));

    // --- the failure, stated plainly ---
    lines.push(chrome::agent_head(
        "Turn stopped \u{2014} the request never reached the model",
    ));
    lines.push(Line::from(vec![
        Span::raw("   "),
        dot(BAD),
        c(" Network unreachable", BAD),
        m("  \u{2014}  "),
        t("could not connect to the model endpoint"),
    ]));
    lines.push(Line::from(""));

    // --- just the facts a user needs (nothing more) ---
    lines.push(fact("endpoint", "api.anthropic.com", TXT));
    lines.push(fact("model", "claude-opus", TXT));
    lines.push(Line::from(vec![
        m("   attempts   "),
        c("3 of 3", BAD),
        m("   \u{00b7}   backoff 2s \u{2192} 4s \u{2192} 8s"),
    ]));
    lines.push(fact("reason", "connection timed out after 8s", DIM));
    lines.push(fact("your work", "saved \u{00b7} nothing was lost", DIM));
    lines.push(Line::from(""));

    // --- what happens next ---
    lines.push(chrome::agent_head("What you can do"));
    lines.push(step("retry the turn", "press r to send the same request again", INFO));
    lines.push(step(
        "check your connection",
        "then press enter to continue where you left off",
        INFO,
    ));
    lines.push(Line::from(""));

    // --- retry hint (hugs the composer) ---
    lines.push(chrome::agent_head(
        "Ready to retry \u{2014} press r to reconnect and finish this turn.",
    ));

    chrome::transcript(f, body, lines);
}

/// A single `  label   value` fact row, dim label, on the canvas.
fn fact(label: &str, value: &str, value_color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {label:<10} "), Style::default().fg(MUTED)),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ])
}

/// A next-step row: `› action  — description`, arrow in the accent hue.
fn step(action: &str, desc: &str, accent: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "   \u{203a} ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{action:<24}"),
            Style::default().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(DIM)),
    ])
}
