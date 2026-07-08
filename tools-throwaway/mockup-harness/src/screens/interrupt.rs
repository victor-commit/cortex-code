//! Screen — states/interrupt. The user pressed esc mid-stream and stopped the
//! agent in the middle of its reply. The partial output stays on screen exactly
//! as far as it got, followed by a clear "Response interrupted" marker and a
//! short line on what to do next — continue from where it stopped, or type a new
//! instruction. Nothing is lost; the partial answer is folded into the history.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, dot, m, t, DIM, FAINT, INFO, TITLE, TXT, WARN};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "explain how the rate limiter works",
        "press enter to continue the response \u{2014} or type a new instruction\u{2026}",
        &[
            ("enter", "continue"),
            ("r", "rephrase"),
            ("esc", "dismiss"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("explain how the rate limiter works", "17:44:31"));
    lines.push(Line::from(""));

    // --- the partial reply, exactly as far as it streamed ---
    lines.push(chrome::agent_head("How the rate limiter works"));
    lines.push(body_line(
        "The limiter tracks each caller in a fixed one-minute window and counts",
    ));
    lines.push(body_line(
        "requests as they arrive. When a caller crosses the cap the middleware",
    ));
    lines.push(body_line(
        "short-circuits the handler and returns 429 with a Retry-After header,",
    ));
    lines.push(body_line("so the client knows exactly when it can try again. The counter"));
    // Interruption point: the stream stops here, mid-sentence.
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled("resets when the window rolls over and the next request", Style::default().fg(TXT)),
        Span::styled(" \u{2588}", Style::default().fg(FAINT)),
    ]));
    lines.push(Line::from(""));

    // --- the interruption marker ---
    lines.push(Line::from(vec![
        Span::raw("   "),
        dot(WARN),
        c(" Response interrupted", WARN),
        m("  \u{2014}  "),
        t("you stopped the reply at 17:44:38"),
    ]));
    lines.push(Line::from(vec![
        m("   kept   "),
        t("the partial answer is saved to this conversation"),
    ]));
    lines.push(Line::from(""));

    // --- what happens next ---
    lines.push(chrome::agent_head("What you can do"));
    lines.push(step("continue", "press enter to finish the reply from where it stopped", INFO));
    lines.push(step("rephrase", "press r to ask the question a different way", INFO));
    lines.push(Line::from(""));

    // --- hint (hugs the composer) ---
    lines.push(chrome::agent_head(
        "Press enter to continue, or type a new instruction below.",
    ));

    chrome::transcript(f, body, lines);
}

/// A partial-reply body line, indented to the 3-column agent body gutter.
fn body_line(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled(text.to_string(), Style::default().fg(TXT)),
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
            format!("{action:<12}"),
            Style::default().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc.to_string(), Style::default().fg(DIM)),
    ])
}
