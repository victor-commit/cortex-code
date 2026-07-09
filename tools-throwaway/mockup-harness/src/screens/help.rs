//! Curated help view. It intentionally points to the full palette instead
//! of trying to enumerate every action on a single screen.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, BRIGHT, FAINT};

const KEY_W: usize = 14;
const ACTION_W: usize = 37;
const GROUP_W: usize = KEY_W + ACTION_W;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "help · leader is ctrl+g", &status);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("› ", ACCENT),
            m("filter: "),
            c("_", BRIGHT),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("curated shortcuts"),
            c(" · ", FAINT),
            m("ctrl+p for all actions"),
        ]),
        Line::from(""),
        section("Session", "Views"),
        separator(),
        binding("ctrl+g n", "new session", "ctrl+g t", "open timeline"),
        binding("ctrl+g s", "open sessions", "ctrl+g d", "open diff"),
        binding("ctrl+g f", "fork current turn", "ctrl+g r", "open review"),
        binding("ctrl+c", "interrupt a turn", "ctrl+g m", "switch model"),
        Line::from(""),
        section("Composer", "Global"),
        separator(),
        binding("enter", "send message", "ctrl+p", "all actions"),
        binding("shift+enter", "insert newline", "ctrl+g h", "open help"),
        binding("/", "slash commands", "ctrl+g l", "switch theme"),
        binding("@", "mention a file", "esc", "close a picker"),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            m("keys are scoped to the active view or composer"),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), body);
}

fn section(left: &str, right: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c(&format!("{left:<GROUP_W$}"), BRIGHT),
        c(" │ ", FAINT),
        c(right, BRIGHT),
    ])
}

fn separator() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c(&"─".repeat(GROUP_W), FAINT),
        c(" │ ", FAINT),
        c(&"─".repeat(GROUP_W), FAINT),
    ])
}

fn binding(left_key: &str, left_action: &str, right_key: &str, right_action: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{left_key:<KEY_W$}")),
        t(&format!("{left_action:<ACTION_W$}")),
        c(" │ ", FAINT),
        m(&format!("{right_key:<KEY_W$}")),
        t(right_action),
    ])
}
