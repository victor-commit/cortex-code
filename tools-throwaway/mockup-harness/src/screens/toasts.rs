//! Screen — states/toasts. A normal session turn with a small stack of toast
//! notifications in the top-right corner: a success toast (checkpoint saved), an
//! info toast (model switched), and an error toast in plain English. Toasts are
//! transient status cards — a colored status dot carries the level, never a
//! decorative glyph — that float over the scrollback and fade on their own.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::chrome::{self, c, m, t, BAD, BRIGHT, DIM, FAINT, INFO, OK, ORANGE_D, TITLE};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "save a checkpoint before the refactor",
        "keep going \u{2014} toasts clear on their own, or press t to dismiss them\u{2026}",
        &[
            ("s", "retry save"),
            ("t", "dismiss toasts"),
            ("enter", "send"),
            ("u", "undo"),
            ("q", "quit"),
        ],
    );

    // --- the base session turn (bottom-anchored scrollback) ---
    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/checkpoint save auth-refactor", "17:02:19"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head(
        "Checkpoint saved \u{2014} you can restore this point at any time",
    ));
    lines.push(Line::from(vec![
        m("   name   "),
        c("auth-refactor", TITLE),
        m("     files  "),
        t("2 tracked"),
        m("     size  "),
        t("18.4 KB"),
    ]));
    lines.push(Line::from(vec![
        m("   scope  "),
        t("crates/server/src/auth/*"),
        m("     restore with  "),
        t("/checkpoint restore auth-refactor"),
    ]));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head(
        "Ready \u{2014} continue the refactor, the session picks up from here.",
    ));
    chrome::transcript(f, body, lines);

    // --- toast stack, floated in the top-right corner over the scrollback ---
    let tw: u16 = 40;
    let tx = body.x + body.width.saturating_sub(tw + 2);
    let ty = body.y + 1;
    let gap: u16 = 1;
    let th: u16 = 4;

    toast(
        f,
        Rect::new(tx, ty, tw, th),
        BAD,
        "BAD",
        "Could not save your settings",
        "s retry save",
    );
    toast(
        f,
        Rect::new(tx, ty + th + gap, tw, th),
        OK,
        "OK",
        "Saved checkpoint",
        "auth-refactor \u{00b7} 2 files kept",
    );
    toast(
        f,
        Rect::new(tx, ty + 2 * (th + gap), tw, th),
        INFO,
        "INFO",
        "Switched to claude-opus",
        "applies from the next turn",
    );
}

/// One toast card: rounded border, a `● LABEL  Title` line and a dim detail
/// line. The status level is carried by the dot + label color, not a glyph.
fn toast(
    f: &mut Frame,
    area: Rect,
    level_color: ratatui::style::Color,
    label: &str,
    title: &str,
    detail: &str,
) {
    let b = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ORANGE_D));
    let inner = b.inner(area);
    f.render_widget(b, area);

    let title_color = if level_color == BAD { BAD } else { BRIGHT };
    let glyph = if level_color == INFO {
        "\u{25cf}"
    } else {
        chrome::state_glyph(level_color)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{glyph} "), Style::default().fg(level_color)),
            Span::styled(
                format!(" {label:<4} "),
                Style::default().fg(level_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                title.to_string(),
                Style::default().fg(title_color).add_modifier(Modifier::BOLD),
            ),
        ])),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("      ", Style::default().fg(FAINT)),
            Span::styled(detail.to_string(), Style::default().fg(DIM)),
        ])),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
}
