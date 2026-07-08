//! Screen 5 — the reviewer verdict framed as a live CLI turn: the user runs
//! `/review feat-api-routes`, the L3 Code Reviewer streams its intent audit and
//! the L4 QA Validator streams its process check, then the combined gate posts
//! the outcome and asks whether to re-post to the worker. The composer awaits.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, dot, m, t, BAD, DIM, FAINT, INFO, MUTED, OK, TITLE, TXT};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/review feat-api-routes",
        "type a reply to the worker \u{2014} or press a to accept, r to re-post the findings\u{2026}",
        &[
            ("r", "re-post findings"),
            ("", "a accept"),
            ("d", "open diff"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("/review feat-api-routes", "14:41:07"));
    lines.push(Line::from(""));

    // --- summary ---
    lines.push(chrome::agent_head(
        "Review complete \u{2014} feat-api-routes \u{00b7} two gates ran",
    ));
    lines.push(Line::from(vec![
        m("   diff  "),
        t("6 files \u{00b7} +214 \u{2212}37"),
        m("     worker  "),
        c("ws-3f9a2c", INFO),
        m("     branch  "),
        t("workflow/wf-42/feat-api-routes"),
    ]));
    lines.push(Line::from(vec![
        m("   verdict  "),
        dot(BAD),
        c(" CHANGES REQUESTED", BAD),
        m("   \u{00b7}   any "),
        c("fail", BAD),
        m(" blocks the merge"),
    ]));
    lines.push(Line::from(""));

    // --- L3 code reviewer ---
    lines.push(chrome::agent_head(
        "Code Reviewer \u{2014} L3 intent audit   (5 checks \u{00b7} 3 pass \u{00b7} 2 fail \u{00b7} \u{201c}intent, not just diff\u{201d})",
    ));
    lines.extend(check("1", "Scope fidelity", true, "changes map to the request; nothing extra", "6 files"));
    lines.extend(check("2", "Correctness", true, "logic satisfies the acceptance contract", "ACC-API-010"));
    lines.extend(check("3", "Dead weight", false, "unused helper build_stub() left behind", "server/util.rs:88"));
    lines.extend(check("4", "Swallowed failure", false, "catch {} drops the error, masking a 500", "routes.rs:142"));
    lines.extend(check("5", "Honest report", true, "the summary matches the actual diff", "pr-summary"));
    lines.push(Line::from(""));

    // --- L4 qa validator ---
    lines.push(chrome::agent_head(
        "QA Validator \u{2014} L4 process check   (4 checks \u{00b7} 2 pass \u{00b7} 2 fail \u{00b7} \u{201c}un-run is not done\u{201d})",
    ));
    lines.extend(check("6", "Tests executed", true, "42 passed, 0 failed", "cargo test"));
    lines.extend(check("7", "Surface exercised", true, "rate-limit held at call 6", "curl :8080/login"));
    lines.extend(check("8", "Evidence captured", false, "absent for feat-audit-log", "run-log.txt"));
    lines.extend(check("9", "Regression re-run", false, "never re-run after the fix landed", "cargo test --all"));
    lines.push(Line::from(""));

    // --- combined gate (hugs the composer) ---
    lines.push(chrome::agent_head(
        "Gate \u{2014} merge withheld \u{00b7} 4 findings (2 code \u{00b7} 2 process) \u{00b7} cannot confirm without evidence",
    ));
    lines.push(Line::from(vec![
        m("   next  re-post to "),
        c("ws-3f9a2c", INFO),
        m(" with: "),
        c("Q3, Q4, Q8, Q9", BAD),
        m("  \u{00b7}  worker fixes \u{2192} re-review"),
    ]));

    chrome::transcript(f, body, lines);
}

// Shared name column: every row's locus gutter aligns only while each check
// name fits within NAME_W, otherwise the gutter drifts right and breaks the rule.
const NAME_W: usize = 32;

fn headline(num: &str, name: &str, ok: bool, locus: &str) -> Line<'static> {
    let (word, color) = if ok { ("pass", OK) } else { ("fail", BAD) };
    let mut spans = vec![
        Span::raw("  "),
        dot(color),
        Span::styled(format!(" {word} "), Style::default().fg(color)),
        Span::styled(format!("Q{num}  "), Style::default().fg(MUTED)),
        Span::styled(
            format!("{name:<NAME_W$}"),
            Style::default().fg(TITLE).add_modifier(Modifier::BOLD),
        ),
    ];
    if !locus.is_empty() {
        spans.push(Span::styled(locus.to_string(), Style::default().fg(FAINT)));
    }
    Line::from(spans)
}

fn evidence(text: &str, emphasize: bool) -> Line<'static> {
    Line::from(vec![
        Span::raw("         "),
        Span::styled(
            text.to_string(),
            Style::default().fg(if emphasize { TXT } else { DIM }),
        ),
    ])
}

fn check(num: &str, name: &str, ok: bool, note: &str, at: &str) -> Vec<Line<'static>> {
    vec![headline(num, name, ok, at), evidence(note, !ok)]
}
