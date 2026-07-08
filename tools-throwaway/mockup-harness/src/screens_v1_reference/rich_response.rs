use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{
    self, c, cb, t, ACCENT, BAD, BRIGHT, DIM, FAINT, GHOST, INFO, OK, TITLE, TXT, WARN,
};

const COLS: [usize; 4] = [14, 11, 8, 22];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "Chat",
        "reply to Cortex, or ask a follow-up\u{2026}",
        &[
            ("enter", "send"),
            ("ctrl+o", "collapse"),
            ("/", "commands"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd(
        "compare our options for capping concurrent requests",
        "15:07:41",
    ));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head("Cortex"));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("Three strategies fit here. The table ranks them by "),
        c("throughput", TITLE),
        t(" and "),
        c("memory", TITLE),
        t(","),
    ]));
    lines.push(Line::from(vec![
        Span::raw("   "),
        t("then I call out where each one is the right tool."),
    ]));
    lines.push(Line::from(""));

    lines.extend(table());
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("   "),
        Span::styled(
            "rust ",
            Style::default().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2500}".repeat(46), Style::default().fg(GHOST)),
    ]));
    lines.push(code(vec![kw("async fn "), id("serve"), pun("("), id("limit"), pun(": "), ty("Quota"), pun(") -> "), ty("Result"), pun("<()> {")]));
    lines.push(code(vec![pad(4), kw("let "), id("sem"), pun(" = "), ty("Semaphore"), pun("::"), id("new"), pun("("), id("limit"), pun("."), id("max"), pun(");")]));
    lines.push(code(vec![pad(4), id("tokio"), pun("::"), id("spawn"), pun("("), kw("async move "), pun("{ "), id("handle"), pun("("), id("sem"), pun(")."), kw("await"), pun(" });")]));
    lines.push(code(vec![pad(4), ty("Ok"), pun("(())")]));
    lines.push(code(vec![pun("}")]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::raw("   "),
        chrome::chip("Recommendation", ACCENT),
        t("  go with "),
        cb("async tasks", OK),
        t(" bounded by a "),
        c("Semaphore", TITLE),
        t("."),
    ]));

    chrome::transcript(f, body, lines);
}

fn cell(spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(chrome::vsep());
    out
}

fn header_row() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["STRATEGY", "THROUGHPUT", "MEMORY", "VERDICT"];
    let mut spans = vec![chrome::vsep()];
    for (label, w) in labels.iter().zip(COLS) {
        spans.extend(cell(vec![hd(label, w)]));
    }
    Line::from(spans)
}

fn data_row(
    strategy: &str,
    throughput: (&str, ratatui::style::Color),
    memory: (&str, ratatui::style::Color),
    verdict: (&str, ratatui::style::Color),
) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![Span::styled(
        format!("{strategy:<w$}", w = COLS[0]),
        Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD),
    )]));
    spans.extend(cell(vec![c(
        &format!("{:<w$}", throughput.0, w = COLS[1]),
        throughput.1,
    )]));
    spans.extend(cell(vec![c(
        &format!("{:<w$}", memory.0, w = COLS[2]),
        memory.1,
    )]));
    let pad = COLS[3].saturating_sub(2 + verdict.0.chars().count());
    spans.extend(cell(vec![
        c(chrome::state_glyph(verdict.1), verdict.1),
        c(&format!(" {}", verdict.0), verdict.1),
        t(&" ".repeat(pad)),
    ]));
    Line::from(spans)
}

fn table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.push(data_row(
        "async tasks",
        ("high", OK),
        ("low", OK),
        ("scales cleanly", OK),
    ));
    lines.push(data_row(
        "thread pool",
        ("medium", WARN),
        ("high", WARN),
        ("memory heavy", WARN),
    ));
    lines.push(data_row(
        "process fork",
        ("low", BAD),
        ("high", WARN),
        ("avoid at scale", BAD),
    ));
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}

fn code(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut out = vec![
        Span::raw("   "),
        Span::styled("\u{2502} ".to_string(), Style::default().fg(FAINT)),
    ];
    out.append(&mut spans);
    Line::from(out)
}

fn pad(n: usize) -> Span<'static> {
    Span::raw(" ".repeat(n))
}

fn kw(s: &str) -> Span<'static> {
    c(s, INFO)
}

fn ty(s: &str) -> Span<'static> {
    c(s, TITLE)
}

fn id(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(BRIGHT))
}

fn pun(s: &str) -> Span<'static> {
    Span::styled(s.to_string(), Style::default().fg(TXT))
}
