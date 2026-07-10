use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, WARN};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "type a follow-up, it runs after this turn…", &status);
    let mut recovery = chrome::key_span("r", "retry now");
    recovery.push(m(" · "));
    recovery.extend(chrome::key_span("q", "quit"));

    let lines = vec![
        chrome::user_cmd("run the test suite and summarize failures", None),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("● ", WARN),
            t("Rate limit reached"),
        ]),
        detail(vec![m("plan      "), t("Pro · per-minute limits")]),
        Line::from(""),
        chrome::agent_head("Usage this minute"),
        usage("requests", 50, 50, "50 / 50 · at cap"),
        usage("tokens", 58, 80, "58k / 80k · 73%"),
        Line::from(""),
        chrome::refreshing_note(),
        detail(vec![m("Your paused turn resumes on its own.")]),
        detail(recovery),
        chrome::working_band("Retry in", 42.0, None),
    ];

    chrome::transcript(f, body, lines);
}

fn usage(label: &str, filled: usize, total: usize, detail: &str) -> Line<'static> {
    let mut row = vec![Span::raw("     "), m(&format!("{label:<10}"))];
    row.extend(chrome::bar_in(filled, total, 26, WARN));
    row.push(m(&format!("  {detail}")));
    Line::from(row)
}

fn detail(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("     ")];
    row.append(&mut spans);
    Line::from(row)
}
