use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, INFO, OK};

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(f, "reply when ready…", &status);
    let url = "http://127.0.0.1:3200/share/shr-71b2ce";

    let lines = vec![
        chrome::user_cmd("/share", Some("just now")),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t("Local share reference created"),
        ]),
        field(vec![m("link      "), c(url, INFO)]),
        field(vec![m("share id  "), t("shr-71b2ce")]),
        field(vec![m("session   "), t("rate-limit review · sess-31c9")]),
        field(vec![m("shared    "), t("read-only transcript · local access only")]),
        Line::from(""),
        recovery(),
    ];

    chrome::transcript(f, body, lines);
}

fn field(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("     ")];
    row.append(&mut spans);
    Line::from(row)
}

fn recovery() -> Line<'static> {
    let mut spans = vec![Span::raw("     ")];
    spans.extend(chrome::key_span("c", "copy link"));
    spans.push(m(" · "));
    spans.extend(chrome::key_span("/unshare", "revoke local share reference"));
    Line::from(spans)
}
