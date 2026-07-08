//! input/slash-menu — the `/` command menu. Instead of a floating overlay the
//! list DOCKS in place of the input composer (cortex-code-design N-7): the
//! builtin slash commands (name + one-line description) sit below the search
//! line with one row selected (`›` marker + bright label). Command
//! names/descriptions are the real BUILTINS from `crates/tui/src/slash.rs`.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{
    self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, GHOST, ORANGE, TXT,
};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "Session",
        19,
        &[("\u{2191}\u{2193}", "move"), ("enter", "run"), ("esc", "close")],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Commands", AMBER, &[]);
    let w = inner.width as usize;

    let mut lines: Vec<Line> = vec![search_line("", "type to filter commands", w), rule_line(w)];

    let cmds: [(&str, &str); 15] = [
        ("/help", "Show the command reference and keybindings"),
        ("/new", "Start a fresh session"),
        ("/sessions", "Open the session picker"),
        ("/models", "Open the model picker"),
        ("/agents", "Open the agent picker"),
        ("/skills", "Open the skill picker"),
        ("/status", "Open the status view"),
        ("/timeline", "Open the session timeline"),
        ("/review", "Run a code review"),
        ("/diff", "Open the diff viewer"),
        ("/compact", "Compact the conversation"),
        ("/mcp", "Open the MCP status view"),
        ("/init", "Initialize the project (AGENTS.md)"),
        ("/export", "Export the current session"),
        ("/exit", "Exit Cortex Code"),
    ];
    let selected = 3usize;
    for (i, (name, desc)) in cmds.iter().enumerate() {
        lines.push(cmd_row(w, i == selected, name, desc));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

const KEY_W: usize = 12;

fn cmd_row(width: usize, sel: bool, name: &str, desc: &str) -> Line<'static> {
    let marker = if sel { cb(" \u{203a} ", ACCENT) } else { Span::raw("   ") };
    let key_span = if sel {
        cb(&format!("{name:<KEY_W$}"), BRIGHT)
    } else {
        Span::styled(format!("{name:<KEY_W$}"), Style::default().fg(ACCENT))
    };
    let desc_style = Style::default().fg(if sel { TXT } else { DIM });

    let used = 3 + KEY_W + 2 + desc.chars().count();
    let pad = width.saturating_sub(used);
    Line::from(vec![
        marker,
        key_span,
        m("  "),
        Span::styled(desc.to_string(), desc_style),
        Span::raw(" ".repeat(pad)),
    ])
}

/// Panel search row: the `/` prompt + typed query (or a faint placeholder).
fn search_line(query: &str, placeholder: &str, width: usize) -> Line<'static> {
    let mut spans = vec![Span::styled(
        " / ".to_string(),
        Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
    )];
    if query.is_empty() {
        spans.push(Span::styled(placeholder.to_string(), Style::default().fg(FAINT)));
    } else {
        spans.push(Span::styled(query.to_string(), Style::default().fg(BRIGHT)));
    }
    let used = 3 + if query.is_empty() { placeholder.chars().count() } else { query.chars().count() };
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    Line::from(spans)
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
        f(" \u{203a} refactor the auth middleware to add rate limiting"),
        f(""),
        f("   Edited crates/server/src/auth/routes.rs \u{00b7} +214 \u{2212}37"),
        f("   Ran cargo test --all \u{00b7} 42 passed"),
        f("   Rate limit held at the sixth attempt \u{00b7} 429 returned"),
        f(""),
        f(" \u{203a} /"),
    ]
}
