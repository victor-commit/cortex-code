//! Live child-subagent picker dock for `/subagents`.

use ratatui::{Frame, text::Line, widgets::Paragraph};

use crate::chrome::{self, AnnotatedListRow, StatusLine};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 11, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd(
                "check the authentication boundary before changing it",
                Some("2m ago"),
            ),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   Three child agents are helping with this investigation."),
            Line::from(""),
            chrome::user_cmd("/subagents", None),
        ],
    );

    let rows = [
        AnnotatedListRow {
            name: "explore",
            detail: "find existing auth middleware",
            meta: chrome::inline_live_state("running", 12.0),
            meta_color: chrome::MUTED,
            disabled: false,
        },
        AnnotatedListRow {
            name: "general",
            detail: "summarize redaction audit",
            meta: format!("✓ done {}", chrome::timefmt(64.0)),
            meta_color: chrome::OK,
            disabled: false,
        },
        AnnotatedListRow {
            name: "plan",
            detail: "design token-refresh flow",
            meta: "○ queued".to_owned(),
            meta_color: chrome::FAINT,
            disabled: false,
        },
    ];
    let inner = chrome::card(
        f,
        dock,
        "Subagents",
        &[
            ("↑↓", "move"),
            ("enter", "view"),
            ("ctrl+c", "cancel"),
            ("esc", "close"),
        ],
    );
    let lines = chrome::annotated_select_list(inner.width as usize, &rows, 0, None);
    f.render_widget(Paragraph::new(lines), inner);
}
