//! Session screen — first-run onboarding: the wordmark lockup, a short welcome,
//! then a three-step setup checklist (theme, model, account) with the current
//! step marked, earlier steps done, and later steps pending. A single primary
//! action sits below; no composer, the user walks the steps with the keyboard.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, ACCENT, BRIGHT, DIM, FAINT, OK, TITLE, TXT};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_view(
        f,
        "Welcome",
        &[
            ("enter", "next step"),
            ("\u{2191}\u{2193}", "move"),
            ("s", "skip for now"),
            ("?", "help"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Cortex Code",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        m("   v0.1.0"),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "Welcome!",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ),
        m("  Your AI-powered coding assistant."),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(m(
        "  Three quick steps and you're ready \u{2014} step 2 of 3.",
    )));
    lines.push(Line::from(""));

    lines.extend(step(
        "1",
        StepState::Done,
        "Pick a theme",
        "Mono-orange on near-black",
        "you can change this later in settings",
    ));
    lines.extend(step(
        "2",
        StepState::Current,
        "Choose a model",
        "claude-opus",
        "the default for planning, editing and review",
    ));
    lines.extend(step(
        "3",
        StepState::Pending,
        "Connect your account",
        "sign in to your provider",
        "unlocks running the agent on your projects",
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        chrome::pill(" enter "),
        c("  continue to step 3", TXT),
        m("       or press "),
        c("s", ACCENT),
        m(" to finish setup later"),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(m(
        "  You can revisit any step from Settings later.",
    )));

    f.render_widget(
        Paragraph::new(lines),
        Rect::new(body.x, body.y, body.width, body.height),
    );
}

#[derive(Clone, Copy)]
enum StepState {
    Done,
    Current,
    Pending,
}

/// A setup step rendered as two rows: a status marker + numbered title + value,
/// then a dim helper line. The marker carries the state (done / current /
/// pending) as color + glyph weight, never a decorative badge.
fn step(num: &str, state: StepState, title: &str, value: &str, help: &str) -> Vec<Line<'static>> {
    let (marker, num_color, title_color, tag, tag_color): (
        Span<'static>,
        ratatui::style::Color,
        ratatui::style::Color,
        &str,
        ratatui::style::Color,
    ) = match state {
        StepState::Done => (c(chrome::state_glyph(OK), OK), OK, TXT, "done", OK),
        StepState::Current => (
            Span::styled(
                "\u{25b6}",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            ACCENT,
            TITLE,
            "current step",
            ACCENT,
        ),
        StepState::Pending => (
            Span::styled("\u{25cb}", Style::default().fg(FAINT)),
            FAINT,
            DIM,
            "pending",
            FAINT,
        ),
    };

    let head = Line::from(vec![
        Span::raw("  "),
        marker,
        Span::styled(
            format!(" {num}  "),
            Style::default().fg(num_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{title:<22}"),
            Style::default().fg(title_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{value:<30}"), Style::default().fg(TXT)),
        Span::styled(tag.to_string(), Style::default().fg(tag_color)),
    ]);
    let helper = Line::from(vec![
        Span::raw("      "),
        Span::styled(help.to_string(), Style::default().fg(DIM)),
    ]);
    vec![head, helper, Line::from("")]
}
