//! Screen 2 — run-workflow Step 0, framed as a live CLI turn: the user runs
//! `run_workflow wf-42`, the agent streams the plan, preflight and the feature
//! graph, then asks to spawn worker #1. The composer awaits the confirmation.

use ratatui::{
    style::Color,
    text::{Line, Span},
    Frame,
};

use crate::chrome::{self, c, cb, dot, m, t, ACCENT, DIM, OK, ORANGE_D, TITLE, WARN};

pub fn render(f: &mut Frame) {
    let body = chrome::cli_page(
        f,
        "/run workflow",
        "confirm to spawn worker #1 \u{2014} or type an instruction to adjust the plan\u{2026}",
        &[
            ("enter", "spawn worker #1"),
            ("N", "parallelism"),
            ("e", "edit plan"),
            ("q", "quit"),
        ],
    );

    let mut lines: Vec<Line> = Vec::new();
    lines.push(chrome::user_cmd("run_workflow wf-42", "14:02:11"));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head(
        "Loaded wf-42 \u{2014} goal: ship m1-auth \u{00b7} m2-api \u{00b7} m3-web",
    ));
    lines.push(Line::from(m(
        "   runbook: preflight \u{2192} schedule waves \u{2192} workers \u{2192} verify \u{2192} integrate",
    )));
    lines.push(Line::from(vec![
        m("   parallelism "),
        t("N=3"),
        m("   \u{00b7}   budget "),
        t("$2.00"),
        m("   \u{00b7}   approval "),
        c("auto within allow-rules", TITLE),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head(
        "Preflight \u{2014} author-time artifacts (Phases B/C)",
    ));
    lines.push(artifacts(&[
        "architecture.md",
        "acceptance-contract.md",
        "acceptance-state.json",
    ]));
    lines.push(artifacts(&["features.json", "boundaries.md", "services.yaml"]));
    lines.push(artifacts(&["init.sh", "skills/", ""]));
    lines.push(Line::from(vec![
        m("   runner-seeded (Step 0):   "),
        bullet(),
        t(" workflow/wf-42/integration    "),
        bullet(),
        t(" progress-log.jsonl    "),
        bullet(),
        t(" intents/"),
    ]));
    lines.push(Line::from(vec![
        t("   "),
        dot(OK),
        t(" "),
        cb("preflight OK \u{2014} 11/11 artifacts present", OK),
        m("    \u{00b7}  abort form: "),
        m("MISSING <artifact>; exit 1"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head(
        "Feature dependency graph \u{2014} features.json   (node = feature \u{00b7} edge = precondition)",
    ));
    lines.push(Line::from(vec![t("   "), cb("m1-auth", ACCENT)]));
    lines.push(node("     ", dot_ready(), "feat-db-schema", "ACC-AUTH-002", ""));
    lines.push(node(
        "     \u{251c}\u{2500} ",
        None,
        "feat-auth-login",
        "ACC-AUTH-001",
        "pre: [feat-db-schema]",
    ));
    lines.push(Line::from(vec![t("   "), cb("m2-api", ACCENT)]));
    lines.push(node(
        "     \u{251c}\u{2500} ",
        None,
        "feat-api-routes",
        "ACC-API-010",
        "pre: [feat-db-schema]",
    ));
    lines.push(node(
        "     \u{2514}\u{2500} ",
        None,
        "feat-web-ui",
        "ACC-WEB-003",
        "pre: [feat-auth-login, feat-api-routes]",
    ));
    lines.push(Line::from(vec![
        m("   coverage: "),
        c("each ACC-<AREA>-NNN has exactly one fulfills", TITLE),
    ]));
    lines.push(Line::from(vec![
        m("   schedule: "),
        c("wave 1", OK),
        m(" "),
        dot(OK),
        t(" feat-db-schema"),
        m("     wave 2  feat-auth-login, feat-api-routes"),
        m("     wave 3  feat-web-ui"),
    ]));
    lines.push(Line::from(""));

    lines.push(chrome::agent_head(
        "Ready to spawn worker #1 on wave 1 (1 feature). Confirm to launch.",
    ));

    chrome::transcript(f, body, lines);
}

fn bullet() -> Span<'static> {
    c("\u{2022}", ORANGE_D)
}

fn artifacts(names: &[&str]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![t("   ")];
    for name in names {
        if name.is_empty() {
            spans.push(t(&format!("{:<26}", "")));
            continue;
        }
        spans.push(bullet());
        spans.push(t(&format!(" {name:<24}")));
    }
    Line::from(spans)
}

fn dot_ready() -> Option<Color> {
    Some(OK)
}

fn node(
    prefix: &str,
    status: Option<Color>,
    feat: &str,
    acc: &str,
    pre: &str,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![m(prefix)];
    if let Some(col) = status {
        spans.push(dot(col));
        spans.push(t(" "));
    }
    spans.push(c(feat, if status.is_some() { OK } else { WARN }));
    spans.push(m(&format!("  \u{2192} {acc}")));
    if !pre.is_empty() {
        spans.push(Span::styled(
            format!("    {pre}"),
            ratatui::style::Style::default().fg(DIM),
        ));
    }
    Line::from(spans)
}
