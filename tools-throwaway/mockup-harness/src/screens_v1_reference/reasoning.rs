//! Session screen — a reasoning trace shown ahead of the answer: a dim
//! "Thinking" block (a quote-barred list of the steps the agent worked through)
//! followed by the opening of the actual reply. The thinking is de-emphasised so
//! the eye lands on the answer; the composer waits below.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, t, ACCENT, BRIGHT, DIM, FAINT, GHOST, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("r", "hide reasoning"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "why is /login returning 500 for some users after the deploy?",
        "14:31:08",
    ));
    lines.push(Line::from(""));

    // --- thinking block (dim, quote-barred, de-emphasised) ---
    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled(
            "Thinking",
            Style::default().fg(FAINT).add_modifier(Modifier::ITALIC),
        ),
        Span::styled("  8s", Style::default().fg(GHOST)),
    ]));
    lines.push(think("The 500s started right after the deploy, so I'll suspect the migration or a config change."));
    lines.push(think("The new sessions table added a not-null token_version column with no default value."));
    lines.push(think("Existing rows were backfilled, but sessions created before the deploy have it as null."));
    lines.push(think("When those users hit /login the token refresh reads token_version and the query panics."));
    lines.push(think("That matches the pattern: only returning users fail, brand-new sign-ins succeed."));
    lines.push(Line::from(""));

    // --- answer start ---
    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("The 500 comes from stale session rows, not the login handler itself. The deploy added a"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("non-null "),
        c("token_version", TITLE),
        t(" column, but sessions created before the migration still have it set to "),
        c("null", ACCENT),
        t(", so"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("the token-refresh query fails for those users. Two fixes worth applying together:"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        c("1  ", ACCENT),
        Span::styled(
            "Backfill",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        t("  set token_version = 1 for every row where it is currently null."),
    ]));

    chrome::transcript(f, body, lines);
}

/// A single thinking step: a faint quote bar plus dim text, so the whole trace
/// reads as background context rather than the answer.
fn think(step: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("   \u{2502} ", Style::default().fg(GHOST)),
        Span::styled(step.to_string(), Style::default().fg(DIM)),
    ])
}
