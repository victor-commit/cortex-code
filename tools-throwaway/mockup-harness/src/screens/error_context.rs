use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, cb, m, t, StatusLine, BRIGHT, WARN};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "choose how to continue…", &status);

    let mut lines = vec![
        chrome::user_cmd("continue the auth refactor", None),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("● ", WARN),
            t("Context limit reached"),
        ]),
        detail(vec![
            m("tokens    "),
            t("198,842 / 200,000"),
            m(" · "),
            c("99%", WARN),
        ]),
    ];

    let mut usage = vec![Span::raw("     ")];
    usage.extend(chrome::bar_ramped(99, 100, 100, 12));
    lines.push(Line::from(usage));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head("Token breakdown"));
    lines.push(detail(fact("history", "176,400 tokens")));
    lines.push(detail(fact("instructions", "9,800 tokens")));
    lines.push(detail(fact("tool definitions", "4,600 tokens")));
    lines.push(detail(fact("this turn", "8,042 tokens")));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head("How to keep going"));
    lines.push(detail(vec![
        cb("/compact", BRIGHT),
        m(" · reclaim "),
        cb("~150k tokens", BRIGHT),
        m(" and keep working"),
    ]));
    lines.push(detail(vec![
        t("/new"),
        m(" · start a fresh session, this one stays saved"),
    ]));

    chrome::transcript(f, body, lines);
}

fn fact(label: &str, value: &str) -> Vec<Span<'static>> {
    vec![m(&format!("{label:<18}")), t(value)]
}

fn detail(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("     ")];
    row.append(&mut spans);
    Line::from(row)
}
