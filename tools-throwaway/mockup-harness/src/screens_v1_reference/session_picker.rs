//! input/session-picker — resume a past session. Instead of a floating overlay
//! the searchable list DOCKS in place of the input composer (cortex-code-design
//! N-7): each row shows the title, relative age and message count, and the
//! cursor rests on one row (`›` marker + bright label).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, ACCENT, AMBER, BRIGHT, DIM, FAINT, GHOST, ORANGE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        11,
        &[("\u{2191}\u{2193}", "move"), ("enter", "resume"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Resume session", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![search_line("filter sessions", w), rule_line(w)];

    // (title, relative age, message count)
    let sessions: [(&str, &str, &str); 6] = [
        ("Refactor auth middleware", "2h ago", "48 msgs"),
        ("Fix flaky rate-limit test", "5h ago", "22 msgs"),
        ("Add MCP server config", "yesterday", "91 msgs"),
        ("Migrate sessions table to v3", "2d ago", "37 msgs"),
        ("Wire session revocation endpoint", "4d ago", "63 msgs"),
        ("Scaffold auth crate", "1w ago", "15 msgs"),
    ];
    let selected = 0usize;
    for (i, (title, age, count)) in sessions.iter().enumerate() {
        lines.push(session_row(w, i == selected, title, age, count));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

const TITLE_W: usize = 34;
const AGE_W: usize = 11;

fn session_row(width: usize, sel: bool, title: &str, age: &str, count: &str) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let title_span = if sel {
        cb(&format!("{title:<TITLE_W$}"), BRIGHT)
    } else {
        Span::styled(format!("{title:<TITLE_W$}"), Style::default().fg(TXT))
    };

    let count_text = format!("{count:>8}");
    let meta_len = AGE_W + count_text.chars().count();
    let used = 3 + TITLE_W;
    let pad = width.saturating_sub(used + meta_len + 1);

    Line::from(vec![
        marker,
        title_span,
        Span::raw(" ".repeat(pad)),
        Span::styled(format!("{age:<AGE_W$}"), Style::default().fg(DIM)),
        Span::styled(count_text, Style::default().fg(FAINT)),
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
        f(" \u{203a} resume where I left off yesterday"),
        f(""),
        f("   6 saved sessions in this project"),
        f("   Opening the session picker"),
        f(""),
        f(" \u{203a} /sessions"),
    ]
}
