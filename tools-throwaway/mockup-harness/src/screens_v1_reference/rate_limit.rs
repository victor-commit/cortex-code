//! Screen — states/rate-limit. The account has sent too many requests to the
//! model in a short window: a plain-English "Rate limit reached", which limit
//! was hit and when it clears (an absolute reset time), and the current usage
//! against the per-minute caps. The paused turn resumes on its own; the
//! composer awaits. `rate_limit` never shown as jargon.

use ratatui::{
    style::Style,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, dot, m, t, BAD, FAINT, OK, TITLE, TXT, WARN};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "run the test suite and summarize failures",
        "the paused turn resumes automatically when the limit clears\u{2026}",
        &[("q", "quit")],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "run the test suite and summarize failures",
        "16:18:44",
    ));
    lines.push(Line::from(""));

    // --- the limit, stated plainly ---
    lines.push(chrome::agent_head(
        "Turn paused \u{2014} the model is receiving too many requests",
    ));
    lines.push(Line::from(vec![
        Span::raw("   "),
        dot(WARN),
        c(" Rate limit reached", WARN),
        m("  \u{2014}  "),
        t("sending requests faster than the plan allows"),
    ]));
    lines.push(Line::from(""));

    // --- which limit, and when it clears ---
    lines.push(Line::from(vec![
        m("   limit    "),
        t("requests per minute"),
        m("   \u{00b7}   plan  "),
        t("Pro"),
    ]));
    lines.push(Line::from(vec![
        m("   resets   "),
        c("16:19", TITLE),
        m("   \u{00b7}   the paused turn resumes on its own"),
    ]));
    lines.push(Line::from(""));

    // --- usage against the per-minute caps ---
    lines.push(chrome::agent_head("Usage this minute"));
    lines.push(gauge("requests", 40, 40, "50 / 50", BAD, "at the cap"));
    lines.push(gauge("tokens", 28, 40, "58k / 80k", WARN, "72%"));
    lines.push(Line::from(""));

    // --- wait hint (hugs the composer) ---
    lines.push(Line::from(vec![
        Span::raw("   "),
        dot(OK),
        c(" Waiting", OK),
        m("  \u{2014}  the turn will resume automatically once the limit clears."),
    ]));

    chrome::transcript(f, body, lines);
}

/// A usage gauge: `  label  [bar]  count  note`, filled/total in a status hue.
fn gauge(
    label: &str,
    filled: usize,
    total: usize,
    count: &str,
    note_color: ratatui::style::Color,
    note: &str,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!("   {label:<10}"),
        Style::default().fg(TXT),
    )];
    spans.extend(chrome::bar_spans(filled, total));
    spans.push(Span::styled(format!("  {count:<10}"), Style::default().fg(FAINT)));
    spans.push(Span::styled(note.to_string(), Style::default().fg(note_color)));
    Line::from(spans)
}
