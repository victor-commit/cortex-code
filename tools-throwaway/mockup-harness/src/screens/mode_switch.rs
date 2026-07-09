use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, MUTED};

const MODES: &[(&str, &str, &str)] = &[
    ("Plan", "read-only investigation", "8 tools"),
    ("Edit", "read, write, and run", "13 tools · current"),
    ("Agent", "autonomous work with subagents", "15 tools · selected"),
    ("Chat", "interactive conversation", "6 tools"),
    ("Headless", "non-interactive single shot", "10 tools"),
];

const GRANTS: &[(&str, [&str; 5], bool)] = &[
    ("Read file", ["yes", "yes", "yes", "yes", "yes"], false),
    ("Search code", ["yes", "yes", "yes", "yes", "yes"], false),
    ("Run shell", ["ro", "gated", "gated", "-", "gated"], false),
    ("Edit file", ["-", "yes", "yes", "-", "yes"], false),
    ("Plan note", ["yes", "-", "yes", "-", "yes"], true),
    ("Spawn subagent", ["-", "-", "yes", "-", "yes"], true),
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 10, &status);

    let mut lines = vec![
        chrome::user_cmd("/mode agent", Some("now")),
        Line::from(""),
        chrome::agent_head("Mode change requested"),
        Line::from(vec![
            Span::raw("   "),
            m("current  "),
            t("Edit Mode"),
            m("  ·  requested  "),
            t("Agent Mode"),
            m("  ·  2 new grants"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("policy   "),
            t("the runtime grants modes after this confirmation"),
        ]),
        Line::from(""),
        chrome::agent_head("Modes"),
    ];
    lines.extend(chrome::select_list(
        body.width as usize,
        MODES,
        2,
        None,
    ));
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("Tool grants by mode"),
            m("  ·  + unlocked by this switch"),
        ]),
        matrix_head(),
        matrix_rule(),
    ]);
    lines.extend(GRANTS.iter().map(|(tool, grants, newly_granted)| {
        matrix_row(tool, *grants, *newly_granted)
    }));
    lines.push(Line::from(vec![
        Span::raw("   "),
        m("promotion  "),
        t("Edit → Agent"),
        m("  ·  policy check → session grant"),
    ]));
    chrome::transcript(f, body, lines);

    let inner = chrome::card(
        f,
        dock,
        "Confirm Agent mode",
        &[("enter", "request"), ("esc", "cancel")],
    );
    let dock_lines = vec![
        Line::from("  Request Agent Mode for this session."),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            c("› ", BRIGHT),
            t("Agent Mode"),
            m("  ·  selected target"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            c("+ ", BRIGHT),
            t("Plan note and Spawn subagent will be granted."),
        ]),
        Line::from(vec![
            Span::raw("  "),
            m("The mode changes only after the runtime approves this request."),
        ]),
    ];
    f.render_widget(Paragraph::new(dock_lines), inner);
}

fn matrix_head() -> Line<'static> {
    Line::from(vec![
        m("   tool                 "),
        m(" Plan  "),
        m(" Edit  "),
        m(" Agent "),
        m(" Chat  "),
        m(" Headless"),
    ])
}

fn matrix_rule() -> Line<'static> {
    Line::from(c(&format!("   {}", "─".repeat(69)), FAINT))
}

fn matrix_row(tool: &str, grants: [&str; 5], newly_granted: bool) -> Line<'static> {
    let mut spans = vec![Span::raw("   ")];
    if newly_granted {
        spans.push(c("+ ", BRIGHT));
        spans.push(c(&format!("{tool:<17}"), BRIGHT));
    } else {
        spans.push(Span::raw("  "));
        spans.push(m(&format!("{tool:<17}")));
    }
    for grant in grants {
        spans.push(c(&format!(" {grant:^6}"), MUTED));
    }
    Line::from(spans)
}
