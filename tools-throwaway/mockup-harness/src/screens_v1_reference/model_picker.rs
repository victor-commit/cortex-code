//! input/model-picker — pick the model that handles the next turn. Instead of a
//! floating overlay the searchable list DOCKS in place of the input composer
//! (cortex-code-design N-7): the active model is marked `current`, the cursor
//! rests on a different row (`›` marker + bright label).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, GHOST, OK, ORANGE, TITLE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        11,
        &[("\u{2191}\u{2193}", "move"), ("enter", "select"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Select model", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![search_line("filter models", w), rule_line(w)];

    let models: [(&str, &str, &str, bool); 5] = [
        ("claude-opus-4", "Anthropic", "200K", true),
        ("claude-sonnet-4", "Anthropic", "200K", false),
        ("gpt-5", "OpenAI", "272K", false),
        ("gemini-2.5-pro", "Google", "1M", false),
        ("o3", "OpenAI", "200K", false),
    ];
    let selected = 1usize;
    for (i, (name, provider, ctx, current)) in models.iter().enumerate() {
        lines.push(model_row(w, i == selected, name, provider, ctx, *current));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

const NAME_W: usize = 18;

fn model_row(
    width: usize,
    sel: bool,
    name: &str,
    provider: &str,
    ctx: &str,
    current: bool,
) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let name_span = if sel {
        cb(&format!("{name:<NAME_W$}"), BRIGHT)
    } else if current {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(TITLE))
    } else {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(TXT))
    };

    let ctx_text = format!("{ctx:>4} ctx");
    let badge = if current { "  \u{25cf} current" } else { "" };
    let meta_len = ctx_text.chars().count() + badge.chars().count();
    let used = 3 + NAME_W + 2 + provider.chars().count();
    let pad = width.saturating_sub(used + meta_len + 1);

    let mut spans = vec![
        marker,
        name_span,
        m("  "),
        Span::styled(provider.to_string(), Style::default().fg(DIM)),
        Span::raw(" ".repeat(pad)),
        Span::styled(ctx_text, Style::default().fg(if sel { ACCENT } else { FAINT })),
    ];
    if current {
        spans.push(Span::styled(badge.to_string(), Style::default().fg(OK)));
    }
    Line::from(spans)
}

fn search_line(placeholder: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            " \u{203a} ".to_string(),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(placeholder.to_string(), Style::default().fg(FAINT)),
        Span::raw(" ".repeat(width.saturating_sub(3 + placeholder.chars().count()))),
    ])
}

fn rule_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "\u{2500}".repeat(width),
        Style::default().fg(GHOST),
    ))
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        chrome::user_cmd("which model handles this refactor best?", "14:22:07"),
        Line::from(""),
        dim("   Reviewing the change set across 6 files"),
        dim("   Pick a model below — it applies from the next turn"),
    ]
}
