//! input/status — the `/status` view as a CLI turn: the user asks where the
//! session stands, the agent prints session identity, model, working tree, and
//! live budget (a `█░` context bar) in aligned two-column rows on the canvas.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, bar_spans, c, m, t, BRIGHT, DIM, INFO, OK, TXT, WARN};

const COLS: [usize; 4] = [8, 11, 8, 11];

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/status",
        "type a follow-up, or press c to compact the context\u{2026}",
        &[("c", "compact"), ("e", "export"), ("q", "quit")],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/status", "14:55:31"));
    lines.push(Line::from(""));
    lines.push(chrome::agent_head("Session status \u{2014} wf-42 \u{00b7} active"));
    lines.push(Line::from(""));

    lines.extend(identity_table());
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        m("   cwd     "),
        t("~/projects/cortex-code"),
    ]));
    lines.push(Line::from(vec![
        m("   branch  "),
        c("main", TXT),
        m("   "),
        c("\u{25cf} clean", OK),
    ]));
    lines.push(Line::from(""));

    // Context budget bar
    let mut ctx_line = vec![m("   context ")];
    ctx_line.extend(bar_spans(9, 20));
    ctx_line.push(m("  "));
    ctx_line.push(c("47%", WARN));
    ctx_line.push(m("   "));
    ctx_line.push(Span::styled("94k / 200k tokens", Style::default().fg(DIM)));
    lines.push(Line::from(ctx_line));

    lines.push(Line::from(vec![
        m("   cost    "),
        Span::styled("$0.0184", Style::default().fg(TXT)),
        m("   this session"),
    ]));

    chrome::transcript(f, body, lines);
}

fn cell(spans: Vec<Span<'static>>, strong: bool) -> Vec<Span<'static>> {
    let mut out = vec![t(" ")];
    out.extend(spans);
    out.push(t(" "));
    out.push(if strong { block_sep() } else { chrome::vsep() });
    out
}

fn block_sep() -> Span<'static> {
    Span::styled(
        "\u{2502}".to_string(),
        Style::default()
            .fg(chrome::ORANGE)
            .add_modifier(Modifier::BOLD),
    )
}

fn header_row() -> Line<'static> {
    let hd = |label: &str, w: usize| {
        Span::styled(
            format!("{label:<w$}"),
            Style::default().fg(DIM).add_modifier(Modifier::BOLD),
        )
    };
    let labels = ["FIELD", "VALUE", "FIELD", "VALUE"];
    let mut spans = vec![chrome::vsep()];
    for (i, (label, w)) in labels.iter().zip(COLS).enumerate() {
        spans.extend(cell(vec![hd(label, w)], i == 1));
    }
    Line::from(spans)
}

fn kv_row(
    f1: &str,
    v1: &str,
    v1c: ratatui::style::Color,
    f2: &str,
    v2: &str,
    v2c: ratatui::style::Color,
) -> Line<'static> {
    let mut spans = vec![chrome::vsep()];
    spans.extend(cell(vec![c(&format!("{f1:<w$}", w = COLS[0]), DIM)], false));
    spans.extend(cell(vec![c(&format!("{v1:<w$}", w = COLS[1]), v1c)], true));
    spans.extend(cell(vec![c(&format!("{f2:<w$}", w = COLS[2]), DIM)], false));
    spans.extend(cell(vec![c(&format!("{v2:<w$}", w = COLS[3]), v2c)], false));
    Line::from(spans)
}

fn identity_table() -> Vec<Line<'static>> {
    let mut lines = vec![
        chrome::table_rule(&COLS, "\u{250c}", "\u{252c}", "\u{2510}"),
        header_row(),
        chrome::table_rule(&COLS, "\u{251c}", "\u{253c}", "\u{2524}"),
    ];
    lines.extend([
        kv_row("session", "wf-42", INFO, "started", "14:02:10", TXT),
        kv_row("agent", "Build", BRIGHT, "uptime", "53m 21s", TXT),
        kv_row("model", "claude-opus", BRIGHT, "messages", "38", TXT),
        kv_row("provider", "anthropic", TXT, "turns", "17", TXT),
    ]);
    lines.push(chrome::table_rule(&COLS, "\u{2514}", "\u{2534}", "\u{2518}"));
    lines
}
