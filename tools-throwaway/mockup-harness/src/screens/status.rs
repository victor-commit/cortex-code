//! Full-screen session status view with aligned identity fields and a
//! context budget that owns the only context and cost figures on screen.

use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, FieldPair, StatusLine, BAD, FAINT, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "session status", &status);
    let uptime = chrome::timefmt(3201.0);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            m("session identity"),
            c("  ·  active", FAINT),
        ]),
        Line::from(""),
    ];
    lines.extend(chrome::paired_field_rows(&[
        FieldPair {
            left_label: "session",
            left_value: "wf-42",
            right_label: "started",
            right_value: "10:14",
        },
        FieldPair {
            left_label: "agent",
            left_value: "build",
            right_label: "uptime",
            right_value: &uptime,
        },
        FieldPair {
            left_label: "model",
            left_value: "Claude Opus 4.8",
            right_label: "messages",
            right_value: "38",
        },
        FieldPair {
            left_label: "provider",
            left_value: "Anthropic",
            right_label: "turns",
            right_value: "17",
        },
    ]));
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            m("cwd         "),
            t("~/projects/cortex-code"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("branch      "),
            t("master  "),
            c("● ", OK),
            t("clean"),
        ]),
        Line::from(""),
    ]);

    let mut context = vec![Span::raw("   "), m("context     ")];
    context.extend(chrome::bar_in(91, 100, 24, BAD));
    context.extend([
        Span::raw("  "),
        c("91%", BAD),
        m(" · 182k / 200k tokens"),
        m(" · "),
    ]);
    context.extend(chrome::key_span("c", "compact"));
    lines.push(Line::from(context));
    lines.push(Line::from(vec![
        Span::raw("   "),
        m("cost        "),
        t("$0.0184"),
        m(" · this session"),
    ]));

    f.render_widget(Paragraph::new(lines), body);
}
