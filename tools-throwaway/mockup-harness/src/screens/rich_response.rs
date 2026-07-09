//! Session screen - a rich agent answer holding BOTH a markdown table and a
//! code block. The table is light: aligned space-separated columns, a MUTED
//! header row and one FAINT rule under it (no heavy box-drawing frame, which
//! would read as a second bordered zone). The Verdict column pairs a glyph
//! with its color (OK check / WARN dot / BAD cross) so verdicts survive with
//! color stripped. The code block gets a language label, a FAINT gutter and
//! a `y copy` affordance adjacent to it; below the table the prose adds
//! exactly ONE recommendation line and never re-lists per-row verdicts.

use ratatui::{
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BRIGHT, FAINT, MUTED, WARN};

/// Table geometry: name / throughput / memory columns, then the verdict.
const COL_STRATEGY: usize = 16;
const COL_THROUGHPUT: usize = 13;
const COL_MEMORY: usize = 9;
/// Full table width (three fixed columns + the widest verdict cell).
const TABLE_W: usize = COL_STRATEGY + COL_THROUGHPUT + COL_MEMORY + 16;

/// Code identifier styling (single treatment for the whole answer).
fn ident(s: &str) -> Span<'static> {
    c(s, BRIGHT)
}

/// One aligned data row; `verdict` supplies the glyph+color cell.
fn row(strategy: &str, thr: &str, mem: &str, verdict: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![
        Span::raw("   "),
        t(&format!("{strategy:<COL_STRATEGY$}")),
        t(&format!("{thr:<COL_THROUGHPUT$}")),
        t(&format!("{mem:<COL_MEMORY$}")),
    ];
    spans.extend(verdict);
    Line::from(spans)
}

/// One code-block row: FAINT `│` gutter, code text in TXT.
fn code(s: &str) -> Line<'static> {
    Line::from(vec![Span::raw("   "), c("│ ", FAINT), t(s)])
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_page(
        f,
        "reply to Cortex, or ask a follow-up\u{2026}",
        &status,
    );

    // WARN verdict cell: glyph carries the hue, text stays TXT (same rule as
    // chrome::verdict for OK/BAD).
    let mut caution = vec![c(&format!("{} ", chrome::DOT), WARN)];
    caution.push(t("memory heavy"));

    // Code-block head: language label + copy affordance + FAINT rule filling
    // out to the table width.
    let mut code_head: Vec<Span> = vec![
        Span::raw("   "),
        m("rust"),
        c(&format!(" {} ", chrome::SEP), FAINT),
    ];
    code_head.extend(chrome::key_span("y", "copy"));
    code_head.push(Span::raw(" "));
    code_head.push(c(&"─".repeat(TABLE_W - 14), FAINT));

    let lines: Vec<Line> = vec![
        chrome::user_cmd(
            "compare our options for capping concurrent requests",
            Some("3m ago"),
        ),
        Line::from(""),
        chrome::agent_head("Cortex"),
        Line::from(""),
        para(vec![
            t("Three strategies fit here. The table ranks them by throughput and memory cost,"),
        ]),
        para(vec![
            t("then the sketch shows the bounded async variant."),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            m(&format!("{:<COL_STRATEGY$}", "Strategy")),
            m(&format!("{:<COL_THROUGHPUT$}", "Throughput")),
            m(&format!("{:<COL_MEMORY$}", "Memory")),
            m("Verdict"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c(&"─".repeat(TABLE_W), FAINT),
        ]),
        row(
            "async tasks",
            "high",
            "low",
            chrome::verdict(true, "scales cleanly"),
        ),
        row("thread pool", "medium", "high", caution),
        row(
            "process fork",
            "low",
            "high",
            chrome::verdict(false, "avoid at scale"),
        ),
        Line::from(""),
        Line::from(code_head),
        code("async fn serve(limit: Quota) -> Result<()> {"),
        code("    let sem = Semaphore::new(limit.max);"),
        code("    tokio::spawn(async move { handle(sem).await });"),
        code("    Ok(())"),
        code("}"),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            chrome::dot(MUTED),
            c(" Recommendation", BRIGHT),
            t("  go with async tasks bounded by a "),
            ident("Semaphore"),
            t("."),
        ]),
        Line::from(""),
    ];

    chrome::transcript(f, body, lines);
}

/// A prose row indented to column 4, under the `agent_head` heading.
fn para(mut spans: Vec<Span<'static>>) -> Line<'static> {
    let mut row = vec![Span::raw("   ")];
    row.append(&mut spans);
    Line::from(row)
}
