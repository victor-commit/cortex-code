//! Pure orchestration screens (F7).
//!
//! Mission Control V2 premium (user-locked 2026-07-22) owns:
//!   `workflow-monitor` (+ `workflow-monitor-failure` same DNA)
//! as the **only** full MC product surface.
//! Secondary former MC destinations collapse to non-primary stubs:
//!   `concurrency-metrics`, `workflow-launch`, `workflow-launch-blocked`,
//!   `team-board`
//! Residual advanced IDs (scorer/ladder/reviewer/envelope/mode-switch) stay
//! thin non-MC detail surfaces and must not compete as alternate MC homes.
//! Authority: mission `library/mission-control-v2-premium-fable5-note.md`;
//! Factory ref `/tmp/factory-mission-control-ref.png`. Brand
//! `.: Mission Control` under CortexNight on the primary home only (+ stubs that
//! point home). Home paint is orange + 3 grays only (Path accent; Ok/Bad
//! terminal only).

use crate::chrome::canvas::Canvas;
use crate::chrome::components::{
    paint_default_putty, paint_demo_top_bar, paint_detail_footer, paint_detail_header,
    paint_dock_frame, paint_quiet_origin, reject_copper_residual, reject_structure_residual,
    DEMO_CWD, DEMO_CTX_TOTAL, DEMO_CTX_USED,
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

/// VAL-V3-MC-005 residual: assign absorbed on home (`A Assign`). Short pointer only.
fn workflow_launch() -> Result<Canvas, String> {
    mc_home_pointer_stub(McStubKind::Launch)
}

/// VAL-V3-MC-005 residual: pool-full / blocked signal points home — not a second dashboard.
fn workflow_launch_blocked() -> Result<Canvas, String> {
    mc_home_pointer_stub(McStubKind::LaunchBlocked)
}

/// MC V2 premium home (VAL-V3-MC-004): Factory-grade parallel Mission Control.
fn workflow_monitor() -> Result<Canvas, String> {
    mission_control_home(McVariant::RunningAll)
}

/// MC parallel failure (VAL-V3-MC-003 / DNA of MC-004): one worker ✗ while siblings ●.
fn workflow_monitor_failure() -> Result<Canvas, String> {
    mission_control_home(McVariant::OneFailed)
}

#[derive(Clone, Copy)]
enum McVariant {
    RunningAll,
    OneFailed,
}

/// Secondary orch destinations collapsed under VAL-V3-MC-005 (non-primary stubs).
#[derive(Clone, Copy)]
enum McStubKind {
    Concurrency,
    Launch,
    LaunchBlocked,
    TeamBoard,
}

/// Factory MC accent is orange Path (`#ff9e64`). Premium home paints Path + grays
/// only; Ok/Bad reserved for terminal ✓/✗. No cyan Running / purple Accent panels.
const MC_ORANGE: ColorToken = ColorToken::Path;

/// Thin home-pointer residual for former multi-page MC destinations.
/// Must NOT paint full multi-panel home chrome (Workers pane + Features + Stream).
fn mc_home_pointer_stub(kind: McStubKind) -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    // Quiet residual top bar: brand + cwd + budget only (no full MC metrics lane).
    let brand = ".: Mission Control";
    canvas.write(0, 2, brand, TextStyle::bold(MC_ORANGE))?;
    let after_brand = 2 + brand.chars().count() as u16 + 2;
    canvas.write(
        0,
        after_brand,
        DEMO_CWD,
        TextStyle::plain(ColorToken::Muted),
    )?;
    let budget = format!("{DEMO_CTX_USED} / {DEMO_CTX_TOTAL}");
    let col = CANVAS_COLUMNS.saturating_sub(budget.chars().count() as u16 + 2);
    canvas.write(0, col, &budget, TextStyle::plain(ColorToken::Muted))?;

    paint_mc_rule(&mut canvas, 2)?;

    let (title, line1, line2, line3, primary) = match kind {
        McStubKind::Concurrency => (
            "Pool detail absorbed on Mission Control home",
            "Parallel 3/4 · live/free/queued · slot cells live in the home header.",
            "Open Mission Control home for Workers, Features, and Stream.",
            "This residual is not a second pool dashboard.",
            "› Open Mission Control home",
        ),
        McStubKind::Launch => (
            "Assign absorbed on Mission Control home",
            "Use A Assign on home when a free slot is ready (queue depth 2).",
            "Parallel 3/4 stays on the single Mission Control page.",
            "This residual is not a launch tour or multi-panel board.",
            "› Open Mission Control home",
        ),
        McStubKind::LaunchBlocked => (
            "Assign blocked · return to Mission Control home",
            "Pool full or a precondition failed: free a slot or fix preflight on home.",
            "Parallel 4/4 saturation is visible from the home header when full.",
            "This residual is not a competing blocked-launch dashboard.",
            "› Open Mission Control home",
        ),
        McStubKind::TeamBoard => (
            "Assignment map absorbed on Mission Control home",
            "Feature-to-worker pairing lives in Workers + Features on home.",
            "Parallel 3/4 multi-● rows stay on the single Mission Control page.",
            "This residual is not a second assignment product or tour page.",
            "› Open Mission Control home",
        ),
    };

    canvas.write(4, 2, title, TextStyle::bold(ColorToken::Bright))?;
    canvas.write(6, 2, line1, TextStyle::plain(ColorToken::Text))?;
    canvas.write(7, 2, line2, TextStyle::plain(ColorToken::Muted))?;
    canvas.write(8, 2, line3, TextStyle::plain(ColorToken::Dim))?;

    canvas.write(11, 2, primary, TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        12,
        2,
        "  Stay on residual · no multi-panel Mission Control here",
        TextStyle::plain(ColorToken::Muted),
    )?;

    canvas.write(
        39,
        1,
        "Esc Mission Control home · residual stub only",
        TextStyle::plain(ColorToken::Dim),
    )?;

    finish_mc_stub(canvas, kind)
}

fn finish_mc_stub(canvas: Canvas, kind: McStubKind) -> Result<Canvas, String> {
    let text = canvas.to_text();
    reject_copper_residual(&text)?;
    reject_structure_residual(&text)?;
    if !text.contains(".: Mission Control") {
        return Err("MC stub missing .: Mission Control brand".to_owned());
    }
    if !text.contains("Open Mission Control home") {
        return Err("MC stub missing home pointer primary".to_owned());
    }
    // Must remain a short residual — never the full premium multi-panel chrome.
    for banned in [
        "Workers (live)",
        "Progress Log",
        "Stream  #",
        "▮▮▮▯",
        "1-4 Jump",
        "Tab Cycle",
        "Preconditions",
        "Features 18/42",
        "Features 17/42",
    ] {
        if text.contains(banned) {
            return Err(format!("MC stub competes with home chrome: {banned}"));
        }
    }
    // Rainbow / invent residuals.
    if text.contains("Tasks monitor") || text.contains("Goals board") || text.contains("Owner ·") {
        return Err("MC stub residual: banned invent chrome".to_owned());
    }
    if text.contains("Claude") || text.contains("Cortex Code") {
        return Err("MC stub residual: Claude/Cortex wording".to_owned());
    }
    match kind {
        McStubKind::Concurrency => {
            if !text.contains("Parallel 3/4") {
                return Err("concurrency stub missing Parallel 3/4 cue".to_owned());
            }
            if !text.contains("absorbed") && !text.contains("Pool detail") {
                return Err("concurrency stub missing absorbed/pool language".to_owned());
            }
        }
        McStubKind::Launch => {
            if !text.contains("Assign") {
                return Err("launch stub missing Assign language".to_owned());
            }
            if !text.contains("Parallel 3/4") {
                return Err("launch stub missing Parallel 3/4".to_owned());
            }
        }
        McStubKind::LaunchBlocked => {
            if !(text.contains("blocked") || text.contains("Blocked") || text.contains("full")) {
                return Err("blocked stub missing blocked/full language".to_owned());
            }
        }
        McStubKind::TeamBoard => {
            if !(text.contains("assignment") || text.contains("Assignment")) {
                return Err("team-board stub missing assignment language".to_owned());
            }
            if !text.contains("Parallel 3/4") {
                return Err("team-board stub missing Parallel 3/4".to_owned());
            }
        }
    }
    Ok(canvas)
}

/// Shared Mission Control V2 premium shell under CortexNight.
fn mission_control_home(variant: McVariant) -> Result<Canvas, String> {
    let mut canvas = Canvas::new(Theme::CortexNight);
    paint_mc_top_bar_premium(&mut canvas)?;

    // (1) Full-width hatched/dense RUNNING bar + fraction chip + live/free/queued.
    let (frac, live_meta) = match variant {
        McVariant::RunningAll => ("18/42", "3 live · 1 free · 2 queued · Parallel 3/4"),
        McVariant::OneFailed => ("17/42", "2 live · 1 failed · 1 free · Parallel 3/4"),
    };
    canvas.write(2, 2, "● RUNNING", TextStyle::bold(MC_ORANGE))?;
    // Dense hatched strip: filled + light blocks across the mid band.
    let bar = "████████████████░░░░░░░░░░░░░░░░░░";
    canvas.write(2, 13, bar, TextStyle::plain(MC_ORANGE))?;
    let after_bar = 13 + bar.chars().count() as u16 + 2;
    let chip = format!("{frac}  ·  {live_meta}");
    canvas.write(2, after_bar, &chip, TextStyle::plain(ColorToken::Secondary))?;

    // (9) Section rule under RUNNING chrome.
    paint_mc_rule(&mut canvas, 3)?;

    // Column split: Workers left (~2-58), Features + Progress Log right (~60-117).
    let right = 60u16;
    let (workers_title, features_title) = match variant {
        McVariant::RunningAll => ("Workers (live) 3/4", "Features 18/42"),
        McVariant::OneFailed => ("Workers 3/4 · 1 failed", "Features 17/42"),
    };
    canvas.write(4, 2, workers_title, TextStyle::bold(ColorToken::Bright))?;
    canvas.write(4, right, features_title, TextStyle::bold(ColorToken::Bright))?;

    // (3) Accordion workers: selected expands Preconditions/Expected; others 2 lines + mini ticks.
    let cards: [WorkerCard; 3] = match variant {
        McVariant::RunningAll => [
            WorkerCard {
                id: "#21",
                selected: true,
                mark: "●",
                feature: "note-open-risks",
                timer: "4m12s",
                skill: "v3-flow-worker",
                ticks: "▰▰▰▰▰▱▱▱",
                stream: "Execute cargo test note -- --nocapture",
            },
            WorkerCard {
                id: "#22",
                selected: false,
                mark: "●",
                feature: "safe-publish-path",
                timer: "1m40s",
                skill: "v3-flow-worker",
                ticks: "▰▰▰▱▱▱▱▱",
                stream: "Read staging/checklist.md (42 lines)",
            },
            WorkerCard {
                id: "#23",
                selected: false,
                mark: "●",
                feature: "window-os-actions",
                timer: "55s",
                skill: "v3-flow-worker",
                ticks: "▰▰▱▱▱▱▱▱",
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
                ticks: "▰▰▰▰▰▱▱▱",
                stream: "Execute cargo test note -- --nocapture",
            },
            WorkerCard {
                id: "#22",
                selected: false,
                mark: "✗",
                feature: "safe-publish-path",
                timer: "1m40s",
                skill: "v3-flow-worker",
                ticks: "▰▰▰▱▱▱▱▱",
                stream: "Failed · staging checklist missing rollback",
            },
            WorkerCard {
                id: "#23",
                selected: false,
                mark: "●",
                feature: "window-os-actions",
                timer: "55s",
                skill: "v3-flow-worker",
                ticks: "▰▰▱▱▱▱▱▱",
                stream: "Execute npx playwright test window",
            },
        ],
    };
    let mut wr: u16 = 5;
    for card in &cards {
        wr = paint_worker_card_premium(&mut canvas, wr, card)?;
    }
    write_left(
        &mut canvas,
        wr,
        2,
        "slot 4 free · 2 queued · Assign ready",
        TextStyle::plain(ColorToken::Muted),
    )?;

    // (4) Features ✓/●/○ + right [#id] badges; selection on focused live feature.
    paint_feature_row(
        &mut canvas,
        5,
        right,
        FeatureRow {
            selected: false,
            mark: "✓",
            name: "publish-checklist",
            badge: None,
            mark_tok: ColorToken::Ok,
        },
    )?;
    paint_feature_row(
        &mut canvas,
        6,
        right,
        FeatureRow {
            selected: false,
            mark: "✓",
            name: "staging-rollback",
            badge: None,
            mark_tok: ColorToken::Ok,
        },
    )?;
    paint_feature_row(
        &mut canvas,
        7,
        right,
        FeatureRow {
            selected: true,
            mark: "●",
            name: "note-open-risks",
            badge: Some("[#21]"),
            mark_tok: MC_ORANGE,
        },
    )?;
    match variant {
        McVariant::RunningAll => {
            paint_feature_row(
                &mut canvas,
                8,
                right,
                FeatureRow {
                    selected: false,
                    mark: "●",
                    name: "safe-publish-path",
                    badge: Some("[#22]"),
                    mark_tok: MC_ORANGE,
                },
            )?;
        }
        McVariant::OneFailed => {
            paint_feature_row(
                &mut canvas,
                8,
                right,
                FeatureRow {
                    selected: false,
                    mark: "✗",
                    name: "safe-publish-path",
                    badge: Some("[#22]"),
                    mark_tok: ColorToken::Bad,
                },
            )?;
        }
    }
    paint_feature_row(
        &mut canvas,
        9,
        right,
        FeatureRow {
            selected: false,
            mark: "●",
            name: "window-os-actions",
            badge: Some("[#23]"),
            mark_tok: MC_ORANGE,
        },
    )?;
    paint_feature_row(
        &mut canvas,
        10,
        right,
        FeatureRow {
            selected: false,
            mark: "○",
            name: "github-actions-ci",
            badge: Some("queued"),
            mark_tok: ColorToken::Muted,
        },
    )?;
    paint_feature_row(
        &mut canvas,
        11,
        right,
        FeatureRow {
            selected: false,
            mark: "○",
            name: "e2e-test-suite",
            badge: Some("queued"),
            mark_tok: ColorToken::Muted,
        },
    )?;

    // (5) Progress Log disciplined (fixed timestamps, dim brackets).
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
        canvas.write(14 + i as u16, right, line, TextStyle::plain(*tok))?;
    }

    // Mid rule before stream.
    paint_mc_rule(&mut canvas, 23)?;

    // (6) Bottom Stream Execute/Read with ✓ results + spinner (6–10 lines).
    canvas.write(
        24,
        2,
        "Stream  #21  note-open-risks",
        TextStyle::bold(ColorToken::Bright),
    )?;
    canvas.write(24, 100, "Time 4m12s", TextStyle::plain(ColorToken::Muted))?;
    // Verb bold orange; meta dim; results Ok; spinner orange.
    canvas.write(25, 2, "Execute", TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        25,
        11,
        "cargo test note -- --nocapture",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(26, 4, "✓", TextStyle::plain(ColorToken::Ok))?;
    canvas.write(
        26,
        6,
        "14 tests passed. Fixture state verified.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(27, 2, "Read", TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        27,
        11,
        "src/screens/orchestration.rs  (214 lines)",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(28, 2, "Read", TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        28,
        11,
        "library/mission-control-v2-premium-fable5-note.md  (98 lines)",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(29, 2, "Execute", TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        29,
        11,
        "cargo clippy --all-targets --features png",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(30, 4, "✓", TextStyle::plain(ColorToken::Ok))?;
    canvas.write(
        30,
        6,
        "0 warnings. Palette restraint holds on focused stream.",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    canvas.write(31, 2, "Read", TextStyle::bold(MC_ORANGE))?;
    canvas.write(
        31,
        11,
        "library/mission-control-parallel-design.md  (64 lines)",
        TextStyle::plain(ColorToken::Secondary),
    )?;
    // Spinner for focused worker live step.
    canvas.write(32, 2, "⠋", TextStyle::plain(MC_ORANGE))?;
    canvas.write(
        32,
        4,
        "Running multi-worker fixture paint first…",
        TextStyle::plain(ColorToken::Text),
    )?;
    canvas.write(
        33,
        4,
        "skill v3-flow-worker · milestone v3-mission-control-v2-premium",
        TextStyle::plain(ColorToken::Dim),
    )?;

    // (10) Footer F W 1-4 Tab A I Esc
    paint_mc_footer_premium(&mut canvas)?;
    finish_mc_premium(canvas)
}

fn paint_mc_rule(canvas: &mut Canvas, row: u16) -> Result<(), String> {
    let rule = "─".repeat((CANVAS_COLUMNS - 4) as usize);
    canvas.write(row, 2, &rule, TextStyle::plain(ColorToken::Dim))?;
    Ok(())
}

/// Premium top bar: brand orange, metrics, slot cells ▮▮▮▯, context budget.
fn paint_mc_top_bar_premium(canvas: &mut Canvas) -> Result<(), String> {
    let brand = ".: Mission Control";
    canvas.write(0, 2, brand, TextStyle::bold(MC_ORANGE))?;
    let after_brand = 2 + brand.chars().count() as u16 + 2;
    canvas.write(
        0,
        after_brand,
        DEMO_CWD,
        TextStyle::plain(ColorToken::Muted),
    )?;
    // (7) Header metrics + slot cells; keep Parallel 3/4 + budget for existing gates.
    // Paint pieces so slot cells get orange without UTF-8 byte-offset bugs.
    let left = "Time 32m12s · In 3.8K · Out 1.3M · ";
    let right = format!(" · Parallel 3/4 · {DEMO_CTX_USED} / {DEMO_CTX_TOTAL}");
    let slots = "▮▮▮▯";
    let total = left.chars().count() + slots.chars().count() + right.chars().count();
    let col = CANVAS_COLUMNS.saturating_sub(total as u16 + 2);
    canvas.write(0, col, left, TextStyle::plain(ColorToken::Muted))?;
    let slot_col = col + left.chars().count() as u16;
    canvas.write(0, slot_col, "▮▮▮", TextStyle::plain(MC_ORANGE))?;
    canvas.write(0, slot_col + 3, "▯", TextStyle::plain(ColorToken::Dim))?;
    canvas.write(
        0,
        slot_col + 4,
        &right,
        TextStyle::plain(ColorToken::Muted),
    )?;
    Ok(())
}

/// (10) Premium home footer: F W 1-4 Tab A I Esc.
fn paint_mc_footer_premium(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        39,
        1,
        "F Features   W Workers   1-4 Jump   Tab Cycle   A Assign   I Intervene   Esc",
        TextStyle::plain(ColorToken::Dim),
    )?;
    // Hotkeys in orange so footer restraint still reads premium (single accent).
    // Columns match starts inside the footer string beginning at col 1.
    for (col, key) in [
        (1u16, "F"),
        (14, "W"),
        (26, "1-4"),
        (37, "Tab"),
        (49, "A"),
        (60, "I"),
        (74, "Esc"),
    ] {
        canvas.write(39, col, key, TextStyle::plain(MC_ORANGE))?;
    }
    Ok(())
}

struct WorkerCard {
    id: &'static str,
    selected: bool,
    mark: &'static str,
    feature: &'static str,
    timer: &'static str,
    skill: &'static str,
    ticks: &'static str,
    stream: &'static str,
}

struct FeatureRow {
    selected: bool,
    mark: &'static str,
    name: &'static str,
    badge: Option<&'static str>,
    mark_tok: ColorToken,
}

fn paint_feature_row(canvas: &mut Canvas, row: u16, col: u16, feat: FeatureRow) -> Result<(), String> {
    // (8) Selection invert bar for focused feature.
    if feat.selected {
        canvas.fill_row_bg(
            row,
            col,
            CANVAS_COLUMNS.saturating_sub(2),
            TextStyle::on(ColorToken::Bright, ColorToken::Surface),
        )?;
        canvas.write(row, col, "▌", TextStyle::plain(MC_ORANGE))?;
        canvas.write(
            row,
            col + 2,
            feat.mark,
            TextStyle::bold(feat.mark_tok),
        )?;
        canvas.write(
            row,
            col + 4,
            feat.name,
            TextStyle::bold(ColorToken::Bright),
        )?;
    } else {
        canvas.write(row, col + 2, feat.mark, TextStyle::plain(feat.mark_tok))?;
        canvas.write(
            row,
            col + 4,
            feat.name,
            TextStyle::plain(ColorToken::Text),
        )?;
    }
    if let Some(badge) = feat.badge {
        let badge_col = CANVAS_COLUMNS.saturating_sub(badge.chars().count() as u16 + 3);
        let badge_style = if feat.selected {
            TextStyle::plain(ColorToken::Secondary)
        } else {
            TextStyle::plain(ColorToken::Muted)
        };
        canvas.write(row, badge_col, badge, badge_style)?;
    }
    Ok(())
}

/// Left workers pane ends before Features column (right=60). Clamp to 2..57.
const MC_LEFT_MAX: u16 = 57;

fn write_left(canvas: &mut Canvas, row: u16, col: u16, text: &str, style: TextStyle) -> Result<(), String> {
    let max_len = MC_LEFT_MAX.saturating_sub(col) as usize;
    if max_len == 0 {
        return Ok(());
    }
    let clipped: String = text.chars().take(max_len).collect();
    if !clipped.is_empty() {
        canvas.write(row, col, &clipped, style)?;
    }
    Ok(())
}

/// Accordion worker paint. Selected expands preconditions/expected; others collapse.
/// Returns the next free row after this card.
fn paint_worker_card_premium(
    canvas: &mut Canvas,
    row: u16,
    card: &WorkerCard,
) -> Result<u16, String> {
    if card.selected {
        // (8) Selection invert bar across worker head (left pane only).
        canvas.fill_row_bg(
            row,
            2,
            MC_LEFT_MAX,
            TextStyle::on(ColorToken::Bright, ColorToken::Surface),
        )?;
        write_left(canvas, row, 2, "▌", TextStyle::plain(MC_ORANGE))?;
        let head = format!("{} {} {}", card.id, card.mark, card.feature);
        write_left(canvas, row, 4, &head, TextStyle::bold(ColorToken::Bright))?;
        write_left(canvas, row, 34, card.ticks, TextStyle::plain(MC_ORANGE))?;
        write_left(canvas, row, 48, card.timer, TextStyle::plain(ColorToken::Muted))?;

        write_left(
            canvas,
            row + 1,
            4,
            &format!("skill {} · milestone MCx", card.skill),
            TextStyle::plain(ColorToken::Secondary),
        )?;
        write_left(
            canvas,
            row + 2,
            4,
            "Preconditions",
            TextStyle::bold(ColorToken::Bright),
        )?;
        write_left(
            canvas,
            row + 3,
            6,
            "fable5 note present · harness-v4 builds",
            TextStyle::plain(ColorToken::Muted),
        )?;
        write_left(
            canvas,
            row + 4,
            6,
            "Factory MC ref available · pool 3/4",
            TextStyle::plain(ColorToken::Muted),
        )?;
        write_left(
            canvas,
            row + 5,
            4,
            "Expected",
            TextStyle::bold(ColorToken::Bright),
        )?;
        write_left(
            canvas,
            row + 6,
            6,
            "Factory hierarchy · Parallel 3/4 obvious",
            TextStyle::plain(ColorToken::Text),
        )?;
        write_left(
            canvas,
            row + 7,
            6,
            "Orange+gray · Execute/Read stream results",
            TextStyle::plain(ColorToken::Text),
        )?;
        Ok(row + 9)
    } else {
        let mark_style = if card.mark == "✗" {
            TextStyle::plain(ColorToken::Bad)
        } else {
            TextStyle::plain(ColorToken::Text)
        };
        let head = format!("  {} {} {}", card.id, card.mark, card.feature);
        write_left(canvas, row, 2, &head, mark_style)?;
        // Re-color the mark when running (orange ●) or failed (already Bad on whole head).
        if card.mark == "●" {
            // place ● after "  #NN " (2 spaces + id + space)
            let mark_col = 2 + 2 + card.id.chars().count() as u16 + 1;
            write_left(canvas, row, mark_col, "●", TextStyle::plain(MC_ORANGE))?;
        }
        write_left(canvas, row, 34, card.ticks, TextStyle::plain(ColorToken::Muted))?;
        write_left(canvas, row, 48, card.timer, TextStyle::plain(ColorToken::Muted))?;
        write_left(canvas, row + 1, 6, card.stream, TextStyle::plain(ColorToken::Dim))?;
        Ok(row + 3)
    }
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

/// Premium home finish: pack residuals + palette restraint cues on the paint tokens used.
fn finish_mc_premium(canvas: Canvas) -> Result<Canvas, String> {
    let canvas = finish_mc(canvas)?;
    let text = canvas.to_text();
    if !text.contains("▮▮▮▯") {
        return Err("MC premium missing slot cells ▮▮▮▯".to_owned());
    }
    if !text.contains("Workers (live) 3/4") && !text.contains("Workers 3/4") {
        return Err("MC premium missing Workers live 3/4".to_owned());
    }
    if !text.contains("[#21]") {
        return Err("MC premium missing feature [#id] badges".to_owned());
    }
    if !text.contains("Execute") || !text.contains("Read") {
        return Err("MC premium missing Execute/Read stream grammar".to_owned());
    }
    if !text.contains('✓') {
        return Err("MC premium missing stream result ✓".to_owned());
    }
    if !text.contains('⠋') {
        return Err("MC premium missing spinner cue".to_owned());
    }
    if !text.contains("1-4 Jump") || !text.contains("Tab Cycle") {
        return Err("MC premium missing F W 1-4 Tab footer".to_owned());
    }
    // Rainbow panel language (banned multi-hue panel labels).
    for banned in ["purple", "cyan", "magenta", "violet", "rainbow"] {
        if text.to_lowercase().contains(banned) {
            return Err(format!("MC premium residual rainbow cue: {banned}"));
        }
    }
    // Token usage restraint: home paint must not emit Info/Model/Plan/Accent/Running
    // cell tokens (cyan/magenta multi-hue). Inspect canvas cells directly.
    for cell in &canvas.cells {
        match cell.style.foreground {
            ColorToken::Accent | ColorToken::Info | ColorToken::Running | ColorToken::Model
            | ColorToken::Plan => {
                return Err(format!(
                    "MC premium rainbow token {:?} on glyph {:?}",
                    cell.style.foreground, cell.glyph
                ));
            }
            _ => {}
        }
        if let Some(bg) = cell.style.background {
            match bg {
                ColorToken::Accent
                | ColorToken::Info
                | ColorToken::Running
                | ColorToken::Model
                | ColorToken::Plan
                | ColorToken::Ok
                | ColorToken::Bad
                | ColorToken::Warn
                | ColorToken::Path => {
                    return Err(format!("MC premium rainbow/hot bg token {bg:?}"));
                }
                _ => {}
            }
        }
    }
    Ok(canvas)
}

/// VAL-V3-MC-005 residual: assignment absorbed on home — short pointer, not a board.
fn team_board() -> Result<Canvas, String> {
    mc_home_pointer_stub(McStubKind::TeamBoard)
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

/// VAL-V3-MC-005 residual: pool metrics absorbed into home header slots / Workers line.
fn concurrency_metrics() -> Result<Canvas, String> {
    mc_home_pointer_stub(McStubKind::Concurrency)
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

    /// Primary full MC product surfaces only (VAL-V3-MC-004/006).
    const MC_PRIMARY: &[&str] = &["workflow-monitor", "workflow-monitor-failure"];
    /// Former multi-page MC destinations collapsed to stubs (VAL-V3-MC-005).
    const MC_SECONDARY_STUBS: &[&str] = &[
        "concurrency-metrics",
        "workflow-launch",
        "workflow-launch-blocked",
        "team-board",
    ];

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
            let is_primary = MC_PRIMARY.contains(&id);
            let is_stub = MC_SECONDARY_STUBS.contains(&id);
            if is_primary {
                assert!(
                    text.contains(".: Mission Control"),
                    "{id} missing MC brand"
                );
                assert!(
                    text.contains("Parallel 3/4") || text.contains("Parallel 4/4"),
                    "{id} missing pool metric"
                );
                assert!(
                    text.contains("Workers (live)") || text.contains("Workers 3/4"),
                    "{id} missing Workers pane"
                );
            } else if is_stub {
                assert!(
                    text.contains(".: Mission Control"),
                    "{id} stub missing MC brand"
                );
                assert!(
                    text.contains("Open Mission Control home"),
                    "{id} stub missing home pointer"
                );
                // Stubs must not compete as full multi-panel homes.
                assert!(
                    !text.contains("Workers (live)"),
                    "{id} stub must not paint Workers live pane"
                );
                assert!(
                    !text.contains("Progress Log"),
                    "{id} stub must not paint Progress Log"
                );
                assert!(!text.contains("Stream  #"), "{id} stub must not paint Stream");
            } else {
                assert!(
                    text.contains("Cortex")
                        || text.contains("Tasks")
                        || text.contains("Dashboard")
                        || text.contains("plan")
                        || text.contains("Shift+Tab")
                        || text.contains("Subagents")
                        || text.contains("Watchers")
                        || text.contains("$ cargo")
                        || text.contains("Advanced"),
                    "{id} lacks pure Cortex residual language"
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

    /// VAL-V3-MC-001 + MC-004: multi-worker Mission Control home anatomy.
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
        let live_marks = mon.matches("●").count();
        assert!(
            live_marks >= 2,
            "expected multi live ● markers, got {live_marks}"
        );
        assert!(mon.contains("Features"));
        assert!(mon.contains("Progress Log"));
        assert!(mon.contains("Worker #21") || mon.contains("Worker #22"));
        assert!(mon.contains("skill"));
        assert!(mon.contains("milestone"));
        assert!(mon.contains("Preconditions"));
        assert!(mon.contains("Expected"));
        assert!(mon.contains("Execute"));
        assert!(mon.contains("Read"));
        assert!(mon.contains("F Features") || mon.contains("F "));
        assert!(mon.contains("W Workers") || mon.contains("W "));
        assert!(mon.contains("Esc"));
        assert!(!mon.contains("Tasks monitor"));
        assert!(!mon.contains("Subagents"));
    }

    /// VAL-V3-MC-004: premium Factory hierarchy cues.
    #[test]
    fn mc_premium_home_factory_hierarchy_and_stream_grammar() {
        let c = render("workflow-monitor").unwrap_or_else(|e| panic!("{e}"));
        let mon = c.to_text();
        // (1) full-width RUNNING bar + fraction chip
        assert!(mon.contains("● RUNNING"));
        assert!(mon.contains("████████████████"));
        assert!(mon.contains("18/42"));
        assert!(mon.contains("3 live") && mon.contains("1 free") && mon.contains("2 queued"));
        // (7) slot cells
        assert!(mon.contains("▮▮▮▯"));
        // (3) accordion: selected shows Preconditions/Expected; collapsed stream lines on siblings
        assert!(mon.contains("Preconditions"));
        assert!(mon.contains("Expected"));
        assert!(mon.contains("Read staging/checklist.md") || mon.contains("Execute npx playwright"));
        assert!(mon.contains("▰▰"));
        // (4) feature badges
        assert!(mon.contains("[#21]") && mon.contains("[#22]") && mon.contains("[#23]"));
        assert!(mon.contains('✓') && mon.contains('○'));
        // (6) stream grammar
        assert!(mon.contains("Stream  #21"));
        assert!(mon.contains("Execute"));
        assert!(mon.contains("Read"));
        assert!(mon.contains("14 tests passed") || mon.contains('✓'));
        assert!(mon.contains('⠋'));
        // (10) footer
        assert!(mon.contains("1-4 Jump"));
        assert!(mon.contains("Tab Cycle"));
        assert!(mon.contains("A Assign"));
        assert!(mon.contains("I Intervene"));
        // (8) selection bar glyph
        assert!(mon.contains('▌'));
        // multi-worker still obvious
        assert!(mon.contains("#21") && mon.contains("#22") && mon.contains("#23"));
    }

    /// VAL-V3-MC-004: no rainbow multi-accent panel tokens on premium home.
    #[test]
    fn mc_premium_home_palette_restraint_no_rainbow_tokens() {
        let c = render("workflow-monitor").unwrap_or_else(|e| panic!("{e}"));
        let mon = c.to_text();
        for banned in ["purple", "cyan", "magenta", "violet", "rainbow"] {
            assert!(
                !mon.to_lowercase().contains(banned),
                "banned palette word {banned}"
            );
        }
        let mut saw_path = false;
        let mut saw_ok = false;
        for cell in &c.cells {
            match cell.style.foreground {
                ColorToken::Accent | ColorToken::Info | ColorToken::Running | ColorToken::Model
                | ColorToken::Plan => {
                    panic!(
                        "rainbow token {:?} on glyph {:?}",
                        cell.style.foreground, cell.glyph
                    );
                }
                ColorToken::Path => saw_path = true,
                ColorToken::Ok => saw_ok = true,
                _ => {}
            }
        }
        assert!(saw_path, "orange Path accent must be used");
        assert!(saw_ok, "Ok terminal for ✓ results");
        // Orange accent count should dominate; Path cells are the single hot accent family.
        let path_n = c
            .cells
            .iter()
            .filter(|c| c.style.foreground == ColorToken::Path)
            .count();
        assert!(path_n >= 8, "expected multiple Path/orange cues, got {path_n}");
    }

    /// VAL-V3-MC-004: multi-worker + stream grammar focused cases.
    #[test]
    fn mc_premium_home_multi_worker_and_stream_cues() {
        let mon = render("workflow-monitor")
            .unwrap_or_else(|e| panic!("{e}"))
            .to_text();
        // ≥3 live worker ids
        for id in ["#21", "#22", "#23"] {
            assert!(mon.contains(id), "missing worker {id}");
        }
        assert!(mon.contains("Workers (live) 3/4"));
        // Stream block
        let stream_at = mon.find("Stream  #21").expect("stream header");
        let stream = &mon[stream_at..];
        assert!(stream.contains("Execute"));
        assert!(stream.contains("Read"));
        assert!(stream.contains('✓'));
        assert!(stream.contains('⠋') || stream.contains("Running multi-worker"));
        // Collapsed siblings keep mini activity without full Preconditions blocks for each
        let pre_count = mon.matches("Preconditions").count();
        assert_eq!(
            pre_count, 1,
            "accordion: only selected worker expands Preconditions"
        );
    }

    /// VAL-V3-MC-003 + MC-006: one worker fails without killing the pool run.
    #[test]
    fn mc_failure_keeps_siblings_running() {
        let fail = render("workflow-monitor-failure")
            .unwrap_or_else(|e| panic!("{e}"))
            .to_text();
        assert!(fail.contains(".: Mission Control"));
        assert!(fail.contains("Parallel 3/4"));
        assert!(
            fail.contains("● RUNNING"),
            "run must stay RUNNING after per-worker fail"
        );
        assert!(fail.contains("✗"));
        assert!(fail.contains("#22"));
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
        let worker_ids = ["#21", "#22", "#23"]
            .iter()
            .filter(|id| fail.contains(*id))
            .count();
        assert!(worker_ids >= 2, "need multi-worker presence after fail");
    }

    /// VAL-V3-MC-006: failure variant keeps premium home DNA (Factory hierarchy).
    #[test]
    fn mc_failure_keeps_premium_home_dna() {
        let c = render("workflow-monitor-failure").unwrap_or_else(|e| panic!("{e}"));
        let fail = c.to_text();
        // Same premium shell as home.
        assert!(fail.contains(".: Mission Control"));
        assert!(fail.contains("▮▮▮▯"), "slot cells");
        assert!(fail.contains("● RUNNING"));
        assert!(fail.contains("████████████████"), "dense RUNNING bar");
        assert!(fail.contains("17/42") || fail.contains("Features 17/42"));
        assert!(fail.contains("Workers 3/4") || fail.contains("Workers (live)"));
        assert!(fail.contains("1 failed"));
        // Per-worker fail + siblings running.
        assert!(fail.contains('✗'));
        assert!(fail.contains("#21") && fail.contains("#22") && fail.contains("#23"));
        assert!(fail.contains("[#21]") && fail.contains("[#22]") && fail.contains("[#23]"));
        // Stream grammar retained for focused running sibling.
        assert!(fail.contains("Stream  #21"));
        assert!(fail.contains("Execute") && fail.contains("Read"));
        assert!(fail.contains('✓') && fail.contains('⠋'));
        assert!(fail.contains("1-4 Jump") && fail.contains("Tab Cycle"));
        assert!(fail.contains("Preconditions") && fail.contains("Expected"));
        // Log records the fail while pool stays RUNNING.
        assert!(fail.contains("Worker #22 failed") || fail.contains("failed [safe-publish-path]"));
        // Palette restraint (no rainbow tokens).
        for cell in &c.cells {
            match cell.style.foreground {
                ColorToken::Accent | ColorToken::Info | ColorToken::Running | ColorToken::Model
                | ColorToken::Plan => {
                    panic!(
                        "failure premium rainbow token {:?} on {:?}",
                        cell.style.foreground, cell.glyph
                    );
                }
                _ => {}
            }
        }
    }

    /// VAL-V3-MC-005: secondary orch pages are non-primary stubs / home pointers.
    #[test]
    fn mc_secondary_stubs_are_home_pointers_not_full_dashboards() {
        for id in MC_SECONDARY_STUBS {
            let t = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(
                t.contains(".: Mission Control"),
                "{id} keeps brand for residual pointer"
            );
            assert!(
                t.contains("Open Mission Control home"),
                "{id} must point home"
            );
            assert!(
                t.contains("residual") || t.contains("absorbed") || t.contains("not a"),
                "{id} must read as residual/absorbed, not primary product"
            );
            // Full multi-panel MC home chrome must NOT appear on stubs.
            for banned in [
                "Workers (live)",
                "Progress Log",
                "Stream  #",
                "▮▮▮▯",
                "1-4 Jump",
                "Tab Cycle",
                "Preconditions",
                "Features 18/42",
                "Features 17/42",
            ] {
                assert!(
                    !t.contains(banned),
                    "{id} must not compete with home chrome {banned:?}"
                );
            }
            assert!(!t.contains("Goals board"), "{id}");
            assert!(!t.contains("Owner ·"), "{id}");
            assert!(!t.contains("Tasks monitor"), "{id}");
            assert_eq!(t.lines().count(), 40, "{id}");
            assert!(
                t.contains("9.5K / 500K") || t.contains("K / "),
                "{id} budget chrome"
            );
        }

        // Distinct residual copy per stub.
        let pool = render("concurrency-metrics").unwrap().to_text();
        assert!(pool.contains("Parallel 3/4"));
        assert!(
            pool.contains("Pool detail") || pool.contains("absorbed"),
            "pool residual language"
        );

        let launch = render("workflow-launch").unwrap().to_text();
        assert!(launch.contains("Assign") && launch.contains("Parallel 3/4"));
        assert!(launch.contains("A Assign") || launch.contains("free slot"));

        let blocked = render("workflow-launch-blocked").unwrap().to_text();
        assert!(
            blocked.contains("blocked")
                || blocked.contains("Blocked")
                || blocked.contains("full")
                || blocked.contains("Pool full"),
            "blocked residual"
        );
        assert_ne!(launch, blocked, "launch and blocked stubs must differ");

        let board = render("team-board").unwrap().to_text();
        assert!(
            board.contains("assignment") || board.contains("Assignment"),
            "assignment residual"
        );
        assert!(!board.contains("kanban"));
    }

    /// VAL-V3-MC-005: only primary home (+ failure) paints full multi-panel chrome.
    #[test]
    fn mc_main_page_is_only_full_product() {
        let mon = render("workflow-monitor").unwrap().to_text();
        assert!(mon.contains("Workers (live) 3/4"));
        assert!(mon.contains("Progress Log"));
        assert!(mon.contains("Stream  #21"));
        assert!(mon.contains("▮▮▮▯"));

        let fail = render("workflow-monitor-failure").unwrap().to_text();
        assert!(fail.contains("Workers 3/4") || fail.contains("Workers (live)"));
        assert!(fail.contains("Progress Log"));
        assert!(fail.contains("Stream  #21"));
        assert!(fail.contains("▮▮▮▯"));

        // Every other orch ID must lack the full triple chrome competition.
        for id in [
            "concurrency-metrics",
            "workflow-launch",
            "workflow-launch-blocked",
            "team-board",
            "mode-switch",
            "mode-switch-refused",
            "reviewer-output",
            "reviewer-approved",
            "verification-ladder",
            "intent-scorer-pass",
            "intent-scorer-fail",
            "tool-envelope",
        ] {
            let t = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            let has_workers = t.contains("Workers (live)") || t.contains("Workers 3/4 ·");
            let has_log = t.contains("Progress Log");
            let has_stream = t.contains("Stream  #");
            let competing = has_workers && has_log && has_stream;
            assert!(
                !competing,
                "{id} paints full multi-panel MC home chrome competing with workflow-monitor"
            );
        }
    }

    /// Primary home + stubs brand consistency (VAL-V3-MC-004/005).
    #[test]
    fn mc_pack_shell_consistency() {
        for id in MC_PRIMARY {
            let t = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(t.contains(".: Mission Control"), "{id} missing MC brand");
            assert!(
                t.contains("Parallel 3/4") || t.contains("Parallel 4/4"),
                "{id} missing Parallel N/max"
            );
            assert!(
                t.contains("9.5K / 500K") || t.contains("K / "),
                "{id} missing CortexNight context budget"
            );
            assert!(t.contains("▮▮▮▯"), "{id} premium slots");
            assert!(!t.contains("Cortex Code"), "{id}");
            assert!(!t.contains("Code needs"), "{id}");
            assert!(!t.contains("Tasks monitor"), "{id}");
            assert!(!t.contains("Goals board"), "{id}");
            assert_eq!(t.lines().count(), 40, "{id}");
        }
        for id in MC_SECONDARY_STUBS {
            let t = render(id)
                .unwrap_or_else(|e| panic!("{id}: {e}"))
                .to_text();
            assert!(t.contains(".: Mission Control"), "{id}");
            assert!(t.contains("Open Mission Control home"), "{id}");
            assert!(!t.contains("Workers (live)"), "{id}");
            assert!(!t.contains("Tasks monitor"), "{id}");
            assert!(!t.contains("Goals board"), "{id}");
            assert_eq!(t.lines().count(), 40, "{id}");
        }
        let launch = render("workflow-launch").unwrap().to_text();
        let blocked = render("workflow-launch-blocked").unwrap().to_text();
        assert_ne!(launch, blocked, "launch and blocked must differ");
    }

    #[test]
    fn tasks_pane_and_dashboard_language_visible() {
        // Primary MC home owns multi-panel chrome; secondary stubs point home.
        let mon = render("workflow-monitor").unwrap().to_text();
        assert!(mon.contains(".: Mission Control"));
        assert!(mon.contains("Workers (live)"));
        assert!(!mon.contains("Tasks monitor"));
        assert!(!mon.contains("i intervene"));

        let board = render("team-board").unwrap().to_text();
        assert!(board.contains(".: Mission Control"));
        assert!(board.contains("assignment") || board.contains("Assignment"));
        assert!(board.contains("Open Mission Control home"));
        assert!(!board.contains("Goals board"));
        assert!(!board.contains("Owner ·"));
        assert!(!board.contains("Workers (live)"));

        let env = render("tool-envelope").unwrap().to_text();
        assert!(env.contains("$ cargo test publish"));
        assert!(!env.contains("Shell ·"));
        assert!(!env.contains("Shell  "));
    }
}
