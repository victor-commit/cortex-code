//! input/command-palette — a filtered list of actions matching the typed query,
//! each showing its leader-key chord. Instead of a floating overlay the palette
//! DOCKS in place of the input composer (cortex-code-design N-7); one action is
//! selected. Actions + chords mirror `crates/tui/src/keybind.rs` (leader =
//! Ctrl+G).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, ACCENT, AMBER, BRIGHT, FAINT, GHOST, ORANGE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        11,
        &[("\u{2191}\u{2193}", "move"), ("enter", "run"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Command palette", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![search_line("prompt", w), rule_line(w)];

    // Actions whose label matches the typed query "prompt".
    let actions: [(&str, &str, bool); 5] = [
        ("Recall an older prompt", "Ctrl+G H", false),
        ("Move toward the stashed prompt", "Ctrl+G L", false),
        ("Stash the current prompt draft", "Ctrl+G G", false),
        ("Restore the stashed prompt", "Ctrl+G F", false),
        ("Palette submenu: queued prompts", "Ctrl+G 8", true),
    ];
    let selected = 0usize;
    for (i, (label, chord, submenu)) in actions.iter().enumerate() {
        lines.push(action_row(w, i == selected, label, chord, *submenu));
    }
    lines.push(Line::from(""));
    lines.push(count_line(w, "5 of 28 actions"));

    f.render_widget(Paragraph::new(lines), inner);
}

fn action_row(width: usize, sel: bool, label: &str, chord: &str, submenu: bool) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let label_span = if sel {
        cb(label, BRIGHT)
    } else {
        Span::styled(label.to_string(), Style::default().fg(TXT))
    };

    let meta = if submenu { format!("{chord} \u{203a}") } else { chord.to_string() };
    let meta_len = meta.chars().count();
    let used = 3 + label.chars().count();
    let pad = width.saturating_sub(used + meta_len + 1);
    Line::from(vec![
        marker,
        label_span,
        Span::raw(" ".repeat(pad)),
        Span::styled(meta, Style::default().fg(if sel { ACCENT } else { FAINT })),
    ])
}

/// Panel search row: the `›` prompt + typed query.
fn search_line(query: &str, width: usize) -> Line<'static> {
    let used = 3 + query.chars().count();
    Line::from(vec![
        Span::styled(
            " \u{203a} ".to_string(),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(query.to_string(), Style::default().fg(BRIGHT)),
        Span::raw(" ".repeat(width.saturating_sub(used))),
    ])
}

fn rule_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "\u{2500}".repeat(width),
        Style::default().fg(GHOST),
    ))
}

/// Trailing count, right-aligned inside the card.
fn count_line(width: usize, note: &str) -> Line<'static> {
    let note_len = note.chars().count();
    let pad = width.saturating_sub(note_len + 1);
    Line::from(vec![
        Span::raw(" ".repeat(pad)),
        Span::styled(note.to_string(), Style::default().fg(FAINT)),
    ])
}

fn backdrop() -> Vec<Line<'static>> {
    let f = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        f(" \u{203a} add a rate limiter to the login route"),
        f(""),
        f("   Edited crates/server/src/auth/routes.rs \u{00b7} +214 \u{2212}37"),
        f("   Ran cargo test --all \u{00b7} 42 passed"),
        f(""),
        f(" \u{203a} "),
    ]
}
