//! input/agent-picker — pick the agent/mode for the next turn. Instead of a
//! floating overlay the list DOCKS in place of the input composer
//! (cortex-code-design N-7): the active agent is marked `current`, the cursor
//! rests on a different row (`›` marker + bright label).

use ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, OK, TITLE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        9,
        &[("\u{2191}\u{2193}", "move"), ("enter", "select"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Select agent", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();

    // (name, description, current)
    let agents: [(&str, &str, bool); 6] = [
        ("Build", "Read, write, run, and spawn subagents", true),
        ("Plan", "Read-only planning and inspection", false),
        ("Chat", "Interactive question and answer", false),
        ("Explore", "Fast read-only codebase search", false),
        ("Oracle", "Deep reasoning and architecture review", false),
        ("Review", "Code review and QA verification gate", false),
    ];
    let selected = 1usize;
    for (i, (name, desc, current)) in agents.iter().enumerate() {
        lines.push(agent_row(w, i == selected, name, desc, *current));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

const NAME_W: usize = 10;

fn agent_row(width: usize, sel: bool, name: &str, desc: &str, current: bool) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let name_span = if sel {
        cb(&format!("{name:<NAME_W$}"), BRIGHT)
    } else if current {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(TITLE))
    } else {
        Span::styled(format!("{name:<NAME_W$}"), Style::default().fg(TXT))
    };

    let badge = if current { "\u{25cf} current" } else { "" };
    let meta_len = badge.chars().count();
    let used = 3 + NAME_W + 2 + desc.chars().count();
    let pad = width.saturating_sub(used + meta_len + 1);

    let mut spans = vec![
        marker,
        name_span,
        m("  "),
        Span::styled(desc.to_string(), Style::default().fg(if sel { TXT } else { DIM })),
        Span::raw(" ".repeat(pad)),
    ];
    if current {
        spans.push(Span::styled(badge.to_string(), Style::default().fg(OK)));
    }
    Line::from(spans)
}

fn backdrop() -> Vec<Line<'static>> {
    let f = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        f(" \u{203a} switch to a read-only agent before I inspect prod"),
        f(""),
        f("   Current agent grants write and run"),
        f("   Opening the agent picker"),
        f(""),
        f(" \u{203a} /agents"),
    ]
}
