use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, FAINT};

const MODES: &[(&str, &str, &str)] = &[
    ("Plan", "read-only investigation", "8 tools"),
    ("Edit", "read, write, and run", "13 tools · current"),
    ("Agent", "autonomous work with subagents", "15 tools"),
    ("Chat", "interactive conversation", "6 tools"),
    ("Headless", "non-interactive single shot", "10 tools"),
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 10, &status);

    let mut recovery = chrome::key_span("m", "pick another mode");
    recovery.push(c(&format!(" {} ", chrome::SEP), FAINT));
    recovery.extend(chrome::key_span("esc", "keep Edit Mode"));

    let mut lines = vec![
        chrome::user_cmd("/mode agent", Some("now")),
        Line::from(""),
        chrome::agent_head("Mode change refused"),
        chrome::fail_head("Agent Mode was not granted"),
    ];
    lines.extend(chrome::fail_rows(vec![
        vec![t("Subagent grants need approval in this workspace.")],
        vec![m("Edit Mode remains in effect. Ask a workspace owner to approve Agent Mode.")],
        recovery,
    ]));
    lines.extend([
        Line::from(""),
        chrome::agent_head("Modes"),
    ]);
    lines.extend(chrome::select_list(
        body.width as usize,
        MODES,
        1,
        None,
    ));
    chrome::transcript(f, body, lines);

    let inner = chrome::card(
        f,
        dock,
        "Choose next step",
        &[("↑↓", "move"), ("enter", "choose"), ("esc", "keep Edit Mode")],
    );
    let dock_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            c("› ", ACCENT),
            t("Ask a workspace owner"),
            m("  ·  approve Agent Mode first"),
        ]),
        Line::from(vec![
            Span::raw("    "),
            t("Stay in Edit Mode"),
            m("  ·  keep current permissions"),
        ]),
    ];
    f.render_widget(Paragraph::new(dock_lines), inner);
}
