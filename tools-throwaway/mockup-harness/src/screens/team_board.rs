//! Team task board. The stage-sorted table is the single task-state source.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::chrome::{self, c, m, t, StatusLine, BAD, BRIGHT, FAINT, MUTED, OK};

const TASKS: &[Task] = &[
    Task {
        id: "T-106",
        stage: Stage::Blocked,
        task: "migrate sessions table to v3",
        owner: "unassigned",
        detail: "failed dependency: field missing",
    },
    Task {
        id: "T-102",
        stage: Stage::Active,
        task: "add rate-limit middleware to /login",
        owner: "ws-77b1e0",
        detail: "tests in progress",
    },
    Task {
        id: "T-103",
        stage: Stage::Claimed,
        task: "extract token refresh service",
        owner: "ws-3f9a2c",
        detail: "depends on T-100",
    },
    Task {
        id: "T-101",
        stage: Stage::Submitted,
        task: "wire session revocation endpoint",
        owner: "ws-2c4d10",
        detail: "awaiting review",
    },
    Task {
        id: "T-104",
        stage: Stage::Ready,
        task: "add audit-log sink for auth events",
        owner: "unassigned",
        detail: "depends on T-102",
    },
    Task {
        id: "T-105",
        stage: Stage::Ready,
        task: "rotate signing keys on deploy",
        owner: "unassigned",
        detail: "depends on T-100",
    },
    Task {
        id: "T-107",
        stage: Stage::Open,
        task: "backfill legacy password hashes",
        owner: "unassigned",
        detail: "no dependency",
    },
    Task {
        id: "T-100",
        stage: Stage::Closed,
        task: "create sessions table and schema",
        owner: "ws-3f9a2c",
        detail: "✓ accepted",
    },
    Task {
        id: "T-099",
        stage: Stage::Closed,
        task: "scaffold auth crate and wiring",
        owner: "ws-2c4d10",
        detail: "✓ accepted",
    },
];

struct Task {
    id: &'static str,
    stage: Stage,
    task: &'static str,
    owner: &'static str,
    detail: &'static str,
}

enum Stage {
    Open,
    Ready,
    Claimed,
    Active,
    Submitted,
    Closed,
    Blocked,
}

pub fn render(f: &mut Frame) {
    let status = StatusLine::default();
    let body = chrome::cli_view(f, "team board · wf-42", &status);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(17),
            Constraint::Length(11),
            Constraint::Min(0),
        ])
        .split(body);

    let mut board = vec![
        Line::from(header_keys()),
        Line::from(""),
        pipeline_ribbon(),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            t("task board"),
            m(" · stage-sorted work first, closed last"),
        ]),
        chrome::rule_line(105),
        task_head(),
    ];
    board.extend(TASKS.iter().map(task_row));
    board.push(Line::from(""));
    f.render_widget(Paragraph::new(board), rows[0]);

    let inner = chrome::card(
        f,
        rows[1],
        "Focused item T-102 · contract · coordinator assignments",
        &[],
    );
    f.render_widget(Paragraph::new(contract_rows()), inner);

    let events = chrome::rule_head(f, rows[2], "recent board activity");
    f.render_widget(Paragraph::new(activity_rows()), events);
}

fn header_keys() -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw("   ")];
    spans.extend(chrome::key_span("c", "claim"));
    spans.push(c(" · ", FAINT));
    spans.push(m("READY rows only"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("enter", "inspect item"));
    spans.push(c("  ·  ", FAINT));
    spans.extend(chrome::key_span("j/k", "move"));
    spans
}

fn pipeline_ribbon() -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        c("○ OPEN 1", MUTED),
        c("  →  ", FAINT),
        c("○ READY 2", MUTED),
        c("  →  ", FAINT),
        c("● CLAIMED 1", MUTED),
        c("  →  ", FAINT),
        c("● ACTIVE 1", MUTED),
        c("  →  ", FAINT),
        c("● SUBMITTED 1", MUTED),
        c("  →  ", FAINT),
        c("✓ CLOSED 2", OK),
        c("  ·  ", FAINT),
        c("✗ BLOCKED 1", BAD),
    ])
}

fn task_head() -> Line<'static> {
    Line::from(vec![
        Span::raw("     "),
        m("ID      "),
        m("STATE               "),
        m("TASK                                 "),
        m("OWNER       "),
        m("DEPENDS / ARTIFACT"),
    ])
}

fn task_row(task: &Task) -> Line<'static> {
    let marker = if matches!(task.stage, Stage::Blocked) {
        c("│ ", BAD)
    } else {
        Span::raw("  ")
    };
    let mut spans = vec![
        Span::raw("   "),
        marker,
        t(&format!("{:<8}", task.id)),
    ];
    spans.extend(stage_cell(&task.stage));
    spans.push(t(&format!("{:<37}", task.task)));
    spans.push(m(&format!("{:<12}", task.owner)));
    if matches!(task.stage, Stage::Closed) {
        spans.push(c(task.detail, OK));
    } else {
        spans.push(m(task.detail));
    }
    Line::from(spans)
}

fn stage_cell(stage: &Stage) -> Vec<Span<'static>> {
    let (glyph, word, hue, claimable) = match stage {
        Stage::Open => ("○", "OPEN", MUTED, false),
        Stage::Ready => ("○", "READY", MUTED, true),
        Stage::Claimed => ("●", "CLAIMED", MUTED, false),
        Stage::Active => ("●", "ACTIVE", MUTED, false),
        Stage::Submitted => ("●", "SUBMITTED", MUTED, false),
        Stage::Closed => ("✓", "CLOSED", OK, false),
        Stage::Blocked => ("✗", "BLOCKED", BAD, false),
    };
    let claim = if claimable { " · c claim" } else { "" };
    let state = format!("{glyph} {word}{claim}");
    vec![c(&format!("{state:<20}"), hue)]
}

fn contract_rows() -> Vec<Line<'static>> {
    vec![
        contract_section_row("id        T-102", "Coordinator"),
        contract_row("task      rate-limit middleware for /login", "posts contracts · workers claim work"),
        contract_row("expected  429 on the 6th login attempt; tests pass", "core moves state · no self-closing"),
        contract_row("tools     read · edit · run", "reconcile result against expected"),
        contract_row("forbidden crates/db/** · production config", "single owner per active work"),
        contract_section_row("depends   T-100 · accepted", "Assignments"),
        contract_row("owner     ws-77b1e0", "ws-77b1e0 → T-102"),
        contract_row("result    status + artifact reference", "ws-3f9a2c → T-103"),
        contract_row("scope     login request path only", "ws-2c4d10 → T-101"),
    ]
}

fn contract_row(left: &str, right: &str) -> Line<'static> {
    contract_row_with_right(left, m(right))
}

fn contract_section_row(left: &str, heading: &str) -> Line<'static> {
    contract_row_with_right(left, c(heading, BRIGHT))
}

fn contract_row_with_right(left: &str, right: Span<'static>) -> Line<'static> {
    const DIVIDER_COL: usize = 60;
    let used = 3 + left.chars().count();
    let pad = " ".repeat(DIVIDER_COL.saturating_sub(used));
    Line::from(vec![
        Span::raw("   "),
        t(left),
        Span::raw(pad),
        c("│", FAINT),
        Span::raw("  "),
        right,
    ])
}

fn activity_rows() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::raw("   "),
            m("TIME    "),
            m("WHO          "),
            m("EVENT                    "),
            m("NOTE"),
        ]),
        activity("14:02", "ws-77b1e0", "began T-102 work", "rate-limit work started"),
        activity("14:01", "core", "posted T-104 and T-105", "both can be claimed"),
        activity("14:00", "ws-2c4d10", "sent T-101 for review", "awaiting coordinator review"),
    ]
}

fn activity(time: &str, who: &str, event: &str, note: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("   "),
        m(&format!("{time:<8}")),
        t(&format!("{who:<13}")),
        t(&format!("{event:<25}")),
        m(note),
    ])
}
