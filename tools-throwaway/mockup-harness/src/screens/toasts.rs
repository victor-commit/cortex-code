use ratatui::{
    layout::Rect,
    style::Color,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, FAINT, INFO, OK};

const TOAST_WIDTH: u16 = 45;

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "reply when ready…", &status);

    let lines = vec![
        chrome::user_cmd("/settings save", Some("1m ago")),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para("Your preferences stay in this project after they are saved."),
    ];
    chrome::transcript(f, body, lines);

    let x = body.x + body.width.saturating_sub(TOAST_WIDTH + 2);
    toast(
        f,
        Rect::new(x, body.y + 1, TOAST_WIDTH, 2),
        BAD,
        "✗",
        "BAD",
        "Settings were not saved",
        Some(("s", "retry save · stays until you act")),
    );
    toast(
        f,
        Rect::new(x, body.y + 4, TOAST_WIDTH, 2),
        OK,
        "✓",
        "OK",
        "Checkpoint saved",
        None,
    );
    toast(
        f,
        Rect::new(x, body.y + 7, TOAST_WIDTH, 2),
        INFO,
        "●",
        "INFO",
        "Model switched",
        None,
    );
}

fn para(text: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), t(text)])
}

fn toast(
    f: &mut Frame,
    area: Rect,
    level: Color,
    glyph: &str,
    label: &str,
    title: &str,
    recovery: Option<(&str, &str)>,
) {
    let bar = if level == BAD { BAD } else { FAINT };
    let head = Line::from(vec![
        c("│ ", bar),
        c(&format!("{glyph} "), level),
        c(&format!("{label:<5}"), level),
        Span::raw(" "),
        t(title),
    ]);
    let detail = match recovery {
        Some((key, label)) => {
            let mut row = vec![c("│ ", bar), Span::raw("  ")];
            row.extend(chrome::key_span(key, label));
            Line::from(row)
        }
        None => Line::from(vec![c("│ ", bar), Span::raw("  "), m("clears on its own")]),
    };
    f.render_widget(Paragraph::new(vec![head, detail]), area);
}
