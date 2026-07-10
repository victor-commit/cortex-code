use ratatui::{
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, ACCENT, BAD, BRIGHT, FAINT, MUTED, OK};

const PRESENT_AUTHOR_TIME_INPUTS: &[&str] = &[
    "architecture.md",
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

    let mut recovery = chrome::key_span("r", "re-run preflight");
    recovery.push(c(&format!(" {} ", chrome::SEP), FAINT));
    recovery.extend(chrome::key_span("esc", "close"));

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
            m("levers  parallelism "),
            c("[N=3]", BRIGHT),
            c(" p", ACCENT),
            m(" edit · budget "),
            c("[$2.00]", BRIGHT),
            c(" b", ACCENT),
            m(" edit"),
        ]),
        Line::from(""),
        chrome::agent_head("Preflight · Step 0"),
        chrome::fail_head("MISSING acceptance-contract.md"),
    ];
    lines.extend(chrome::fail_rows(vec![
        vec![
            c("MISSING <artifact>; exit 1", BAD),
            m("  "),
            t("Create acceptance-contract.md from the approved mission contract."),
        ],
        recovery,
    ]));
    lines.extend(PRESENT_AUTHOR_TIME_INPUTS.iter().map(|artifact| {
        Line::from(vec![
            Span::raw("   "),
            c("✓ ", OK),
            t(artifact),
            m("  ·  present and valid"),
        ])
    }));
    lines.extend([
        chrome::agent_head("Runner outputs"),
        Line::from(vec![
            Span::raw("   "),
            c("○ ", MUTED),
            t("workflow/wf-42/integration"),
            m("  ·  not seeded"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("○ ", MUTED),
            t("progress-log.jsonl"),
            m("  ·  not seeded"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("○ ", MUTED),
            t("intents/"),
            m("  ·  not seeded"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("✗ ", BAD),
            t("Preflight FAILED · 1 failed · 7/8 author-time inputs present and valid"),
        ]),
        Line::from(vec![
            Span::raw("   "),
            c("○ ", MUTED),
            t("0/3 runner outputs seeded · Step 0 stopped before seeding"),
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
    ]);
    chrome::transcript(f, body, lines);

    let inner = chrome::card(
        f,
        dock,
        "Workflow launch blocked",
        &[],
    );
    let dock_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            m("Launch disabled until preflight passes. No worker was started."),
        ]),
        Line::from(""),
        Line::from("  Fix the pinned missing artifact, then return to preflight."),
        Line::from("  All three runner outputs remain neutral and not seeded."),
    ];
    f.render_widget(Paragraph::new(dock_lines), inner);
}
