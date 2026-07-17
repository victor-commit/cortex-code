//! Pure orchestration screens (F7) — pure-structure M3 analogues.
//!
//! Gallery IDs stay fixed. Paint is closest real Cortex surfaces from
//! `tasks_pane` (Subagents / Tasks / Watchers) and Dashboard (agent rows,
//! diamonds, dispatch ❯) — never Mission Control invent:
//! - no primary "Tasks monitor" product chrome
//! - no `i intervene` binding
//! - no Owner kanban "Goals board"
//! - no "Progress log" mission ticker as product chrome
//! - no residual `Shell  cmd` / hybrid Shell tool header
//!
//! Source: `/tmp/grok-build-ref` views/tasks_pane.rs + dashboard docs,
//! mission `library/pure-structure-law.md` + fidelity-audit-views-orchestration.md.

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_assistant, paint_default_prompt, paint_default_putty, paint_demo_top_bar,
    paint_detail_footer, paint_detail_header, paint_dock_frame, paint_quiet_origin,
    paint_user_turn, reject_copper_residual, reject_structure_residual, MODEL_LABEL, PROMPT_ARROW,
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

/// Cortex `tasks_pane`: collapsible Subagents / Tasks / Watchers (not Tasks monitor invent).
fn workflow_monitor() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Tasks",
        "Background work for this session · running first",
    )?;
    canvas.write(
        5,
        2,
        "● 1 running · 1 complete · 1 queued",
        TextStyle::plain(ColorToken::Running),
    )?;

    // Subagents group
    canvas.write(7, 2, "▾ Subagents 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        8,
        4,
        "› ● Review · release notes open risks",
        TextStyle::bold(ColorToken::Accent),
    )?;
    canvas.write(
        9,
        6,
        "Running · scanning CHANGELOG risk callouts",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // Tasks group
    canvas.write(11, 2, "▾ Tasks 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "  ✓ Task Verify staging and rollback notes",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        6,
        "Finished · 14s · (82 lines)",
        TextStyle::plain(ColorToken::Dim),
    )?;

    // Watchers group
    canvas.write(15, 2, "▾ Watchers 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        16,
        4,
        "  ○ Monitor next-step note after fixes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        17,
        6,
        "Queued · starts when publish path clears",
        TextStyle::plain(ColorToken::Dim),
    )?;

    canvas.write(
        20,
        2,
        "Enter view output · k kill · Esc resume chat",
        TextStyle::plain(ColorToken::Dim),
    )?;
    paint_default_prompt(&mut canvas, 33, "Ask about the active task…")?;
    paint_default_putty(&mut canvas)?;
    finish(canvas)
}

/// Failed / needs-attention rows still use tasks_pane groups (not Mission invent).
fn workflow_monitor_failure() -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    pane_title(
        &mut canvas,
        "Tasks",
        "Background work for this session · failures first",
    )?;
    canvas.write(
        5,
        2,
        "✗ 1 failed · 1 complete · 1 queued",
        TextStyle::plain(ColorToken::Bad),
    )?;

    canvas.write(7, 2, "▾ Subagents 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        8,
        4,
        "› ✗ Review · safe publish path",
        TextStyle::bold(ColorToken::Bad),
    )?;
    canvas.write(
        9,
        6,
        "Failed · staging checklist missing the rollback step",
        TextStyle::plain(ColorToken::Text),
    )?;

    canvas.write(11, 2, "▾ Tasks 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        12,
        4,
        "  ✓ Task Review release notes for open risks",
        TextStyle::plain(ColorToken::Ok),
    )?;
    canvas.write(
        13,
        6,
        "Finished · 48s",
        TextStyle::plain(ColorToken::Dim),
    )?;

    canvas.write(15, 2, "▾ Watchers 1", TextStyle::bold(ColorToken::Bright))?;
    canvas.write(
        16,
        4,
        "  ○ Monitor next-step note after fixes",
        TextStyle::plain(ColorToken::Muted),
    )?;
    canvas.write(
        17,
        6,
        "Queued · waiting on failed review",
        TextStyle::plain(ColorToken::Dim),
    )?;

    paint_dock_frame(
        &mut canvas,
        26,
        11,
        "Repair this item?",
        "Enter creates fix · Esc keeps failure visible",
    )?;
    canvas.write(
        28,
        5,
        "Create a follow-up for the missing rollback note.",
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
    if text.contains("Claude") || text.contains("Mission Control") {
        return Err("orchestration residual: Claude/Mission Control wording".to_owned());
    }
    if text.lines().any(|l| l.trim() == "Mission") {
        return Err("orchestration residual: bare Mission title".to_owned());
    }
    // VAL-V3-STRUCT-009 bans.
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

    #[test]
    fn tasks_pane_and_dashboard_language_visible() {
        let mon = render("workflow-monitor").unwrap().to_text();
        assert!(mon.contains("Subagents"));
        assert!(mon.contains("Tasks"));
        assert!(mon.contains("Watchers"));
        assert!(mon.contains("Task "));
        assert!(mon.contains("Monitor "));
        assert!(!mon.contains("Active work"));
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
