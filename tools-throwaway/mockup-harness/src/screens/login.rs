//! auth/login — the sign-in landing. A centered brand panel: the `Cortex Code`
//! wordmark (no decorative star), a short welcome line, then the three sign-in
//! methods as a selectable list (the first is selected: `›` marker + warm
//! selection band + brighter text), over the shared CLI chrome.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, cb, m, ACCENT, AMBER, BRIGHT, DIM, FAINT, TXT};

pub fn render(f: &mut Frame) {
    let (body, dock) = chrome::cli_action(
        f,
        "auth \u{00b7} login",
        7,
        &[
            ("enter", "sign in"),
            ("\u{2191}\u{2193}", "choose"),
            ("q", "quit"),
        ],
    );
    chrome::transcript(f, body, backdrop());

    let inner = chrome::card(f, dock, "Sign in", AMBER, &[]);
    let lines: Vec<Line> = vec![
        method_row(true, "Sign in with browser", "authorize Cortex in your browser"),
        method_row(false, "Use an API key", "paste a provider API key"),
        method_row(false, "Connect a provider", "pick from the provider catalog"),
        Line::from(""),
        Line::from(vec![
            chrome::c("Credentials stored locally in ", FAINT),
            chrome::c("auth.json", DIM),
            chrome::c(" (0600)", FAINT),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

const NAME_W: usize = 22;

fn method_row(sel: bool, name: &str, desc: &str) -> Line<'static> {
    let marker = if sel {
        cb(" \u{203a} ", ACCENT)
    } else {
        Span::raw("   ")
    };
    let name_span = if sel {
        cb(&format!("{name:<NAME_W$}"), BRIGHT)
    } else {
        chrome::c(&format!("{name:<NAME_W$}"), TXT)
    };
    Line::from(vec![
        marker,
        name_span,
        m("  "),
        chrome::c(desc, if sel { TXT } else { DIM }),
    ])
}

fn backdrop() -> Vec<Line<'static>> {
    let dim = |s: &str| Line::from(Span::styled(s.to_string(), Style::default().fg(FAINT)));
    vec![
        Line::from(Span::styled(
            " Cortex Code",
            Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        dim("   Welcome back \u{2014} choose how to sign in"),
    ]
}
