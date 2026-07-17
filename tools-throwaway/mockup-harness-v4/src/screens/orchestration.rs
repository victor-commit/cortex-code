//! Pure orchestration screens (F7).
//!
//! The inventory keeps the 14 historical IDs for continuity, but paint is
//! Cortex Tasks / Dashboard / Goals / plan-approval chrome — never Factory
//! Mission Control copper residual.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_default_prompt, paint_default_putty, paint_demo_top_bar, paint_detail_footer,
    paint_detail_header, paint_dock_frame, paint_quiet_origin, paint_user_turn,
    reject_copper_residual, MODEL_LABEL,
};
use crate::chrome::tokens::{ColorToken, TextStyle, Theme};

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

fn shell_goal_header(canvas: &mut Canvas, goal: &str) -> Result<(), String> {
    paint_demo_top_bar(canvas)?;
    canvas.write(3, 4, "Tasks", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(4, 4, goal, TextStyle::plain(ColorToken::Secondary))?;
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

fn workflow_launch() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    paint_user_turn(
        &mut canvas,
        6,
        &["Turn the release work into tracked tasks and walk me through it."],
    )?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        11,
        4,
        "I can run that as three linked tasks under one goal.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "○ Ready · Goal launch  0/3",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        15,
        4,
        "Preflight",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        16,
        4,
        "✓ Workspace trusted · auth ready · tools available",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        17,
        4,
        "No missing inputs. Start opens Tasks monitoring.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        19,
        4,
        "Tasks proposed",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        20,
        4,
        "  1. Review release notes for open risks",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        21,
        4,
        "  2. Check the safe publish path",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        22,
        4,
        "  3. Update the next-step note after fixes",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_dock_frame(
        &mut canvas,
        28,
        9,
        "Start this goal?",
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
        "  Cancel · keep the proposal ready",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn workflow_launch_blocked() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    paint_user_turn(
        &mut canvas,
        6,
        &["Turn the release work into tracked tasks and walk me through it."],
    )?;
    canvas.write(10, 2, "◆ Cortex", TextStyle::accent())?;
    canvas.write(
        11,
        4,
        "Preflight found one missing step before tasks can run.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "✗ Blocked · Goal launch  0/3",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        15,
        4,
        "Needs attention · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        16,
        4,
        "› Folder trust is still pending for /projects/demo",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        17,
        4,
        "  Auth is ready · tools are available",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        19,
        4,
        "Checklist",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        20,
        4,
        "  Add folder trust, then Start tasks again.",
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

fn workflow_monitor() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    canvas.write(
        6,
        4,
        "● Running · Tasks monitor  1/3",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        7,
        4,
        "Running · 1 complete · 2 remaining · no blocker",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Active work · 1/3",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        10,
        4,
        "› ● Check the safe publish path",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        11,
        4,
        "  Verifying staging and rollback notes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        13,
        4,
        "Recently completed · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "  ✓ Review the release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        16,
        4,
        "Queued · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        17,
        4,
        "  ○ Update the next-step note after fixes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        19,
        4,
        "Progress log · 1-3 of 12",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        20,
        4,
        "  Started safe publish path checks · 2m ago",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        21,
        4,
        "  Finished release notes review · 8m ago",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        22,
        4,
        "  Preflight passed · three planned tasks",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        25,
        4,
        "Enter to open task · i intervene · Esc resume chat",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "Ask about the active task…")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn workflow_monitor_failure() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    canvas.write(
        6,
        4,
        "✗ Needs attention · Tasks monitor  1/3",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        7,
        4,
        "1 failed · 1 complete · 1 remaining",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Needs attention · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        10,
        4,
        "› ✗ Check the safe publish path",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        11,
        4,
        "  Staging checklist still missing the rollback step",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        13,
        4,
        "Completed · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "  ✓ Review the release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        16,
        4,
        "Queued · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        17,
        4,
        "  ○ Update the next-step note after fixes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    paint_dock_frame(
        &mut canvas,
        26,
        11,
        "Repair this task?",
        "Enter creates fix · Esc keeps failure visible",
    )?;
    canvas.write(
        28,
        5,
        "Create a fix task for the missing rollback note.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        30,
        5,
        "› Create fix task",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        31,
        5,
        "  Open evidence",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        32,
        5,
        "  Resume conversation",
        TextStyle::plain(ColorToken::Text),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn team_board() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    canvas.write(
        6,
        4,
        "✗ Needs attention · Goals board  1/3",
        TextStyle::plain(ColorToken::Bad),
    )?;
    canvas.write(
        7,
        4,
        "Ownership and assignments for the current goal",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        9,
        4,
        "Needs attention · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        10,
        4,
        "› ✗ Check the safe publish path · Review · blocked",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        11,
        4,
        "  Depends on · next-step note and review",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        13,
        4,
        "Working · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "  ● Update the next-step note after fixes",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        15,
        4,
        "  Owner · Notes agent · working",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        17,
        4,
        "Completed · 1",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        18,
        4,
        "  ✓ Review the release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        19,
        4,
        "  Owner · Docs agent · complete",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        22,
        4,
        "Esc to tasks monitor · Enter opens selection",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

fn reviewer_output() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    canvas.write(
        6,
        4,
        "Plan review · changes requested",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        7,
        4,
        "Review · 2 complete · 1 remaining",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Changes requested",
        TextStyle::bold(ColorToken::Warn),
    )?;
    canvas.write(
        10,
        4,
        "Next-step note still omits open risk and rollback order",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "Affected · Safe publish path",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        13,
        4,
        "Comments",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "  1. Name the open risk before marking the path ready.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        15,
        4,
        "  2. Add rollback steps next to the publish checklist.",
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
    shell_goal_header(
        &mut canvas,
        "Prepare the release safely and review the result.",
    )?;
    canvas.write(
        6,
        4,
        "Plan approved · ready to continue",
        TextStyle::bold(ColorToken::Ok),
    )?;
    canvas.write(
        7,
        4,
        "Review · 3 complete · 0 remaining",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        9,
        4,
        "Summary",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        10,
        4,
        "Release notes, publish path, and next-step note are clear.",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "No open comments · no held blockers.",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        13,
        4,
        "Completed tasks · 3/3",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(
        14,
        4,
        "  ✓ Review the release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        15,
        4,
        "  ✓ Check the safe publish path",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        16,
        4,
        "  ✓ Update the next-step note after fixes",
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
        "  Open dashboard telemetry",
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
        "Self-check progress for the active goal · counts stay compact",
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
        "Enter opens detailed evidence · Esc returns to failure",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · verification stays secondary to the tasks board",
    )?;
    finish(canvas)
}

fn intent_scorer_pass() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Advanced · Verification summary",
        "Advisory outcome for goal fit · not a factory score ladder",
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
    canvas.write(
        10,
        4,
        "Signals",
        TextStyle::bold(ColorToken::Bright),
    )?;
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
    canvas.write(
        10,
        4,
        "Signals",
        TextStyle::bold(ColorToken::Bright),
    )?;
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

fn concurrency_metrics() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_detail_header(
        &mut canvas,
        "Dashboard · Runtime tiles",
        "Time, tokens, and concurrent agent activity for this goal",
    )?;
    canvas.write(
        7,
        4,
        "Tiles · 4",
        TextStyle::bold(ColorToken::Bright),
    )?;
    // Four compact dashboard tiles.
    canvas.write(
        9,
        4,
        "╭ Time ──────────────╮  ╭ Tokens ────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        10,
        4,
        "│ Elapsed · 12m 40s  │  │ ⇣ 48.2k this goal  │",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        11,
        4,
        "│ Active · 1 agent   │  │ Context · 8.5k     │",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        12,
        4,
        "╰────────────────────╯  ╰────────────────────╯",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        14,
        4,
        "╭ Concurrency ───────╮  ╭ Queue ─────────────╮",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        15,
        4,
        "│ Parallel · 1 / 3   │  │ Pending · 1 task   │",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        16,
        4,
        "│ Soft cap · 3       │  │ Send now available │",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        17,
        4,
        "╰────────────────────╯  ╰────────────────────╯",
        TextStyle::plain(ColorToken::Border),
    )?;
    canvas.write(
        20,
        4,
        &format!("Model · {MODEL_LABEL} · always-approve"),
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(
        22,
        4,
        "Telemetry is secondary · Tasks monitor stays the home",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_detail_footer(
        &mut canvas,
        "Esc to return · dashboard tiles only under Advanced",
    )?;
    finish(canvas)
}

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
        "Shell · cargo test publish -- --nocapture",
        TextStyle::plain(ColorToken::Running),
    )?;
    canvas.write(
        8,
        4,
        "Exit · 1 · 4.2s · truncated at 12k / 48k bytes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        10,
        4,
        "Excerpt",
        TextStyle::bold(ColorToken::Bright),
    )?;
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
    canvas.write(
        17,
        4,
        "Redacted",
        TextStyle::bold(ColorToken::Bright),
    )?;
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
    if text.contains("Claude") || text.contains("Mission Control") {
        return Err("orchestration residual: Claude/Mission Control wording".to_owned());
    }
    // Title may still use "Tasks" / plan language, not Factory Mission Control brand.
    if text.lines().any(|l| l.trim() == "Mission") {
        return Err("orchestration residual: bare Mission title".to_owned());
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
            assert!(
                text.contains("Cortex")
                    || text.contains("Tasks")
                    || text.contains("Dashboard")
                    || text.contains("plan")
                    || text.contains("Shift+Tab")
                    || text.contains("Goal"),
                "{id} lacks pure Cortex tasks/plan language"
            );
            assert_eq!(text.lines().count(), 40, "{id}");
        }
    }
}
