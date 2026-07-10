use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, BRIGHT, OK};

const AUTHOR_TIME_INPUTS: &[&str] = &[
    "architecture.md",
    "acceptance-contract.md",
    "acceptance-state.json",
    "features.json",
    "boundaries.md",
    "services.yaml",
    "init.sh",
    "skills/",
];

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let (body, dock) = chrome::cli_action(f, 9, &status);

    let mut lines = vec![
        chrome::user_cmd("run_workflow wf-42", Some("now")),
        Line::from(""),
        chrome::agent_head("Workflow wf-42"),
        Line::from(vec![
            Span::raw("   "),
            m("goal  "),
            t("deliver the auth, API, and web milestones in dependency order"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("levers  execution "),
            c("serial", BRIGHT),
            m("  ·  budget "),
            c("[$2.00]", BRIGHT),
            c(" b", ACCENT),
            m(" edit"),
        ]),
        Line::from(""),
        chrome::agent_head("Preflight · Step 0"),
    ];
    lines.extend(AUTHOR_TIME_INPUTS.iter().map(|artifact| {
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t(artifact),
            m("  ·  present and valid"),
        ])
    }));
    lines.extend([
        Line::from(""),
        chrome::agent_head("Runner outputs · seeded by Step 0"),
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t("workflow/wf-42/integration"),
            m("  ·  seeded by Step 0"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t("progress-log.jsonl"),
            m("  ·  seeded by Step 0"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t("intents/"),
            m("  ·  seeded by Step 0"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("● ", OK),
            t("Step 0 complete · 8/8 author-time inputs present and valid · 3/3 runner outputs seeded by Step 0"),
        ]),
        Line::from(""),
        chrome::agent_head("Feature dependency graph · features.json"),
        Line::from(vec![
            Span::raw("   "),
            m("[wave 1]  "),
            t("feat-db-schema"),
            m("  →  ACC-AUTH-002"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("[wave 2]  "),
            t("feat-auth-login"),
            m("  →  ACC-AUTH-001  ·  pre: feat-db-schema"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("[wave 2]  "),
            t("feat-api-routes"),
            m("  →  ACC-API-010  ·  pre: feat-db-schema"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("[wave 3]  "),
            t("feat-web-ui"),
            m("  →  ACC-WEB-003  ·  pre: feat-auth-login, feat-api-routes"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            m("schedule  "),
            t("wave 1: feat-db-schema  ·  wave 2: 2 features  ·  wave 3: feat-web-ui"),
        ]),
        Line::from(""),
    ]);
    chrome::transcript(f, body, lines);

    let inner = chrome::card(
        f,
        dock,
        "Launch workflow",
        &[("enter", "start workers"), ("esc", "cancel")],
    );
    let dock_lines = vec![
        Line::from("  wf-42  ·  4 features  ·  Step 0 seeded all runner outputs"),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            c("› ", BRIGHT),
            t("Start workers after Step 0"),
        ]),
        Line::from(vec![
            Span::raw("  "),
            m("Integration, progress log, and intents are seeded before worker 1 starts."),
        ]),
    ];
    f.render_widget(Paragraph::new(dock_lines), inner);
}
