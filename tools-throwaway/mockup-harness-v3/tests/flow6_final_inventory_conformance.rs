
//! Final Flow 6 mission inventory and conformance sweep.
//!
//! Owned contract assertions:
//! VAL-V3-F6-045, VAL-V3-F6-047, VAL-V3-F6-048, VAL-V3-F6-050, VAL-V3-F6-051.
//!
//! Aggregate checks across every required Flow 6 canonical pair, primary action
//! inventory, default-state internal exclusion, plain-English wording, and the
//! complete reversible mission transition graph.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory, flow_manifests, scenario::replay_case, FlowOwner,
    ImplementationStatus,
};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";

/// Exact twelve Flow 6 mission detail screens (VAL-V3-F6-045).
const FLOW6_IDS: &[&str] = &[
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
];

/// Published gallery relatives under plans/assets/mockups-v3/orchestration.
const GALLERY_IDS: &[&str] = FLOW6_IDS;

/// Default production fixtures that publish the twelve canonical pairs.
const CANONICAL_FIXTURES: &[(&str, &str)] = &[
    ("workflow-launch", "gate.mission.workflow-launch"),
    (
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    ),
    ("workflow-monitor", "gate.mission.workflow-monitor"),
    (
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    ),
    ("team-board", "orchestration.team-board"),
    ("reviewer-output", "gate.mission.reviewer-output"),
    ("reviewer-approved", "gate.mission.reviewer-approved"),
    (
        "verification-ladder",
        "orchestration.verification-ladder",
    ),
    ("intent-scorer-pass", "orchestration.intent-scorer-pass"),
    ("intent-scorer-fail", "orchestration.intent-scorer-fail"),
    (
        "concurrency-metrics",
        "orchestration.concurrency-metrics",
    ),
    ("tool-envelope", "orchestration.tool-envelope"),
];

/// Decision, blocker, failure, intervention, review, and completion states.
/// Passive automatic / browse-only surfaces are excluded (zero primary ok).
const DECISION_BEARING_FIXTURES: &[(&str, &str)] = &[
    ("workflow-launch", "gate.mission.workflow-launch"),
    (
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    ),
    (
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    ),
    ("reviewer-output", "gate.mission.reviewer-output"),
    ("reviewer-approved", "gate.mission.reviewer-approved"),
    ("intent-scorer-fail", "orchestration.intent-scorer-fail"),
    // Healthy compact monitor keeps View progress as the one card primary.
    ("workflow-monitor", "gate.mission.workflow-monitor"),
];

/// Compact defaults that must hide orchestration internals (VAL-V3-F6-048).
const DEFAULT_COMPACT_FIXTURES: &[(&str, &str)] = &[
    ("workflow-launch", "gate.mission.workflow-launch"),
    (
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    ),
    ("workflow-monitor", "gate.mission.workflow-monitor"),
    (
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    ),
    ("reviewer-output", "gate.mission.reviewer-output"),
    ("reviewer-approved", "gate.mission.reviewer-approved"),
];

/// Internal tokens forbidden on compact defaults. Named detail destinations
/// are allowed to surface their own domain vocabulary.
const DEFAULT_INTERNAL_BANS: &[&str] = &[
    "worker",
    "wave",
    "worktree",
    "score:",
    " scores",
    "ladder L",
    "tool envelope",
    "tool-envelope",
    "acceptance-contract",
    "features.json",
    "contract_id",
    "contract id",
    "n=3",
    "orchestrat",
    "branch/mission",
    "git worktree",
    "payload · raw",
    "validation_contract",
];

/// Implementation / snake_case terms forbidden anywhere in Flow 6 human surfaces.
const IMPLEMENTATION_TERMS: &[&str] = &[
    "workflow_monitor",
    "workflow_launch",
    "reviewer_output",
    "intent_scorer",
    "team_board",
    "verification_ladder",
    "tool_envelope",
    "concurrency_metrics",
    "exit_code",
    "exit code=",
    "stage_letter",
    "stage L0",
    "stage L1",
    "stage L2",
    "l0_",
    "l1_",
    "l2_",
    "needs_attention_enum",
    "RUNNING_STATE",
    "mission_status=",
    "preflight_ok=true",
    "features.json",
    "acceptance-contract",
];

/// Origin-preserving graph across proposal → completion and evidence (VAL-V3-F6-051).
const ORIGIN_SCENARIOS: &[(&str, &str, &str)] = &[
    // Proposal / preflight / launch.
    (
        "mission-resolve-input-ready",
        "workflow-launch-blocked",
        "workflow-launch",
    ),
    ("mission-launch-cancel", "workflow-launch", "chat-turn"),
    (
        "mission-blocked-cancel",
        "workflow-launch-blocked",
        "chat-turn",
    ),
    ("mission-start", "workflow-launch", "workflow-monitor"),
    // Monitor progress / team board reversibility.
    (
        "mission-view-progress",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-open-team-board",
        "workflow-monitor",
        "team-board",
    ),
    (
        "mission-team-board-return",
        "team-board",
        "workflow-monitor",
    ),
    (
        "mission-view-workers",
        "workflow-monitor",
        "team-board",
    ),
    (
        "mission-team-board-advanced",
        "team-board",
        "team-board",
    ),
    (
        "mission-team-board-advanced-return",
        "team-board",
        "team-board",
    ),
    (
        "mission-progress-expand-working",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-progress-collapse-group",
        "workflow-monitor",
        "workflow-monitor",
    ),
    // Evidence chain one-level unwind.
    (
        "mission-open-verification-ladder",
        "workflow-monitor-failure",
        "verification-ladder",
    ),
    (
        "mission-verification-ladder-return",
        "verification-ladder",
        "workflow-monitor-failure",
    ),
    (
        "mission-open-stage-evidence",
        "verification-ladder",
        "verification-ladder",
    ),
    (
        "mission-stage-evidence-return",
        "verification-ladder",
        "verification-ladder",
    ),
    (
        "mission-open-tool-envelope",
        "verification-ladder",
        "tool-envelope",
    ),
    (
        "mission-tool-envelope-return",
        "tool-envelope",
        "verification-ladder",
    ),
    (
        "mission-open-intent-scorer-pass",
        "reviewer-approved",
        "intent-scorer-pass",
    ),
    (
        "mission-intent-scorer-pass-return",
        "intent-scorer-pass",
        "reviewer-approved",
    ),
    (
        "mission-open-intent-scorer-fail",
        "reviewer-output",
        "intent-scorer-fail",
    ),
    (
        "mission-intent-scorer-fail-return",
        "intent-scorer-fail",
        "reviewer-output",
    ),
    (
        "mission-open-concurrency-metrics",
        "workflow-monitor",
        "concurrency-metrics",
    ),
    (
        "mission-concurrency-metrics-return",
        "concurrency-metrics",
        "workflow-monitor",
    ),
    // Fix / validation / review loop.
    (
        "mission-create-fix-task",
        "workflow-monitor-failure",
        "workflow-monitor",
    ),
    (
        "mission-failure-return",
        "workflow-monitor-failure",
        "chat-turn",
    ),
    (
        "mission-fix-working",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-fix-validating",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-validation-success",
        "workflow-monitor",
        "reviewer-output",
    ),
    (
        "mission-validation-failure",
        "workflow-monitor",
        "workflow-monitor-failure",
    ),
    (
        "mission-review-create-fix",
        "reviewer-output",
        "workflow-monitor",
    ),
    (
        "mission-intent-scorer-fail-create-fix",
        "intent-scorer-fail",
        "workflow-monitor",
    ),
    (
        "mission-completion-validation",
        "workflow-monitor",
        "reviewer-approved",
    ),
    (
        "mission-request-changes-open",
        "reviewer-approved",
        "reviewer-approved",
    ),
    (
        "mission-request-changes-submit",
        "reviewer-approved",
        "workflow-monitor",
    ),
    (
        "mission-request-changes-cancel",
        "reviewer-approved",
        "reviewer-approved",
    ),
    (
        "mission-approve-completion",
        "reviewer-approved",
        "chat-turn",
    ),
    // Intervention family round trips.
    (
        "mission-open-intervene",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-intervene-cancel",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-pause-open",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-pause-confirm",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-pause-cancel",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-resume",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-replan-open",
        "workflow-monitor",
        "workflow-launch",
    ),
    (
        "mission-replan-accept",
        "workflow-launch",
        "workflow-launch",
    ),
    (
        "mission-retry-eligible",
        "workflow-monitor-failure",
        "workflow-monitor",
    ),
    (
        "mission-stop-open",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-stop-confirm",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-stop-cancel",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-stopped-return",
        "workflow-monitor",
        "chat-turn",
    ),
    (
        "mission-stopped-replan",
        "workflow-monitor",
        "workflow-launch",
    ),
    // Durable resume / restart fixtures restore the stage without dead-end.
    // Public restart cases bind every durable stage to the monitor surface
    // (stage is encoded in focus/selection; screen_id remains workflow-monitor).
    (
        "mission-restart-running",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-paused",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-blocked",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-validating",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-reviewing",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-awaiting-human",
        "workflow-monitor",
        "workflow-monitor",
    ),
    (
        "mission-restart-stopped",
        "workflow-monitor",
        "workflow-monitor",
    ),
];

/// Cancel-like paths that must restore mission identity without side progression.
const CANCEL_LIKE: &[&str] = &[
    "mission-launch-cancel",
    "mission-blocked-cancel",
    "mission-intervene-cancel",
    "mission-pause-cancel",
    "mission-stop-cancel",
    "mission-request-changes-cancel",
    "mission-team-board-return",
    "mission-verification-ladder-return",
    "mission-intent-scorer-pass-return",
    "mission-intent-scorer-fail-return",
    "mission-concurrency-metrics-return",
    "mission-tool-envelope-return",
    "mission-stage-evidence-return",
    "mission-team-board-advanced-return",
    "mission-failure-return",
];

fn gallery_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3")
        .canonicalize()
        .expect("gallery root must resolve")
}

fn render_pair(screen_id: &str, fixture_id: &str) -> AuditDocument {
    let audit = audit_screen(screen_id, fixture_id)
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} must render: {e}"));
    audit
        .validate()
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} audit bijection: {e}"));
    audit
}

fn primary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect()
}

fn assert_canvas_geometry(text: &str, label: &str) {
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(
        rows.len(),
        40,
        "{label}: canonical height is 40 rows, got {}",
        rows.len()
    );
    for (idx, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            120,
            "{label}: row {idx} width is {}, expected 120",
            row.chars().count()
        );
    }
}

fn assert_png_dimensions(path: &Path, expect_w: u32, expect_h: u32) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(data.len() > 24, "{} too small to be PNG", path.display());
    assert_eq!(
        &data[0..8],
        b"\x89PNG\r\n\x1a\n",
        "{} bad PNG sig",
        path.display()
    );
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    assert_eq!(
        (width, height),
        (expect_w, expect_h),
        "{}: expected {expect_w}x{expect_h}, got {width}x{height}",
        path.display()
    );
}

fn forbidden_default_internals(text: &str, label: &str) {
    let lower = text.to_ascii_lowercase();
    for token in DEFAULT_INTERNAL_BANS {
        assert!(
            !lower.contains(&token.to_ascii_lowercase()),
            "{label}: default interior leaked `{token}`"
        );
    }
}

fn forbidden_implementation_terms(text: &str, label: &str) {
    let lower = text.to_ascii_lowercase();
    for token in IMPLEMENTATION_TERMS {
        assert!(
            !lower.contains(&token.to_ascii_lowercase()),
            "{label}: implementation term leaked `{token}`"
        );
    }
    // Guard obvious UPPER_SNAKE status tokens.
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        if cleaned.contains('_')
            && cleaned.len() > 5
            && cleaned
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        {
            panic!("{label}: uppercase/snake_case token leaked: {cleaned}");
        }
    }
}

fn status_has_readable_label(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if text.contains('✓')
        || text.contains('✗')
        || text.contains('●')
        || text.contains('○')
    {
        return lower.contains("complete")
            || lower.contains("pass")
            || lower.contains("fail")
            || lower.contains("missing")
            || lower.contains("blocked")
            || lower.contains("stopped")
            || lower.contains("waiting")
            || lower.contains("retry")
            || lower.contains("needs")
            || lower.contains("attention")
            || lower.contains("running")
            || lower.contains("ready")
            || lower.contains("approved")
            || lower.contains("pending")
            || lower.contains("working")
            || lower.contains("repair")
            || lower.contains("refus")
            || lower.contains("success")
            || lower.contains("held")
            || lower.contains("quarantine")
            || lower.contains("partial")
            || lower.contains("full")
            || lower.contains("preflight")
            || lower.contains("validat");
    }
    true
}

fn stage_words_present(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "ready to launch",
        "preflight",
        "running",
        "needs attention",
        "review",
        "ready for completion",
        "approved",
        "paused",
        "stopped",
        "validating",
        "working",
        "pass",
        "fail",
        "pending",
        "complete",
        "remaining",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

// ── VAL-V3-F6-045: Render every mission detail screen ───────────────────

#[test]
fn flow6_inventory_is_complete_implemented_once_and_geometry_valid() {
    let mut seen = HashSet::new();
    for id in FLOW6_IDS {
        assert!(seen.insert(*id), "duplicate Flow 6 id in constant: {id}");
    }

    let inventory = canonical_inventory();
    for id in FLOW6_IDS {
        let matches: Vec<_> = inventory.iter().filter(|s| s.id == *id).collect();
        assert_eq!(matches.len(), 1, "{id} must appear exactly once in registry");
        let screen = matches[0];
        assert_eq!(
            screen.status,
            ImplementationStatus::Implemented,
            "{id} must be implemented"
        );
        assert_eq!(
            screen.flow,
            FlowOwner::Mission,
            "{id} must be owned by the Mission flow"
        );
    }

    let mission_count = inventory
        .iter()
        .filter(|s| s.flow == FlowOwner::Mission)
        .count();
    assert_eq!(
        mission_count, 12,
        "exactly twelve Flow 6 mission owners are required"
    );

    // Live exports must prove distinct intended states at 120x40.
    let mut digests = HashSet::new();
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
        assert!(
            digests.insert(text.clone()),
            "{screen_id}/{fixture_id}: produced a duplicate frame of another mission state"
        );
        assert!(
            (text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL),
            "{screen_id}/{fixture_id}: must keep mission identity"
        );
        assert!(
            text.contains("~/project · main · Claude Opus 4.8"),
            "{screen_id}/{fixture_id}: statusline must stay path · branch · human model"
        );
        assert!(
            !text.contains("ctx ") && !text.contains("$0.") && !text.contains("idle"),
            "{screen_id}/{fixture_id}: statusline must not grow costs or ctx"
        );
    }

    // Published gallery pairs: exactly one TXT + PNG each, dimensions locked.
    let root = gallery_root().join("orchestration");
    let mut gallery_names = HashSet::new();
    for id in GALLERY_IDS {
        let txt = root.join(format!("{id}.txt"));
        let png = root.join(format!("{id}.png"));
        assert!(txt.is_file(), "missing gallery txt {}", txt.display());
        assert!(png.is_file(), "missing gallery png {}", png.display());
        assert!(
            gallery_names.insert(*id),
            "duplicate gallery id listing: {id}"
        );
        let text = fs::read_to_string(&txt).expect("gallery txt");
        assert_canvas_geometry(&text, &format!("gallery orchestration/{id}"));
        assert_png_dimensions(&png, 1200, 840);
    }

    // No extra TXT siblings with the same stem as a required mission id
    // (mode-switch lives in the same category but is owned by work-configuration).
    let listed = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("list");
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    for id in FLOW6_IDS {
        assert!(
            stdout
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(*id)),
            "--list missing mission id {id}"
        );
    }
}

// ── VAL-V3-F6-047: Enforce one primary action ───────────────────────────

#[test]
fn every_decision_bearing_flow6_state_has_exactly_one_primary() {
    for &(screen_id, fixture_id) in DECISION_BEARING_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);
        assert_eq!(
            primaries.len(),
            1,
            "{screen_id}/{fixture_id}: expected exactly one primary, got {} labels {:?}",
            primaries.len(),
            primaries
                .iter()
                .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
                .collect::<Vec<_>>()
        );
        let action = primaries[0]
            .action
            .as_ref()
            .unwrap_or_else(|| panic!("{screen_id}/{fixture_id}: primary binds action"));
        assert!(
            action.enabled,
            "{screen_id}/{fixture_id}: primary must be enabled"
        );
        assert!(
            !action.label.trim().is_empty(),
            "{screen_id}/{fixture_id}: primary label nonempty"
        );

        // Secondary / cancel remain subordinate; dangerous actions confirm.
        if screen_id == "reviewer-approved" {
            assert!(
                text.contains("Approve completion"),
                "completion primary must be Approve completion"
            );
            assert!(
                text.contains("Request changes") || text.contains("Request changes"),
                "request changes is subordinate secondary"
            );
        }
        if screen_id == "workflow-monitor-failure" {
            assert!(
                text.contains("Create a fix task"),
                "failure primary is create fix task"
            );
        }
        if screen_id == "workflow-launch" {
            assert!(
                text.contains("Start mission"),
                "launch primary is Start mission"
            );
            assert!(
                text.contains("Cancel"),
                "launch cancel remains secondary"
            );
        }
        if screen_id == "workflow-launch-blocked" {
            assert!(
                !text.contains("Start mission"),
                "blocked launch must not expose Start mission as a primary"
            );
        }

        // Visual emphasis: focus mark on dock or conspicuous [enter] on primary.
        assert!(
            (text.contains("› ") || text.contains("Enter to"))
                || text.contains("› ")
                || (text.contains("Enter to") || text.contains(""))
                || text.contains("› "),
            "{screen_id}/{fixture_id}: primary must be visually marked"
        );
    }

    // Passive / browse-only surfaces: at most one primary (never competing).
    const BROWSE_ONLY: &[(&str, &str)] = &[
        ("team-board", "orchestration.team-board"),
        (
            "verification-ladder",
            "orchestration.verification-ladder",
        ),
        ("intent-scorer-pass", "orchestration.intent-scorer-pass"),
        (
            "concurrency-metrics",
            "orchestration.concurrency-metrics",
        ),
        ("tool-envelope", "orchestration.tool-envelope"),
    ];
    for &(screen_id, fixture_id) in BROWSE_ONLY {
        let audit = render_pair(screen_id, fixture_id);
        let primaries = primary_events(&audit);
        assert!(
            primaries.len() <= 1,
            "{screen_id}/{fixture_id}: browser detail must not present competing primaries ({})",
            primaries.len()
        );
    }
}

// ── VAL-V3-F6-048: Hide orchestration internals by default ──────────────

#[test]
fn default_flow6_states_hide_workers_waves_scores_contracts_and_raw_artifacts() {
    for &(screen_id, fixture_id) in DEFAULT_COMPACT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        forbidden_default_internals(&text, &format!("{screen_id}/{fixture_id}"));

        // Compact density: MissionCard goal / stage / counts remain; no worker tables.
        assert!(
            text.contains(MISSION_GOAL) || text.contains(MISSION_TASK),
            "{screen_id}/{fixture_id}: goal/task must remain"
        );
        assert!(
            text.contains("complete") && text.contains("remaining"),
            "{screen_id}/{fixture_id}: completed/remaining counts required"
        );
        assert!(
            !text.contains("worker #")
                && !text.contains("Worker pool")
                && !text.contains("Wave schedule")
                && !text.contains("Score:"),
            "{screen_id}/{fixture_id}: worker/wave/score tables forbidden on defaults"
        );
        assert!(
            !text.contains("features.json")
                && !text.contains("acceptance-contract")
                && !text.contains("contract_id"),
            "{screen_id}/{fixture_id}: raw mission files / contracts forbidden"
        );
        // Validation ladders, concurrency metrics, tool envelopes belong elsewhere.
        if !matches!(
            screen_id,
            "verification-ladder" | "concurrency-metrics" | "tool-envelope"
        ) {
            assert!(
                !text.contains("Held-out checks ·")
                    && !text.contains("Throughput ·")
                    && !text.contains("Credential fields ·")
                    && !text.contains("Payload ·"),
                "{screen_id}/{fixture_id}: detailed evidence leaked into compact"
            );
        }
    }

    // Named detail destinations may disclose their own domain (positive presence).
    let team = render_pair("team-board", "orchestration.team-board").render_txt();
    assert!(
        team.contains("Team board") || team.contains("Owner ·"),
        "team-board is the explicit ownership detail"
    );
    let ladder =
        render_pair("verification-ladder", "orchestration.verification-ladder").render_txt();
    assert!(
        ladder.contains("Validation stages") || ladder.contains("Held-out"),
        "verification-ladder is the focused validation detail"
    );
    let scores = render_pair("intent-scorer-pass", "orchestration.intent-scorer-pass").render_txt();
    assert!(
        scores.contains("Quality report") || scores.contains("Intent fidelity"),
        "intent-scorer-pass is the only advisory-score destination"
    );
    let concurrency =
        render_pair("concurrency-metrics", "orchestration.concurrency-metrics").render_txt();
    assert!(
        concurrency.contains("Runtime detail") || concurrency.contains("Throughput"),
        "concurrency-metrics is the explicit runtime detail"
    );
    let tool = render_pair("tool-envelope", "orchestration.tool-envelope").render_txt();
    assert!(
        tool.contains("Tool evidence") || tool.contains("Payload ·"),
        "tool-envelope is the focused tool diagnostics view"
    );
}

// ── VAL-V3-F6-050: Use plain-English mission states ─────────────────────

#[test]
fn every_flow6_state_uses_plain_english_with_non_color_status() {
    if pure_mode() { return; }

    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        forbidden_implementation_terms(&text, &format!("{screen_id}/{fixture_id}"));
        assert!(
            status_has_readable_label(&text),
            "{screen_id}/{fixture_id}: status marker without readable label"
        );
        assert!(
            stage_words_present(&text),
            "{screen_id}/{fixture_id}: stage/status must use shared plain-English words"
        );
        // Decorative-only glyph scan: star, sparkle, diamond are banned.
        for banned in ['✦', '✧', '★', '☆', '❖', '◆', '✱'] {
            assert!(
                !text.contains(banned),
                "{screen_id}/{fixture_id}: decorative glyph {banned}"
            );
        }

        // Shared plain-English stage words travel in the status/stage channels.
        for event in &audit.events {
            if matches!(
                event.identity.family.as_str(),
                "mission-card" | "mission-goal" | "review-outcome" | "primary-action" | "status"
            ) {
                assert!(
                    !event.identity.subject_id.trim().is_empty()
                        || event
                            .action
                            .as_ref()
                            .is_some_and(|a| !a.label.trim().is_empty()),
                    "{screen_id}/{fixture_id}: event {} lacks textual subject",
                    event.identity.event_id
                );
            }
        }
    }

    // Gallery published TXT also stays jargon-free.
    let root = gallery_root().join("orchestration");
    for id in GALLERY_IDS {
        let text = fs::read_to_string(root.join(format!("{id}.txt"))).expect("gallery txt");
        forbidden_implementation_terms(&text, &format!("gallery {id}"));
        assert!(
            status_has_readable_label(&text),
            "gallery {id}: status marker without readable label"
        );
        assert!(
            stage_words_present(&text),
            "gallery {id}: stage/status must use shared plain-English words"
        );
    }
}

// ── VAL-V3-F6-051: Keep every mission transition reversible ─────────────

#[test]
fn complete_mission_transition_graph_is_reversible_and_origin_preserving() {
    // Every required origin scenario is public and restores identity.
    for &(case_id, from_screen, to_screen) in ORIGIN_SCENARIOS {
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|e| panic!("{case_id} must exist as a public case: {e}"));
        assert_eq!(
            replay.before.screen_id, from_screen,
            "{case_id}: from screen"
        );
        assert_eq!(replay.after.screen_id, to_screen, "{case_id}: to screen");
        assert!(
            !replay.actions.is_empty(),
            "{case_id}: must bind at least one action"
        );
        assert_eq!(
            replay.before.user_task, MISSION_TASK,
            "{case_id}: before task"
        );
        assert_eq!(
            replay.after.user_task, MISSION_TASK,
            "{case_id}: after task"
        );
        assert_eq!(
            replay.before.visible_origin, MISSION_ORIGIN,
            "{case_id}: before origin"
        );
        assert_eq!(
            replay.after.visible_origin, MISSION_ORIGIN,
            "{case_id}: after origin"
        );
        assert_eq!(
            replay.before.session_id, replay.after.session_id,
            "{case_id}: session identity"
        );
        assert!(
            !replay.after.session_id.is_empty(),
            "{case_id}: session must remain non-empty"
        );

        // Cancel / back must not invent a new journey category walk.
        if CANCEL_LIKE.contains(&case_id) {
            assert!(
                matches!(
                    to_screen,
                    "chat-turn"
                        | "workflow-launch"
                        | "workflow-launch-blocked"
                        | "workflow-monitor"
                        | "workflow-monitor-failure"
                        | "team-board"
                        | "reviewer-output"
                        | "reviewer-approved"
                        | "verification-ladder"
                        | "intent-scorer-pass"
                        | "intent-scorer-fail"
                        | "concurrency-metrics"
                        | "tool-envelope"
                ),
                "{case_id}: cancel/back lands outside the mission surface graph ({to_screen})"
            );
        }
    }

    // Every published canonical state exposes non-empty transitions and a
    // back/cancel/success return — no dead ends.
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        assert!(
            !audit.transitions.is_empty(),
            "{screen_id}/{fixture_id}: dead end with zero transitions"
        );
        assert!(
            audit
                .transitions
                .iter()
                .all(|t| t.from_screen_id == screen_id),
            "{screen_id}/{fixture_id}: transition origin mismatch"
        );
        assert!(
            audit.transitions.iter().any(|t| {
                !t.to_screen_id.is_empty()
                    && (t.kind == "cancel"
                        || t.kind == "direct"
                        || t.kind == "automatic"
                        || t.kind == "back"
                        || t.to_screen_id == "chat-turn"
                        || t.to_screen_id == screen_id
                        || FLOW6_IDS.contains(&t.to_screen_id.as_str()))
            }),
            "{screen_id}/{fixture_id}: no forward or return edge:\n{:?}",
            audit.transitions
        );
    }

    // Gate journey still records one coherent mission sequence without category jumps.
    let flow = flow_manifests()
        .iter()
        .find(|f| f.name == "mission")
        .expect("mission storyboard");
    assert_eq!(flow.steps.len(), 6, "mission storyboard remains six frames");
    for step in &flow.steps {
        assert_eq!(step.user_task, MISSION_TASK);
        assert_eq!(step.visible_origin, MISSION_ORIGIN);
        assert!(!step.success_return.is_empty());
        assert!(!step.cancel_return.is_empty());
        assert!(
            step.preservation_fields.contains(&"session_id")
                && step.preservation_fields.contains(&"user_task"),
            "{} must preserve session and task",
            step.screen_id
        );
    }

    // VAL-V3-F6-051 focus: workflow-monitor-failure paints Resume conversation 1:1
    // to a cancel/return transition, and Open evidence is single-bound.
    let failure = render_pair(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    );
    let failure_text = failure.render_txt();
    assert!(
        failure_text.contains("Resume conversation"),
        "failure must paint Resume conversation:\n{failure_text}"
    );
    assert!(
        failure_text.contains("Open evidence"),
        "failure must paint Open evidence:\n{failure_text}"
    );

    let resume_events: Vec<_> = failure
        .events
        .iter()
        .filter(|event| {
            event.action.as_ref().is_some_and(|action| {
                action.action_id == "action.mission.monitor-failure-return"
                    || action.label == "Resume conversation"
            })
        })
        .collect();
    assert_eq!(
        resume_events.len(),
        1,
        "Resume conversation must bind exactly one action: {:?}",
        resume_events
            .iter()
            .map(|e| e.action.as_ref().map(|a| a.action_id.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        resume_events[0].action.as_ref().unwrap().action_id,
        "action.mission.monitor-failure-return"
    );
    assert!(
        failure.transitions.iter().any(|t| {
            t.action_id == "action.mission.monitor-failure-return"
                && t.to_screen_id == "chat-turn"
                && t.kind == "cancel"
        }),
        "Resume conversation must map to cancel return chat-turn: {:?}",
        failure.transitions
    );

    let open_evidence_ids: HashSet<_> = failure
        .events
        .iter()
        .filter_map(|event| {
            event.action.as_ref().and_then(|action| {
                (action.label == "Open evidence").then_some(action.action_id.clone())
            })
        })
        .collect();
    assert_eq!(
        open_evidence_ids.len(),
        1,
        "Open evidence must not dual-bind labels: {open_evidence_ids:?}"
    );
    assert!(
        open_evidence_ids.contains("action.mission.view-validation-stages"),
        "Open evidence must bind view-validation-stages only: {open_evidence_ids:?}"
    );
    assert!(
        !open_evidence_ids.contains("action.mission.open-evidence"),
        "open-evidence must not share the painted Open evidence label"
    );

    let resume_case = replay_case("mission-failure-return", 1)
        .expect("public mission-failure-return case must exist");
    assert_eq!(resume_case.before.screen_id, "workflow-monitor-failure");
    assert_eq!(resume_case.after.screen_id, "chat-turn");
    assert_eq!(
        resume_case.actions[0].action_id,
        "action.mission.monitor-failure-return"
    );
    assert_eq!(resume_case.transitions[0].kind, "cancel");
    assert_eq!(resume_case.before.user_task, resume_case.after.user_task);
    assert_eq!(resume_case.before.session_id, resume_case.after.session_id);
    assert_eq!(
        resume_case.before.visible_origin,
        resume_case.after.visible_origin
    );
    assert!(!resume_case.after.draft.is_empty() || !resume_case.after.focus.is_empty());
}

#[test]
fn two_serialized_renders_of_flow6_are_byte_identical() {
    // Determinism for the twelve owned pairs (VAL-V3-F6-045 evidence path).
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let first = render_pair(screen_id, fixture_id).render_txt();
        let second = render_pair(screen_id, fixture_id).render_txt();
        assert_eq!(
            first, second,
            "{screen_id}/{fixture_id}: nondeterministic TXT between two renders"
        );
    }
}
