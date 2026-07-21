//! Pure orchestration screens (F7).
//!
//! Most IDs stay tasks_pane / Dashboard analogues (pure-structure).
//! Mission Control parallel pack (user-locked 2026-07-21) owns:
//!   `workflow-monitor`, `workflow-monitor-failure`
//! Authority: mission `library/mission-control-parallel-design.md` +
//! Factory ref `/tmp/factory-mission-control-ref.png`. Brand `.: Mission Control`
//! under CortexNight with default pool 3 live / max 4. Sibling IDs
//! (launch/pool/board) remain for the parallel pack sibling feature.
//!
//! Source: `/tmp/grok-build-ref` tasks_pane + dashboard, pure-structure-law,
//! and MC parallel design supersession on owned home/failure IDs.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_putty, paint_demo_top_bar, paint_detail_footer,
    paint_detail_header, paint_dock_frame, paint_quiet_origin, paint_user_turn,
    reject_copper_residual, reject_structure_residual, DEMO_CWD, DEMO_CTX_TOTAL, DEMO_CTX_USED,
    MODEL_LABEL, PROMPT_ARROW,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme, CANVAS_COLUMNS};

pub fn render(id: &str) -> Result<Canvas, String> {
    match id {
        "mode-switch" => mode_switch(),
        "mode-switch-refused" => mode_switch_refused(),
        "workflow-launch" => workflow_launch(),
        "workflow-launch-blocked" => workflow_launch_blocked(),
        "workflow-monitor" => workflow_monitor(),
        "workflow-monitor-failure" => workflow_monitor_failure(),
        "team-board" => team_board(),
        "reviewer-output" => reviewer_output(),
        "reviewer-approved" => reviewer_approved(),
        "verification-ladder" => verification_ladder(),
        "intent-scorer-pass" => intent_scorer_pass(),
        "intent-scorer-fail" => intent_scorer_fail(),
        "concurrency-metrics" => concurrency_metrics(),
        "tool-envelope" => tool_envelope(),
        other => Err(format!("unknown orchestration pure-Cortex id: {other}")),
    }
}

/// Product pane title stack: authenticated top bar + short surface label.
fn pane_title(canvas: &mut Canvas, title: &str, subtitle: &str) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    canvas.write(2, 2, title, TextStyle::bold(ColorToken::Bright))?;
    if !subtitle.is_empty() {
        canvas.write(3, 2, subtitle, TextStyle::plain(ColorToken::Muted))?;
    }
    Ok(())
}

fn mode_switch() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Switch permission mode before the next tool batch.",
        "",
    )?;
    paint_dock_frame(
        &mut canvas,
        26,
        11,
        "Permission mode · Shift+Tab",
        "Enter applies · Esc keeps always-approve",
    )?;
    canvas.write(
        28,
        5,
        "Requested · plan",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        29,
        5,
        "Plan mode drafts a plan and waits for your go-ahead.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        30,
        5,
        "Scope · this session · model stays Cortex",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        32,
        5,
        "› Apply plan mode",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        33,
        5,
        "  Keep always-approve",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn mode_switch_refused() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_quiet_origin(
        &mut canvas,
        "Try YOLO mode on this locked workspace.",
        "",
    )?;
    paint_dock_frame(
        &mut canvas,
        26,
        11,
        "Mode change blocked",
        "Enter returns · Esc dismisses",
    )?;
    canvas.write(
        28,
        5,
        "yolo is off for this folder until trust is raised.",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        29,
        5,
        "Active mode stays always-approve.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        31,
        5,
        "› Return to conversation",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        32,
        5,
        "  Open folder trust settings",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Closest Cortex: launch linked subagents/tasks from a short plan (not Mission Control worksheet).
fn workflow_launch() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Tasks",
        "Prep linked background work for this session",
    )?;
    paint_user_turn(
        &mut canvas,
        5,
        &["Turn the release work into tracked tasks and walk me through it."],
    )?;
    paint_assistant(
        &mut canvas,
        7,
        &["I can open three linked items under Tasks:"],
    )?;
    canvas.write(
        9,
        2,
        "○ Ready · 3 proposed",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(11, 2, "▾ Subagents 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "○ Review · release notes open risks",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(14, 2, "▾ Tasks 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        15,
        4,
        "○ Task Verify staging and rollback notes",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(17, 2, "▾ Watchers 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        18,
        4,
        "○ Monitor next-step note after fixes",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        20,
        2,
        "✓ Workspace trusted · auth ready · tools available",
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Start these items?",
        "Enter starts · Esc returns to chat",
    )?;
    canvas.write(
        30,
        5,
        "› Start tasks",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Cancel · keep this plan ready",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn workflow_launch_blocked() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Tasks",
        "Prep linked background work for this session",
    )?;
    paint_user_turn(
        &mut canvas,
        5,
        &["Turn the release work into tracked tasks and walk me through it."],
    )?;
    paint_assistant(
        &mut canvas,
        7,
        &["Preflight found one missing step before tasks can run."],
    )?;
    canvas.write(
        9,
        2,
        "✗ Blocked · folder trust pending",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        2,
        "› Folder trust is still pending for /projects/demo",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        12,
        4,
        "Auth is ready · tools are available",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(14, 2, "▸ Subagents 0", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(15, 2, "▸ Tasks 0", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(16, 2, "▸ Watchers 0", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(
        18,
        2,
        "Add folder trust, then start the proposed items again.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Complete preflight first",
        "Enter opens trust · Esc returns",
    )?;
    canvas.write(
        30,
        5,
        "› Open folder trust",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Stay on blocked launch",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// MC parallel home (VAL-V3-MC-001): 3 live / max 4, multi-● Features rail.
fn workflow_monitor() -> Result<Canvas, String> {
    mission_control_home(McVariant::RunningAll)
}

/// MC parallel failure (VAL-V3-MC-003): one worker ✗ while siblings ● and run RUNNING.
fn workflow_monitor_failure() -> Result<Canvas, String> {
    mission_control_home(McVariant::OneFailed)
}

#[derive(Clone, Copy)]
enum McVariant {
    RunningAll,
    OneFailed,
}

/// Shared Mission Control parallel shell under CortexNight.
fn mission_control_home(variant: McVariant) -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_mc_top_bar(&mut canvas)?;

    // Status + hashed progress (Factory RUNNING anatomy).
    match variant {
        McVariant::RunningAll => {
            canvas.write(2, 2, "● RUNNING", TextStyle::bold(ColorToken::Running))?;
            canvas.write(
                2,
                13,
                "████████░░░░",
                TextStyle::plain(ColorToken::Running),
            )?;
            canvas.write(
                2,
                27,
                "18/42 features · 3 live workers",
                TextStyle::plain(ColorToken::Secondary),
            )?;
        }
        McVariant::OneFailed => {
            // Overall run stays RUNNING even with one failed worker.
            canvas.write(2, 2, "● RUNNING", TextStyle::bold(ColorToken::Running))?;
            canvas.write(
                2,
                13,
                "████████░░░░",
                TextStyle::plain(ColorToken::Running),
            )?;
            canvas.write(
                2,
                27,
                "17/42 features · 2 live · 1 failed",
                TextStyle::plain(ColorToken::Secondary),
            )?;
        }
    }

    // Column split: Workers left (~0-57), Features + Progress Log right (~59-119).
    let right = 59u16;
    let (workers_title, features_title) = match variant {
        McVariant::RunningAll => ("Workers (live) 3/4", "Features 18/42"),
        McVariant::OneFailed => ("Workers 3/4 · 1 failed", "Features 17/42"),
    };
    canvas.write(4, 2, workers_title, TextStyle::bold(ColorToken::Bright))?;
    canvas.write(4, right, features_title, TextStyle::bold(ColorToken::Bright))?;

    // --- Workers pane (always 3 cards; failure marks #22 ✗) ---
    let cards: [WorkerCard; 3] = match variant {
        McVariant::RunningAll => [
            WorkerCard {
                id: "#21",
                selected: true,
                mark: "●",
                feature: "note-open-risks",
                timer: "4m12s",
                skill: "v3-flow-worker",
                stream: "Execute cargo test note -- --nocapture",
            },
            WorkerCard {
                id: "#22",
                selected: false,
                mark: "●",
                feature: "safe-publish-path",
                timer: "1m40s",
                skill: "v3-flow-worker",
                stream: "Read staging/checklist.md (42 lines)",
            },
            WorkerCard {
                id: "#23",
                selected: false,
                mark: "●",
                feature: "window-os-actions",
                timer: "55s",
                skill: "v3-flow-worker",
                stream: "Execute npx playwright test window",
            },
        ],
        McVariant::OneFailed => [
            WorkerCard {
                id: "#21",
                selected: true,
                mark: "●",
                feature: "note-open-risks",
                timer: "4m12s",
                skill: "v3-flow-worker",
                stream: "Execute cargo test note -- --nocapture",
            },
            WorkerCard {
                id: "#22",
                selected: false,
                mark: "✗",
                feature: "safe-publish-path",
                timer: "1m40s",
                skill: "v3-flow-worker",
                stream: "Failed · staging checklist missing rollback",
            },
            WorkerCard {
                id: "#23",
                selected: false,
                mark: "●",
                feature: "window-os-actions",
                timer: "55s",
                skill: "v3-flow-worker",
                stream: "Execute npx playwright test window",
            },
        ],
    };
    for (i, card) in cards.iter().enumerate() {
        paint_worker_card(&mut canvas, 5 + (i as u16) * 4, card)?;
    }
    canvas.write(
        17,
        2,
        "pool free 1 · queued 2",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // --- Features rail (multi-● + optional #id badges) ---
    canvas.write(
        5,
        right,
        "  ✓ publish-checklist",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        6,
        right,
        "  ✓ staging-rollback",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        7,
        right,
        "› ● note-open-risks             #21",
        TextStyle::bold(ColorToken::Accent),
    )?;
    match variant {
        McVariant::RunningAll => {
            canvas.write(
                8,
                right,
                "  ● safe-publish-path           #22",
                TextStyle::plain(ColorToken::Running),
            )?;
        }
        McVariant::OneFailed => {
            canvas.write(
                8,
                right,
                "  ✗ safe-publish-path           #22",
                TextStyle::plain(ColorToken::Bad),
            )?;
        }
    }
    canvas.write(
        9,
        right,
        "  ● window-os-actions          #23",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        10,
        right,
        "  ○ github-actions-ci",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        11,
        right,
        "  ○ e2e-test-suite",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // Progress Log with Worker #id lines (Factory anatomy; capital L).
    canvas.write(
        13,
        right,
        "Progress Log               1-8 of 24",
        TextStyle::bold(ColorToken::Bright),
    )?;
    let log_lines: &[(&str, ColorToken)] = match variant {
        McVariant::RunningAll => &[
            ("8h ago Worker #21 started [note-open-risks]", ColorToken::Muted),
            ("7h ago Worker #22 started [safe-publish-path]", ColorToken::Muted),
            ("7h ago Worker #23 started [window-os-actions]", ColorToken::Muted),
            ("6h ago Milestone validation: ci-e2e", ColorToken::Dim),
            ("5h ago Worker #18 completed [publish-checklist]", ColorToken::Dim),
            ("4h ago Handoff items dismissed", ColorToken::Dim),
            ("3h ago Run started: CI features complete", ColorToken::Dim),
            ("2h ago Worker #19 completed [staging-rollback]", ColorToken::Dim),
        ],
        McVariant::OneFailed => &[
            ("8h ago Worker #21 started [note-open-risks]", ColorToken::Muted),
            ("7h ago Worker #22 started [safe-publish-path]", ColorToken::Muted),
            ("7h ago Worker #23 started [window-os-actions]", ColorToken::Muted),
            ("6m ago Worker #22 failed [safe-publish-path]", ColorToken::Bad),
            ("5m ago Handoff kept pool RUNNING", ColorToken::Muted),
            ("4m ago Worker #21 still streaming", ColorToken::Dim),
            ("3m ago Worker #23 still streaming", ColorToken::Dim),
            ("1m ago Pool remains 2 live · 1 free", ColorToken::Dim),
        ],
    };
    for (i, (line, tok)) in log_lines.iter().enumerate() {
        canvas.write(
            14 + i as u16,
            right,
            line,
            TextStyle::plain(*tok),
        )?;
    }

    // Focus detail (selected worker/feature) — Factory Active Feature + Active Worker blend.
    canvas.write(
        23,
        2,
        "Focus  #21  note-open-risks",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        24,
        2,
        "skill v3-flow-worker · milestone v3-mission-control-parallel",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(25, 2, "Preconditions", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        26,
        4,
        "mission-control-parallel-design.md present · harness-v4 builds",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        27,
        4,
        "Factory MC ref available",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(28, 2, "Expected Behavior", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        29,
        4,
        "Multi-worker home with Parallel 3/4 and Features multi-● badges",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        30,
        4,
        "Failure is per-worker; siblings keep running under RUNNING",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        32,
        2,
        "Stream  #21  note-open-risks                              Time 4m12s",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        33,
        2,
        "Execute  cargo test note -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        34,
        2,
        "Read  src/screens/orchestration.rs (214 lines)",
        TextStyle::plain(ColorToken::Path),
    )?;
    canvas.write(
        35,
        2,
        "Read  library/mission-control-parallel-design.md (98 lines)",
        TextStyle::plain(ColorToken::Path),
    )?;
    canvas.write(
        36,
        2,
        ".: Running multi-worker fixture paint first",
        TextStyle::plain(ColorToken::Accent),
    )?;

    paint_mc_footer(&mut canvas)?;
    finish_mc(canvas)
}

fn paint_mc_top_bar(canvas: &mut Canvas) -> Result<(), String> {
    // Brand amber (Path token = #ff9e64) + cwd muted + metrics + context budget.
    let brand = ".: Mission Control";
    canvas.write(0, 2, brand, TextStyle::bold(ColorToken::Path))?;
    let after_brand = 2 + brand.chars().count() as u16 + 2;
    canvas.write(
        0,
        after_brand,
        DEMO_CWD,
        TextStyle::plain(ColorToken::Muted),
    )?;
    // Right metrics strip ends with Parallel 3/4 + context budget for CROSS-028.
    let metrics = format!(
        "Time 32m12s · In 3.8K · Out 1.3M · Parallel 3/4 · {DEMO_CTX_USED} / {DEMO_CTX_TOTAL}"
    );
    let col = CANVAS_COLUMNS.saturating_sub(metrics.chars().count() as u16 + 2);
    canvas.write(0, col, &metrics, TextStyle::plain(ColorToken::Muted))?;
    Ok(())
}

fn paint_mc_footer(canvas: &mut Canvas) -> Result<(), String> {
    // Factory-like key row; putty-compatible middot separators (not exclusive
    // Shift+Tab product putty — CROSS-028 allows non-putty MC footers).
    canvas.write(
        39,
        1,
        "F Features · W Workers · P Pool · A Assign · I Intervene · Esc chat",
        TextStyle::plain(ColorToken::Dim),
    )?;
    Ok(())
}

struct WorkerCard {
    id: &'static str,
    selected: bool,
    mark: &'static str,
    feature: &'static str,
    timer: &'static str,
    skill: &'static str,
    stream: &'static str,
}

fn paint_worker_card(canvas: &mut Canvas, row: u16, card: &WorkerCard) -> Result<(), String> {
    let prefix = if card.selected { "› " } else { "  " };
    let head = format!("{}{} {} {}", prefix, card.id, card.mark, card.feature);
    let head_style = if card.selected {
        TextStyle::bold(ColorToken::Accent)
    } else if card.mark == "✗" {
        TextStyle::plain(ColorToken::Bad)
    } else {
        TextStyle::plain(ColorToken::Text)
    };
    canvas.write(row, 2, &head, head_style)?;
    // Timer right of card head within left pane (~col 48).
    canvas.write(row, 48, card.timer, TextStyle::plain(ColorToken::Muted))?;
    canvas.write(row + 1, 6, card.skill, TextStyle::plain(ColorToken::Secondary))?;
    canvas.write(row + 2, 6, card.stream, TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

fn finish_mc(canvas: Canvas) -> Result<Canvas, String> {
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    // Structure residual still applies (nameplates, 0mNNs, Tasks monitor invent).
    // Progress Log (capital L) and Mission Control brand are intentional MC pack.
    reject_structure_residual(&text)?;
    if !text.contains(".: Mission Control") {
        return Err("MC paint missing .: Mission Control brand".to_owned());
    }
    if !text.contains("Parallel 3/4") {
        return Err("MC paint missing Parallel 3/4 pool metric".to_owned());
    }
    if text.contains("Tasks monitor") || text.contains("Goals board") || text.contains("Owner ·")
    {
        return Err("MC paint residual: banned invent chrome".to_owned());
    }
    if text.contains("0m")
        && text.split_whitespace().any(|tok| {
            tok.len() >= 4
                && tok.starts_with("0m")
                && tok.ends_with('s')
                && tok[2..tok.len() - 1].chars().all(|c| c.is_ascii_digit())
        })
    {
        return Err("MC paint residual: 0mNNs timer".to_owned());
    }
    // Must not reintroduce copper journey residual.
    if text.contains("Claude") || text.contains("Cortex Code") {
        return Err("MC paint residual: Claude/Cortex wording".to_owned());
    }
    Ok(canvas)
}

/// Cortex Agent Dashboard: state-sorted agent rows + diamonds + dispatch ❯.
/// Not an Owner kanban / Goals board invent.
fn team_board() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Cortex · Dashboard - 3 agents · 1 awaiting",
        "State-sorted sessions · subagents stay under parent",
    )?;

    // Needs input first (filled diamond / ●)
    canvas.write(
        5,
        2,
        "› ◆ reviewer · safe publish path     Awaiting your input            2m",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        6,
        4,
        "Pending: approve rollback wording · branch master worktree",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // Working (diamond row markers from product dashboard)
    canvas.write(
        8,
        2,
        "  ◆ implementer · next-step note      Running: cargo test          12m",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        9,
        4,
        "Last tool · $ cargo test note -- --nocapture",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // Idle / completed (hollow diamond)
    canvas.write(
        11,
        2,
        "  ◇ notes · release open risks        idle · completed             48m",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        12,
        4,
        "✓ notes reviewed · no open blockers",
        TextStyle::plain(ColorToken::Ok),
    )?;

    // Dispatch jewel (product `❯` prompt)
    let box_top = 28u16;
    let left = 2u16;
    let right = 117u16;
    let width = (right - left + 1) as usize;
    let top = format!("╭{}╮", "─".repeat(width - 2));
    let bottom = format!("╰{}╯", "─".repeat(width - 2));
    canvas.write(box_top, left, &top, TextStyle::plain(ColorToken::Border))?;
    canvas.write(box_top + 2, left, &bottom, TextStyle::plain(ColorToken::Border))?;
    canvas.write(
        box_top + 1,
        left,
        "│",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        box_top + 1,
        right,
        "│",
        TextStyle::plain(ColorToken::Border),
    )?;
    let dispatch = format!("{PROMPT_ARROW} Dispatch a new agent");
    canvas.write(
        box_top + 1,
        left + 2,
        &dispatch,
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        32,
        2,
        "↑/↓ select · Enter open · Esc back · Ctrl+\\ toggle",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reviewer_output() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Plan review · changes requested",
        "Review · 2 complete · 1 remaining",
    )?;
    canvas.write(5, 2, "Changes requested", TextStyle::bold(ColorToken::Warn))?;
    canvas.write(
        6,
        2,
        "Next-step note still omits open risk and rollback order",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        7,
        2,
        "Affected · Safe publish path",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(9, 2, "Comments", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        10,
        4,
        "1. Name the open risk before marking the path ready.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "2. Add rollback steps next to the publish checklist.",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Next after review",
        "Enter creates fix · Esc returns to tasks",
    )?;
    canvas.write(
        30,
        5,
        "› Create a fix task",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Open verification summary",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reviewer_approved() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Plan approved · ready to continue",
        "Review · 3 complete · 0 remaining",
    )?;
    canvas.write(5, 2, "Summary", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        6,
        2,
        "Release notes, publish path, and next-step note are clear.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        7,
        2,
        "No open comments · no held blockers.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        9,
        2,
        "Completed items · 3/3",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        10,
        4,
        "✓ Review the release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        11,
        4,
        "✓ Check the safe publish path",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        12,
        4,
        "✓ Update the next-step note after fixes",
        TextStyle::plain(ColorToken::Ok),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Continue after approval?",
        "Enter continues · Esc stays on review",
    )?;
    canvas.write(
        30,
        5,
        "› Continue to chat",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Open dashboard",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn verification_ladder() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Advanced · Verification stages",
        "Self-check progress · secondary to Tasks / Dashboard",
    )?;
    canvas.write(
        7,
        4,
        "Origin · failure on the safe publish path",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        9,
        4,
        "✓ Deterministic checks · pass",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        10,
        4,
        "  Format, lint, type check, and build are clean.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        12,
        4,
        "✓ Agent-visible tests · pass",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        4,
        "  Automated cases that stay on this board still pass.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        15,
        4,
        "✗ Held-out checks · needs repair",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        16,
        4,
        "  One hidden assertion failed on the safe publish path.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        17,
        4,
        "  Current stage · repair before review continues.",
        TextStyle::plain(ColorToken::Warn),
    )?;
    canvas.write(
        19,
        4,
        "○ Intent audit · pending",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        20,
        4,
        "○ Process check · pending",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        23,
        4,
        "Enter opens detailed evidence · Esc returns",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · verification stays secondary to Tasks",
    )?;
    finish(canvas)
}

fn intent_scorer_pass() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Advanced · Verification summary",
        "Advisory outcome for goal fit",
    )?;
    canvas.write(
        7,
        4,
        "✓ Goal fit looks strong",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        8,
        4,
        "Release prep plan matches the stated safe-publish goal.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(10, 4, "Signals", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        11,
        4,
        "  ✓ Scope covers notes, path, and follow-up note",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        12,
        4,
        "  ✓ Risks named before publish advice",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        4,
        "  ✓ Rollback language present in the last repair",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        16,
        4,
        "Continue stays with the plan-approval dock above.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · advisory only under Advanced",
    )?;
    finish(canvas)
}

fn intent_scorer_fail() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Advanced · Verification summary",
        "Advisory outcome suggests a repair on goal fit",
    )?;
    canvas.write(
        7,
        4,
        "✗ Goal fit needs repair",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        8,
        4,
        "Next-step note still omits the open risk and rollback order.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(10, 4, "Signals", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        11,
        4,
        "  ✓ Scope mostly covers the release path",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        12,
        4,
        "  ✗ Risk language is incomplete in the latest note",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        13,
        4,
        "  ○ Rollback sequence pending the fix task",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        16,
        4,
        "Primary repair stays Create fix task on the review dock.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · repair on the plan-review surface",
    )?;
    finish(canvas)
}

/// Closest Cortex Dashboard active-agent density (not Mission runtime invent).
fn concurrency_metrics() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Cortex · Dashboard - 3 agents · 1 awaiting",
        "Runtime summary beside the agent list",
    )?;
    canvas.write(
        5,
        2,
        "◆ reviewer · safe publish path     Awaiting your input            2m",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        6,
        2,
        "◆ implementer · next-step note      Running: cargo test          12m",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        7,
        2,
        "◇ notes · release open risks        idle · completed             48m",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(10, 2, "Summary", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        11,
        4,
        "Elapsed · 12m40s · tokens ⇣ 48.2k this goal",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "Active · 1 agent · parallel rooms 1 / 3",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        13,
        4,
        &format!("Model · {MODEL_LABEL} · always-approve"),
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        16,
        2,
        &format!("{PROMPT_ARROW} Dispatch a new agent"),
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        18,
        2,
        "Enter open agent · Esc returns to Tasks",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · dashboard summary only under Advanced",
    )?;
    finish(canvas)
}

/// Tool evidence Latin form: `$ cmd`, never hybrid `Shell ·` invent.
fn tool_envelope() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Advanced · Tool evidence",
        "Focused tool payload with truncation and redaction marks",
    )?;
    canvas.write(
        7,
        4,
        "$ cargo test publish -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        8,
        4,
        "Exit · 1 · 4.2s · truncated at 12k / 48k bytes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(10, 4, "Excerpt", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        11,
        4,
        "  running 14 tests",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        12,
        4,
        "  test path::safe_publish ... ok",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        4,
        "  test path::rollback_note ... FAILED",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        14,
        4,
        "  assertion left missing steps[]",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "  … truncated …",
        TextStyle::plain(ColorToken::Dim),
    )?;
    canvas.write(17, 4, "Redacted", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        18,
        4,
        "  env CORTEX_DEPLOYMENT_KEY=••••••••",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        19,
        4,
        "  path /home/user/.cortex/auth.json (hidden)",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        22,
        4,
        "Page 1 of 3 · contiguous exact-byte windows",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Left/Right pages · Esc returns · no raw secret dump",
    )?;
    finish(canvas)
}

fn finish(canvas: Canvas) -> Result<Canvas, String> {
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    if text.contains("Claude") {
        return Err("orchestration residual: Claude wording".to_owned());
    }
    // Non-MC orchestration IDs still ban bare Mission Control brand.
    if text.contains("Mission Control") {
        return Err("orchestration residual: Mission Control outside MC pack".to_owned());
    }
    if text.lines().any(|l| l.trim() == "Mission") {
        return Err("orchestration residual: bare Mission title".to_owned());
    }
    // VAL-V3-STRUCT-009 bans (tasks_pane / dashboard family).
    for invent in [
        "Tasks monitor",
        "i intervene",
        "Goals board",
        "Progress log",
        "Owner ·",
        "Shell  ",
        "Shell ·",
        "Esc to tasks monitor",
        "Active work ·",
    ] {
        if text.contains(invent) {
            return Err(format!(
                "orchestration residual: banned invent {invent:?}"
            ));
        }
    }
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestration_batch_pure_cortex() {
        for id in [
            "mode-switch",
            "mode-switch-refused",
            "workflow-launch",
            "workflow-launch-blocked",
            "workflow-monitor",
            "workflow-monitor-failure",
            "team-board",
            "reviewer-output",
            "reviewer-approved",
            "verification-ladder",
            "intent-scorer-pass",
            "intent-scorer-fail",
            "concurrency-metrics",
            "tool-envelope",
        ] {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(!text.contains("Claude Opus"), "{id}");
            assert!(!text.contains("How Code should work"), "{id}");
            assert!(!text.contains("Cortex Code"), "{id}");
            assert!(!text.contains("Factory Mission"), "{id}");
            let is_mc = matches!(id, "workflow-monitor" | "workflow-monitor-failure");
            if is_mc {
                assert!(
                    text.contains(".: Mission Control"),
                    "{id} missing MC brand"
                );
                assert!(text.contains("Parallel 3/4"), "{id} missing pool metric");
            } else {
                assert!(
                    text.contains("Cortex")
                        || text.contains("Tasks")
                        || text.contains("Dashboard")
                        || text.contains("plan")
                        || text.contains("Shift+Tab")
                        || text.contains("Subagents")
                        || text.contains("Watchers")
                        || text.contains("$ cargo"),
                    "{id} lacks pure Cortex tasks/dashboard language"
                );
            }
            assert_eq!(text.lines().count(), 40, "{id}");
        }
    }

    #[test]
    fn orchestration_bans_mission_control_invent() {
        for id in [
            "workflow-monitor",
            "workflow-monitor-failure",
            "team-board",
            "workflow-launch",
            "workflow-launch-blocked",
            "concurrency-metrics",
            "tool-envelope",
        ] {
            let text = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            for invent in [
                "Tasks monitor",
                "i intervene",
                "Goals board",
                "Progress log",
                "Owner ·",
                "Shell  ",
                "Shell ·",
                "◆ Cortex",
                "❯ You",
            ] {
                assert!(
                    !text.contains(invent),
                    "{id} still paints invent {invent:?}"
                );
            }
            assert!(
                text.contains("9.5K / 500K") || text.contains("K / "),
                "{id} inherits M1 auth budget chrome"
            );
        }
    }

    /// VAL-V3-MC-001: multi-worker Mission Control home anatomy.
    #[test]
    fn mc_home_shows_multi_worker_parallel_pool() {
        let mon = render("workflow-monitor")
            .unwrap_or_else(|e| panic!("{e}"))
            .to_text();
        assert!(mon.contains(".: Mission Control"));
        assert!(mon.contains("Parallel 3/4"));
        assert!(mon.contains("● RUNNING"));
        assert!(mon.contains("18/42"));
        assert!(mon.contains("Workers (live) 3/4"));
        assert!(mon.contains("#21"));
        assert!(mon.contains("#22"));
        assert!(mon.contains("#23"));
        // ≥2 simultaneous live workers (fixture has 3).
        let live_marks = mon.matches("●").count();
        assert!(
            live_marks >= 2,
            "expected multi live ● markers, got {live_marks}"
        );
        assert!(mon.contains("Features"));
        assert!(mon.contains("Progress Log"));
        assert!(mon.contains("Worker #21") || mon.contains("Worker #22"));
        assert!(mon.contains("Focus"));
        assert!(mon.contains("skill"));
        assert!(mon.contains("milestone"));
        assert!(mon.contains("Preconditions"));
        assert!(mon.contains("Expected Behavior"));
        assert!(mon.contains("Execute"));
        assert!(mon.contains("Read"));
        assert!(mon.contains("F Features"));
        assert!(mon.contains("W Workers"));
        assert!(mon.contains("Esc chat"));
        assert!(!mon.contains("Tasks monitor"));
        assert!(!mon.contains("Subagents"));
    }

    /// VAL-V3-MC-003: one worker fails without killing the pool run.
    #[test]
    fn mc_failure_keeps_siblings_running() {
        let fail = render("workflow-monitor-failure")
            .unwrap_or_else(|e| panic!("{e}"))
            .to_text();
        assert!(fail.contains(".: Mission Control"));
        assert!(fail.contains("Parallel 3/4"));
        // Overall run remains RUNNING (not whole-mission kill).
        assert!(
            fail.contains("● RUNNING"),
            "run must stay RUNNING after per-worker fail"
        );
        assert!(fail.contains("✗"));
        assert!(fail.contains("#22"));
        // At least one other worker still live.
        assert!(
            fail.contains("#21") && fail.contains("#23"),
            "sibling workers must remain visible"
        );
        assert!(
            fail.contains("● note-open-risks")
                || fail.contains("#21 ●")
                || fail.lines().any(|l| l.contains("#21") && l.contains('●')),
            "worker #21 stays running"
        );
        assert!(
            fail.contains("● window-os-actions")
                || fail.lines().any(|l| l.contains("#23") && l.contains('●')),
            "worker #23 stays running"
        );
        assert!(fail.contains("failed") || fail.contains("Failed"));
        assert!(!fail.contains("Tasks monitor"));
        // Not a single-worker-only chrome.
        let worker_ids = ["#21", "#22", "#23"]
            .iter()
            .filter(|id| fail.contains(*id))
            .count();
        assert!(worker_ids >= 2, "need multi-worker presence after fail");
    }

    #[test]
    fn tasks_pane_and_dashboard_language_visible() {
        // MC pack owns monitor home; sibling board/envelope keep dashboard/tool language.
        let mon = render("workflow-monitor").unwrap().to_text();
        assert!(mon.contains(".: Mission Control"));
        assert!(mon.contains("Workers (live)"));
        assert!(!mon.contains("Tasks monitor"));
        assert!(!mon.contains("i intervene"));

        let board = render("team-board").unwrap().to_text();
        assert!(board.contains("Dashboard"));
        assert!(board.contains("Dispatch a new agent") || board.contains(&PROMPT_ARROW.to_string()));
        assert!(board.contains("◆") || board.contains("◇"));
        assert!(!board.contains("Goals board"));
        assert!(!board.contains("Owner ·"));

        let env = render("tool-envelope").unwrap().to_text();
        assert!(env.contains("$ cargo test publish"));
        assert!(!env.contains("Shell ·"));
        assert!(!env.contains("Shell  "));
    }
}
