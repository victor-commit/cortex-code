//! Multi-line draft composer with a collapsed paste token.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, StatusLine, INFO};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let composer_lines = vec![
        Line::from(vec![
            Span::raw(""),
            chrome::c("› ", chrome::ACCENT),
            chrome::t("Please review the provider retry path and explain the"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            chrome::t("failure mode that still needs coverage before we merge"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            chrome::c("[pasted 48 lines #1]", INFO),
            chrome::t(" then propose the smallest fix."),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            chrome::m("3 lines · enter send · shift+enter newline"),
        ]),
    ];
    let body = chrome::cli_page_multiline(f, composer_lines, &status);
    chrome::transcript(
        f,
        body,
        vec![
            chrome::user_cmd("show the retry path before I change it", Some("3m ago")),
            Line::from(""),
            chrome::agent_head("Cortex"),
            Line::from("   The provider retries only after a transient transport failure."),
        ],
    );
}
