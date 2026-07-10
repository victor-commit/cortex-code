//! Full-screen session timeline with a compact legend and one expanded turn.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, StatusLine, BAD, BRIGHT, FAINT, MUTED, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "timeline · 6 turns · wf-42", &status);

    let lines = vec![
        Line::from(""),
        Line::from(header_keys()),
        Line::from(""),
        Line::from(legend()),
        Line::from(""),
        turn("▸", "10:14", "✓", OK, "you", "started the rate-limit session"),
        turn(
            "▸",
            "10:15",
            "✓",
            OK,
            "build",
            "mapped the login request path",
        ),
        turn(
            "▾",
            "10:18",
            "✗",
            BAD,
            "review",
            "changes requested · 2 findings",
        ),
        detail("first finding: retry state was not retained"),
        detail("second finding: tests need the cold-cache case"),
        turn(
            "▸",
            "10:20",
            "✓",
            OK,
            "you",
            "approved the focused repair",
        ),
        turn(
            "▸",
            "10:22",
            "✓",
            OK,
            "build",
            "applied the retry state change",
        ),
        turn(
            "▸",
            "10:24",
            "●",
            MUTED,
            "build",
            "checking the full test suite",
        ),
        Line::from(""),
        chrome::rule_line(94),
        Line::from(vec![
            Span::raw("   "),
            m("oldest first · select a turn to inspect its result"),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), body);
}

fn header_keys() -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("enter", "expand"));
    spans.push(c(" · ", FAINT));
    spans.extend(chrome::key_span("f", "fork"));
    spans.push(c(" → ", FAINT));
    spans.push(c("new session from this turn", FAINT));
    spans
}

fn legend() -> Vec<Span<'static>> {
    vec![
        Span::raw("   "),
        m("legend  "),
        c("✓ ", OK),
        c("done", OK),
        c("  ·  ", FAINT),
        c("✗ ", BAD),
        c("changes requested", BAD),
        c("  ·  ", FAINT),
        c("● ", MUTED),
        c("in progress", MUTED),
    ]
}

fn turn(
    disclosure: &str,
    time: &str,
    glyph: &str,
    glyph_color: ratatui::style::Color,
    actor: &str,
    summary: &str,
) -> Line<'static> {
    let active = glyph == "●";
    let text_color = if active { BRIGHT } else { crate::chrome::TXT };
    Line::from(vec![
        Span::raw("   "),
        c(disclosure, FAINT),
        Span::raw("  "),
        m(&format!("{time:<5}")),
        Span::raw("  "),
        c(&format!("{glyph} "), glyph_color),
        c(&format!("{actor:<8}"), text_color),
        Span::raw("  "),
        c(summary, text_color),
    ])
}

fn detail(text: &str) -> Line<'static> {
    Line::from(vec![Span::raw("       "), c("│ ", FAINT), m(text)])
}
