//! input/skills-picker — toggle the skills active for this session. Instead of a
//! floating overlay the searchable list DOCKS in place of the input composer
//! (cortex-code-design N-7): each skill shows its enabled state (`● on` /
//! `● off`) and the cursor rests on one row (`›` marker + bright label).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, GHOST, OK, ORANGE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        11,
        &[("\u{2191}\u{2193}", "move"), ("space", "toggle"), ("enter", "apply"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Select skill", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![search_line("filter skills", w), rule_line(w)];

    // (name, description, enabled)
    let skills: [(&str, &str, bool); 6] = [
        ("commit-identity", "Attribute commits to the linked identity", true),
        ("use-figma", "Implement Figma designs pixel-perfect", true),
        ("vision-opus", "Route image analysis to a vision model", false),
        ("firecrawl", "Search and scrape the web", true),
        ("security-review", "Parallel vulnerability audit", false),
        ("debugging", "Hypothesis-driven runtime debugging", true),
    ];
    let selected = 2usize;
    for (i, (name, desc, on)) in skills.iter().enumerate() {
        lines.push(skill_row(w, i == selected, name, desc, *on));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

const NAME_W: usize = 18;

fn skill_row(width: usize, sel: bool, name: &str, desc: &str, on: bool) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let name_span = if sel {
        cb(&format!("{name:<NAME_W$}"), BRIGHT)
    } else if on {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(TXT))
    } else {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(DIM))
    };

    let (state, state_color) = if on { ("\u{25cf} on", OK) } else { ("\u{25cf} off", FAINT) };
    let meta_len = state.chars().count();
    let used = 3 + NAME_W + 2 + desc.chars().count();
    let pad = width.saturating_sub(used + meta_len + 1);

    Line::from(vec![
        marker,
        name_span,
        m("  "),
        Span::styled(desc.to_string(), Style::default().fg(if sel { TXT } else { DIM })),
        Span::raw(" ".repeat(pad)),
        Span::styled(state.to_string(), Style::default().fg(state_color)),
    ])
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
    let f = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        f(" \u{203a} enable the vision skill for this screenshot"),
        f(""),
        f("   Loaded 4 of 6 skills for this session"),
        f("   Opening the skill picker"),
        f(""),
        f(" \u{203a} /skills"),
    ]
}
