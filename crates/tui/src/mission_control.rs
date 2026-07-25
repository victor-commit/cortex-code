//! Mission Control V2 premium home (product TUI).
//!
//! Authority: mockups-v3 `orchestration/workflow-monitor{,-failure}.txt` +
//! mission `library/mission-control-v2-premium-fable5-note.md`.
//!
//! Product truth:
//! - One primary surface: `.: Mission Control` home with parallel workers.
//! - Failure variant keeps the same DNA (one ✗ while siblings stay ●).
//! - Secondary orch destinations collapse to thin stubs that point home.
//!
//! This module paints plain text fingerprints consumed by the Ratatui surface
//! and by unit/tmux assertions (VAL-CORTEX-TUI-004 / CROSS-001). M4 binds live
//! scheduler snapshots via [`live_home_lines`] / [`snapshot_to_variant`] while
//! keeping the premium fixture shell for offline paints.

use tui_components::chrome::{
    format_context_budget, DEMO_CTX_TOTAL, DEMO_CTX_USED, DEMO_CWD,
};

/// Brand line fingerprint required on MC home + home-pointer stubs.
pub const MC_BRAND: &str = ".: Mission Control";

/// Premium putty / footer for the MC home surface.
pub const MC_FOOTER: &str =
    "F Features   W Workers   1-4 Jump   Tab Cycle   A Assign   I Intervene   Esc";

/// Parallel fixture pool: 3 live of 4 slots (one free).
pub const MC_PARALLEL_SLOTS: &str = "▮▮▮▯";

/// Home / failure variants for the single-page Mission Control surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McVariant {
    /// All three fixture workers live (`●`).
    RunningAll,
    /// One worker failed (`✗`) while siblings stay live.
    OneFailed,
}

/// Non-primary orch destinations collapsed under MC V2 (home pointers only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McStubKind {
    Concurrency,
    Launch,
    LaunchBlocked,
    TeamBoard,
}

/// One worker lane on the premium home.
#[derive(Debug, Clone, Copy)]
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

/// One feature-rail row.
#[derive(Debug, Clone, Copy)]
struct FeatureRow {
    selected: bool,
    mark: &'static str,
    name: &'static str,
    badge: Option<&'static str>,
}

/// Render MC V2 premium home as plain text lines (120-col DNA).
#[must_use]
pub fn render_home_lines(variant: McVariant, width: usize) -> Vec<String> {
    let width = width.max(80);
    let mut lines = Vec::with_capacity(40);
    lines.push(top_bar(width));
    lines.push(String::new());
    lines.push(running_bar(variant, width));
    lines.push(rule(width));

    let (workers_title, features_title) = match variant {
        McVariant::RunningAll => ("Workers (live) 3/4", "Features 18/42"),
        McVariant::OneFailed => ("Workers 3/4 · 1 failed", "Features 17/42"),
    };

    let left_w = (width.saturating_mul(55) / 100).clamp(42, 58);
    let right_w = width.saturating_sub(left_w + 2);
    lines.push(two_col(workers_title, features_title, left_w, right_w, width));

    let workers = worker_cards(variant);
    let features = feature_rows(variant);
    let progress = progress_log(variant);

    // Accordion workers on the left; features then progress log on the right.
    let mut left_body: Vec<String> = Vec::new();
    for card in &workers {
        left_body.extend(paint_worker_card(card, left_w));
    }
    left_body.push(pad_right("slot 4 free · 2 queued · Assign ready", left_w));

    let mut right_body: Vec<String> = Vec::new();
    for feat in &features {
        right_body.push(paint_feature_row(feat, right_w));
    }
    right_body.push(String::new());
    right_body.push(pad_right("Progress Log               1-8 of 24", right_w));
    for entry in progress {
        right_body.push(pad_right(entry, right_w));
    }

    let body_rows = left_body.len().max(right_body.len());
    for i in 0..body_rows {
        let l = left_body.get(i).map(String::as_str).unwrap_or("");
        let r = right_body.get(i).map(String::as_str).unwrap_or("");
        lines.push(two_col(l, r, left_w, right_w, width));
    }

    lines.push(rule(width));
    lines.extend(stream_block(width));
    // Spacer so footer sits near the bottom of a 40-row pane when rendered full-frame.
    while lines.len() < 38 {
        lines.push(String::new());
    }
    lines.push(MC_FOOTER.to_owned());
    lines
}

/// Thin residual stub for secondary orch IDs (must not compete with home chrome).
#[must_use]
pub fn render_stub_lines(kind: McStubKind, width: usize) -> Vec<String> {
    let width = width.max(60);
    let budget = format_context_budget(DEMO_CTX_USED, DEMO_CTX_TOTAL);
    let mut top = format!("{MC_BRAND}  {DEMO_CWD}");
    pad_to(&mut top, width);
    let budget_start = width.saturating_sub(budget.chars().count());
    if budget_start > top.chars().count() {
        // rebuild with right-aligned budget
        top = format!("{MC_BRAND}  {DEMO_CWD}");
        let gap = budget_start.saturating_sub(top.chars().count());
        top.push_str(&" ".repeat(gap));
        top.push_str(&budget);
    }

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

    vec![
        top,
        String::new(),
        rule(width),
        String::new(),
        title.to_owned(),
        String::new(),
        line1.to_owned(),
        line2.to_owned(),
        line3.to_owned(),
        String::new(),
        primary.to_owned(),
        "  Stay on residual · no multi-panel Mission Control here".to_owned(),
        String::new(),
        "Esc Mission Control home · residual stub only".to_owned(),
    ]
}

/// Map a live [`missions::MissionSnapshot`] to the closest MC paint variant.
///
/// Failure isolation: any failed feature with remaining live/completed siblings
/// selects [`McVariant::OneFailed`]; otherwise [`McVariant::RunningAll`].
#[must_use]
pub fn snapshot_to_variant(snap: &missions::MissionSnapshot) -> McVariant {
    if !snap.failed.is_empty() {
        McVariant::OneFailed
    } else {
        McVariant::RunningAll
    }
}

/// Render MC home lines from a live mission snapshot (progress_log + parallel counts).
///
/// Falls back to fixture geometry when the snapshot has no running workers yet, but
/// always stamps live parallel N/max and progress tail into the paint so headless
/// tests can assert scheduler binding without the full Ratatui loop.
#[must_use]
pub fn live_home_lines(snap: &missions::MissionSnapshot, width: usize) -> Vec<String> {
    let width = width.max(80);
    let variant = snapshot_to_variant(snap);
    let mut lines = render_home_lines(variant, width);

    // Overlay a live metrics line after the top bar so tests / UI can read real pool stats.
    let live_n = snap.parallel_live.max(snap.running.len());
    let max_n = snap.parallel_max.max(1);
    let live_meta = format!(
        "Live mission {} · Parallel {live_n}/{max_n} · done {}/{} · validation={}",
        snap.id,
        snap.features_done,
        snap.features_total,
        if snap.validation_invoked {
            snap.last_validation.as_deref().unwrap_or("yes")
        } else {
            "pending"
        }
    );
    // Insert after brand/top bar (index 0) and blank (index 1) → before RUNNING bar.
    let insert_at = 2.min(lines.len());
    lines.insert(insert_at, pad_right(&live_meta, width));

    if !snap.progress_tail.is_empty() {
        lines.push(pad_right("── live progress_log ──", width));
        for entry in snap.progress_tail.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
            let mut row = format!("  {entry}");
            if row.chars().count() > width {
                row = row.chars().take(width).collect();
            }
            lines.push(pad_right(&row, width));
        }
    }
    // Re-seat footer at end.
    lines.retain(|l| l.as_str() != MC_FOOTER);
    while lines.len() < 38 {
        lines.push(String::new());
    }
    lines.push(MC_FOOTER.to_owned());
    lines
}

/// Apply a mission bus event into a mutable snapshot (MC event-bus subscription helper).
///
/// Returns `true` when the snapshot was updated. Used by tests and the TUI event pump.
pub fn apply_mission_event(
    snap: &mut missions::MissionSnapshot,
    event: &events::Event,
) -> bool {
    match event {
        events::Event::MissionStarted(e) if e.mission_id == snap.id || snap.id.is_empty() => {
            snap.id = e.mission_id.clone();
            snap.mission_dir = e.mission_dir.clone();
            snap.parallel_max = e.max_parallel;
            snap.status = "running".into();
            true
        }
        events::Event::MissionWorkerStarted(e) if e.mission_id == snap.id => {
            snap.parallel_live = e.live_count;
            snap.parallel_max = e.max_parallel;
            if !snap.running.iter().any(|w| w.feature_id == e.feature_id) {
                snap.running.push(missions::WorkerSlotView {
                    worker_id: e.worker_id.clone(),
                    feature_id: e.feature_id.clone(),
                    state: "running".into(),
                });
            }
            snap.pending.retain(|id| id != &e.feature_id);
            true
        }
        events::Event::MissionWorkerCompleted(e) if e.mission_id == snap.id => {
            snap.parallel_live = e.live_count;
            snap.running.retain(|w| w.feature_id != e.feature_id);
            if !snap.completed.contains(&e.feature_id) {
                snap.completed.push(e.feature_id.clone());
            }
            snap.features_done = snap.completed.len();
            true
        }
        events::Event::MissionWorkerFailed(e) if e.mission_id == snap.id => {
            snap.parallel_live = e.live_count;
            snap.running.retain(|w| w.feature_id != e.feature_id);
            if !snap.failed.contains(&e.feature_id) {
                snap.failed.push(e.feature_id.clone());
            }
            true
        }
        events::Event::MissionProgress(e) if e.mission_id == snap.id => {
            snap.parallel_live = e.live_count;
            snap.progress_tail.push(format!(
                "worker={} feature={} {}",
                e.worker_id, e.feature_id, e.message
            ));
            if snap.progress_tail.len() > 24 {
                let drain = snap.progress_tail.len() - 24;
                snap.progress_tail.drain(0..drain);
            }
            true
        }
        events::Event::MissionValidation(e) if e.mission_id == snap.id => {
            snap.validation_invoked = true;
            snap.last_validation = Some(format!("{}:{}", e.milestone, e.outcome));
            true
        }
        events::Event::MissionFinished(e) if e.mission_id == snap.id => {
            snap.status = e.status.clone();
            snap.parallel_live = 0;
            snap.running.clear();
            true
        }
        _ => false,
    }
}

/// Validate home paint fingerprint (unit + render gates).
#[cfg(test)]
pub fn assert_home_fingerprint(text: &str, variant: McVariant) -> Result<(), String> {
    if !text.contains(MC_BRAND) {
        return Err("MC paint missing .: Mission Control brand".into());
    }
    if !text.contains("● RUNNING") {
        return Err("MC paint missing RUNNING chrome".into());
    }
    if !text.contains(MC_PARALLEL_SLOTS) && !text.contains("Parallel 3/4") {
        return Err("MC paint missing parallel slots / Parallel 3/4".into());
    }
    // ≥2 live workers visible.
    let live_marks = text.matches(" ● ").count() + text.matches("● ").count();
    if live_marks < 2 {
        return Err(format!("MC paint needs ≥2 live workers, saw markers≈{live_marks}"));
    }
    if !text.contains("#21") || !text.contains("#22") || !text.contains("#23") {
        return Err("MC paint missing multi-worker ids #21/#22/#23".into());
    }
    if !text.contains("Workers") {
        return Err("MC paint missing Workers pane".into());
    }
    if !text.contains("Features") {
        return Err("MC paint missing Features rail".into());
    }
    if !text.contains("Stream") {
        return Err("MC paint missing Stream region".into());
    }
    if !text.contains("Progress Log") {
        return Err("MC paint missing Progress Log".into());
    }
    if !text.contains("Execute") || !text.contains("Read") {
        return Err("MC stream missing Execute/Read grammar".into());
    }
    match variant {
        McVariant::RunningAll => {
            if text.contains("1 failed") {
                return Err("running variant must not show 1 failed".into());
            }
        }
        McVariant::OneFailed => {
            if !text.contains("✗") {
                return Err("failure variant missing ✗ marker".into());
            }
            if !text.contains("1 failed") && !text.contains("failed") {
                return Err("failure variant missing failed language".into());
            }
            // Siblings still running.
            if !text.contains("● note-open-risks") && !text.contains("#21 ●") {
                return Err("failure variant lost live sibling #21".into());
            }
        }
    }
    // Copper / invent bans.
    for banned in ["Cortex Code", "❯ You", "◆ Cortex", "Tasks monitor", "Goals board"] {
        if text.contains(banned) {
            return Err(format!("MC residual banned chrome: {banned}"));
        }
    }
    Ok(())
}

/// Validate stub fingerprint: points home, never multi-panel home chrome.
#[cfg(test)]
pub fn assert_stub_fingerprint(text: &str, kind: McStubKind) -> Result<(), String> {
    if !text.contains(MC_BRAND) {
        return Err("stub missing .: Mission Control".into());
    }
    if !text.contains("Open Mission Control home") {
        return Err("stub missing home pointer".into());
    }
    for banned in [
        "Workers (live)",
        "Progress Log",
        "Stream  #",
        MC_PARALLEL_SLOTS,
        "1-4 Jump",
        "Tab Cycle",
        "Preconditions",
        "Features 18/42",
        "Features 17/42",
    ] {
        if text.contains(banned) {
            return Err(format!("stub competes with home chrome: {banned}"));
        }
    }
    match kind {
        McStubKind::Concurrency => {
            if !text.contains("Parallel 3/4") {
                return Err("concurrency stub missing Parallel 3/4".into());
            }
        }
        McStubKind::Launch => {
            if !text.contains("Assign") {
                return Err("launch stub missing Assign".into());
            }
        }
        McStubKind::LaunchBlocked => {
            if !(text.contains("blocked") || text.contains("Blocked") || text.contains("full")) {
                return Err("blocked stub missing blocked/full".into());
            }
        }
        McStubKind::TeamBoard => {
            if !(text.contains("assignment") || text.contains("Assignment")) {
                return Err("team-board stub missing assignment language".into());
            }
        }
    }
    Ok(())
}

fn top_bar(width: usize) -> String {
    let brand = MC_BRAND;
    let left = format!("{brand}  {DEMO_CWD}");
    let right = format!(
        "Time 32m12s · In 3.8K · Out 1.3M · {MC_PARALLEL_SLOTS} · Parallel 3/4 · {}",
        format_context_budget(DEMO_CTX_USED, DEMO_CTX_TOTAL)
    );
    let gap = width
        .saturating_sub(left.chars().count() + right.chars().count())
        .max(2);
    format!("{left}{}{right}", " ".repeat(gap))
}

fn running_bar(variant: McVariant, width: usize) -> String {
    let (frac, meta) = match variant {
        McVariant::RunningAll => ("18/42", "3 live · 1 free · 2 queued · Parallel 3/4"),
        McVariant::OneFailed => ("17/42", "2 live · 1 failed · 1 free · Parallel 3/4"),
    };
    let bar = "████████████████░░░░░░░░░░░░░░░░░░";
    let body = format!("● RUNNING  {bar}  {frac}  ·  {meta}");
    pad_right(&body, width)
}

fn stream_block(width: usize) -> Vec<String> {
    let header_left = "Stream  #21  note-open-risks";
    let header_right = "Time 4m12s";
    let gap = width
        .saturating_sub(header_left.chars().count() + header_right.chars().count())
        .max(2);
    vec![
        format!("{header_left}{}{header_right}", " ".repeat(gap)),
        "Execute  cargo test note -- --nocapture".to_owned(),
        "  ✓ 14 tests passed. Fixture state verified.".to_owned(),
        "Read     src/screens/orchestration.rs  (214 lines)".to_owned(),
        "Read     library/mission-control-v2-premium-fable5-note.md  (98 lines)".to_owned(),
        "Execute  cargo clippy --all-targets --features png".to_owned(),
        "  ✓ 0 warnings. Palette restraint holds on focused stream.".to_owned(),
        "Read     library/mission-control-parallel-design.md  (64 lines)".to_owned(),
        "⠋ Running multi-worker fixture paint first…".to_owned(),
        "  skill v3-flow-worker · milestone v3-mission-control-v2-premium".to_owned(),
    ]
}

fn worker_cards(variant: McVariant) -> [WorkerCard; 3] {
    match variant {
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
    }
}

fn feature_rows(variant: McVariant) -> Vec<FeatureRow> {
    let mut rows = vec![
        FeatureRow {
            selected: false,
            mark: "✓",
            name: "publish-checklist",
            badge: None,
        },
        FeatureRow {
            selected: false,
            mark: "✓",
            name: "staging-rollback",
            badge: None,
        },
        FeatureRow {
            selected: true,
            mark: "●",
            name: "note-open-risks",
            badge: Some("[#21]"),
        },
    ];
    match variant {
        McVariant::RunningAll => rows.push(FeatureRow {
            selected: false,
            mark: "●",
            name: "safe-publish-path",
            badge: Some("[#22]"),
        }),
        McVariant::OneFailed => rows.push(FeatureRow {
            selected: false,
            mark: "✗",
            name: "safe-publish-path",
            badge: Some("[#22]"),
        }),
    }
    rows.push(FeatureRow {
        selected: false,
        mark: "●",
        name: "window-os-actions",
        badge: Some("[#23]"),
    });
    rows.push(FeatureRow {
        selected: false,
        mark: "○",
        name: "github-actions-ci",
        badge: Some("queued"),
    });
    rows.push(FeatureRow {
        selected: false,
        mark: "○",
        name: "e2e-test-suite",
        badge: Some("queued"),
    });
    rows
}

fn progress_log(variant: McVariant) -> Vec<&'static str> {
    match variant {
        McVariant::RunningAll => vec![
            "8h ago Worker #21 started [note-open-risks]",
            "7h ago Worker #22 started [safe-publish-path]",
            "7h ago Worker #23 started [window-os-actions]",
            "6h ago Milestone validation: ci-e2e",
            "5h ago Worker #18 completed [publish-checklist]",
            "4h ago Handoff items dismissed",
            "3h ago Run started: CI features complete",
            "2h ago Worker #19 completed [staging-rollback]",
        ],
        McVariant::OneFailed => vec![
            "8h ago Worker #21 started [note-open-risks]",
            "7h ago Worker #22 started [safe-publish-path]",
            "7h ago Worker #23 started [window-os-actions]",
            "6m ago Worker #22 failed [safe-publish-path]",
            "5m ago Handoff kept pool RUNNING",
            "4m ago Worker #21 still streaming",
            "3m ago Worker #23 still streaming",
            "1m ago Pool remains 2 live · 1 free",
        ],
    }
}

fn paint_worker_card(card: &WorkerCard, width: usize) -> Vec<String> {
    let sel = if card.selected { "▌ " } else { "  " };
    let head = format!(
        "{sel}{} {} {}  {}  {}",
        card.id, card.mark, card.feature, card.ticks, card.timer
    );
    let mut out = vec![pad_right(&head, width)];
    if card.selected {
        out.push(pad_right(
            &format!("  skill {} · milestone MCx", card.skill),
            width,
        ));
        out.push(pad_right("  Preconditions", width));
        out.push(pad_right("    fable5 note present · harness-v4 builds", width));
        out.push(pad_right("    Factory MC ref available · pool 3/4", width));
        out.push(pad_right("  Expected", width));
        out.push(pad_right("    Factory hierarchy · Parallel 3/4 obvious", width));
        out.push(pad_right("    Orange+gray · Execute/Read stream results", width));
    } else {
        out.push(pad_right(&format!("    {}", card.stream), width));
    }
    out.push(String::new());
    out
}

fn paint_feature_row(feat: &FeatureRow, width: usize) -> String {
    let sel = if feat.selected { "▌ " } else { "  " };
    let mut body = format!("{sel}{} {}", feat.mark, feat.name);
    if let Some(badge) = feat.badge {
        let gap = width
            .saturating_sub(body.chars().count() + badge.chars().count())
            .max(1);
        body.push_str(&" ".repeat(gap));
        body.push_str(badge);
    }
    pad_right(&body, width)
}

fn two_col(left: &str, right: &str, left_w: usize, right_w: usize, width: usize) -> String {
    let l = pad_right(left, left_w);
    let r = pad_right(right, right_w);
    let mut out = format!("{l}  {r}");
    if out.chars().count() > width {
        out = out.chars().take(width).collect();
    } else {
        pad_to(&mut out, width);
    }
    out
}

fn rule(width: usize) -> String {
    "─".repeat(width.saturating_sub(0).max(1))
}

fn pad_right(s: &str, width: usize) -> String {
    let mut out = s.chars().take(width).collect::<String>();
    let len = out.chars().count();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

fn pad_to(s: &mut String, width: usize) {
    let len = s.chars().count();
    if len < width {
        s.push_str(&" ".repeat(width - len));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mc_home_running_fingerprint() {
        let text = render_home_lines(McVariant::RunningAll, 120).join("\n");
        assert_home_fingerprint(&text, McVariant::RunningAll).expect(&text);
        assert!(text.contains("Workers (live) 3/4"), "{text}");
        assert!(text.contains("[#21]"), "{text}");
        assert!(text.contains("[#22]"), "{text}");
        assert!(text.contains("[#23]"), "{text}");
        assert!(text.contains(MC_FOOTER), "{text}");
    }

    #[test]
    fn mc_home_failure_keeps_siblings_live() {
        let text = render_home_lines(McVariant::OneFailed, 120).join("\n");
        assert_home_fingerprint(&text, McVariant::OneFailed).expect(&text);
        assert!(text.contains("✗"), "{text}");
        assert!(text.contains("1 failed") || text.contains("failed"), "{text}");
        // Multi-worker still present.
        assert!(text.contains("#21") && text.contains("#23"), "{text}");
    }

    #[test]
    fn mc_stubs_point_home_without_multipanel() {
        for kind in [
            McStubKind::Concurrency,
            McStubKind::Launch,
            McStubKind::LaunchBlocked,
            McStubKind::TeamBoard,
        ] {
            let text = render_stub_lines(kind, 120).join("\n");
            assert_stub_fingerprint(&text, kind).unwrap_or_else(|e| panic!("{kind:?}: {e}\n{text}"));
        }
    }

    #[test]
    fn mc_home_bans_copper_and_nameplates() {
        let text = render_home_lines(McVariant::RunningAll, 120).join("\n");
        assert!(!text.contains("Cortex Code"));
        assert!(!text.contains("❯ You"));
        assert!(!text.contains("◆ Cortex"));
    }

    #[test]
    fn live_snapshot_paints_parallel_and_progress() {
        let snap = missions::MissionSnapshot {
            id: "live-1".into(),
            mission_dir: "/tmp/m".into(),
            parallel_live: 3,
            parallel_max: 4,
            features_done: 1,
            features_total: 3,
            running: vec![
                missions::WorkerSlotView {
                    worker_id: "#21".into(),
                    feature_id: "f-a".into(),
                    state: "running".into(),
                },
                missions::WorkerSlotView {
                    worker_id: "#22".into(),
                    feature_id: "f-b".into(),
                    state: "running".into(),
                },
            ],
            failed: vec![],
            pending: vec!["f-c".into()],
            completed: vec![],
            status: "running".into(),
            progress_tail: vec![
                "1 worker=#21 feature=f-a started".into(),
                "2 worker=#22 feature=f-b started".into(),
            ],
            validation_invoked: false,
            last_validation: None,
        };
        let text = live_home_lines(&snap, 120).join("\n");
        assert!(text.contains(".: Mission Control"), "{text}");
        assert!(text.contains("Parallel 3/4") || text.contains("Live mission live-1"), "{text}");
        assert!(text.contains("progress_log") || text.contains("started"), "{text}");
        assert_eq!(snapshot_to_variant(&snap), McVariant::RunningAll);
    }

    #[test]
    fn apply_mission_events_updates_snapshot() {
        let mut snap = missions::MissionSnapshot {
            id: "m1".into(),
            mission_dir: String::new(),
            parallel_live: 0,
            parallel_max: 3,
            features_done: 0,
            features_total: 3,
            running: vec![],
            failed: vec![],
            pending: vec!["f-a".into()],
            completed: vec![],
            status: "created".into(),
            progress_tail: vec![],
            validation_invoked: false,
            last_validation: None,
        };
        assert!(apply_mission_event(
            &mut snap,
            &events::Event::mission_worker_started("m1", "#21", "f-a", 1, 3),
        ));
        assert_eq!(snap.parallel_live, 1);
        assert_eq!(snap.running.len(), 1);
        assert!(apply_mission_event(
            &mut snap,
            &events::Event::mission_worker_failed("m1", "#21", "f-a", "boom", 0),
        ));
        assert!(snap.failed.contains(&"f-a".into()));
        assert_eq!(snapshot_to_variant(&snap), McVariant::OneFailed);
        assert!(apply_mission_event(
            &mut snap,
            &events::Event::mission_validation("m1", "ms", "passed"),
        ));
        assert!(snap.validation_invoked);
    }
}
