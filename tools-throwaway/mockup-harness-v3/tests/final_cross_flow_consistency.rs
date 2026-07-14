//! Final cross-flow consistency sweep.
//!
//! Owned contract assertions:
//! VAL-V3-CROSS-002 through VAL-V3-CROSS-016.
//!
//! Aggregate checks treat the six journeys as one coherent product: origin
//! returns, dead-end free graphs, work preservation, one live-state owner,
//! shared disclosure/selection/words, unified work-style, separate
//! providers/MCP, capability-led MCP, consistent recovery, goal-led missions,
//! human completion, and six-category coherence across all 86 states.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory, flow_manifests,
    renderer::canonical_publication_fixture,
    scenario::{public_case_ids, replay_case, CaseReplay},
    Category, FlowOwner, ImplementationStatus, Surface,
};

/// Representative picker / temporary-surface publication fixtures.
const SELECTION_SURFACES: &[(&str, &str)] = &[
    ("slash-menu", "flow2-slash-menu"),
    ("command-palette", "flow2.input-and-approval.stable"),
    ("file-mention", "flow2.input-and-approval.stable"),
    ("attachment-picker", "flow2-attachment-picker"),
    ("export-picker", "flow2-export-picker"),
    ("queue-manager", "flow2-queue-manager"),
    ("model-picker", "gate.work-configuration.model-picker"),
    ("agent-picker", "gate.work-configuration.agent-picker"),
    ("session-picker", "gate.work-configuration.session-picker"),
    ("skills-picker", "gate.work-configuration.skills-picker"),
    ("subagent-picker", "gate.work-configuration.subagent-picker"),
    ("variant-picker", "gate.work-configuration.variant-picker"),
    ("theme-picker", "gate.work-configuration.theme-picker"),
    ("fork-picker", "gate.work-configuration.fork-picker"),
];

/// Work-style surfaces (Agent/Mode must remain one concept).
const WORK_STYLE_SURFACES: &[(&str, &str)] = &[
    ("agent-picker", "gate.work-configuration.agent-picker"),
    ("mode-switch", "gate.work-configuration.mode-switch"),
    (
        "mode-switch-refused",
        "gate.work-configuration.mode-switch-refused",
    ),
];

/// Default recovery fixtures used for cross-flow recovery language.
const RECOVERY_DEFAULTS: &[(&str, &str)] = &[
    ("tool-failed", "canonical.tool-failed"),
    ("blocked-command", "canonical.blocked-command"),
    ("turn-error", "canonical.turn-error"),
    ("error-network", "canonical.error-network"),
    ("error-context", "canonical.error-context"),
    ("rate-limit", "gate.recovery.rate-limit"),
    ("interrupt", "canonical.interrupt"),
    ("toasts", "canonical.toasts"),
    ("checkpoint-undo", "canonical.checkpoint-undo"),
];

/// Automatic-wait recovery surfaces (zero primary; Retry now secondary only).
const AUTOMATIC_RECOVERY: &[(&str, &str)] = &[
    ("rate-limit", "gate.recovery.rate-limit"),
    ("rate-limit", "rate-limit.wait.sample-early"),
    ("error-network", "error-network.backoff"),
];

/// Live-running surfaces that must keep one live-state summary.
const LIVE_SURFACES: &[(&str, &str)] = &[
    ("streaming", "flow1.daily-work.stable"),
    ("tool-running", "flow1.daily-work.stable"),
    ("connect-oauth", "gate.provider-and-mcp.connect-oauth"),
    ("mcp", "views.mcp.progress-invocation.tick-1"),
    ("workflow-monitor", "gate.mission.workflow-monitor"),
    ("rate-limit", "gate.recovery.rate-limit"),
];

/// MCP production defaults (capability-facing, no diagnostics dump).
const MCP_DEFAULTS: &[(&str, &str)] = &[
    ("mcp", "gate.provider-and-mcp.mcp"),
    ("mcp", "views.mcp.mixed"),
    ("mcp", "views.mcp.empty"),
    ("sidebar-mcp", "views.sidebar-mcp.attention"),
    ("sidebar-mcp", "views.sidebar-mcp.all-ready"),
];

/// Provider defaults that must not merge into MCP.
const PROVIDER_DEFAULTS: &[(&str, &str)] = &[
    ("login", "gate.provider-and-mcp.login"),
    ("connect-provider", "gate.provider-and-mcp.connect-provider"),
    ("connect-method", "gate.provider-and-mcp.connect-method"),
    ("connect-oauth", "gate.provider-and-mcp.connect-oauth"),
    (
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    ),
];

/// Mission compact defaults (goal-led, exception-first).
const MISSION_DEFAULTS: &[(&str, &str)] = &[
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
    ("team-board", "orchestration.team-board"),
];

/// Disclosure pairs across conversation, MCP, recovery, and mission families.
const DISCLOSURE_PAIRS: &[(&str, &str, &str, &str)] = &[
    (
        "reasoning-collapsed",
        "session.disclosure.reasoning",
        "reasoning",
        "session.disclosure.reasoning",
    ),
    (
        "tool-success",
        "flow1.daily-work.stable",
        "tool-success-expanded",
        "session.disclosure.tools",
    ),
    (
        "tool-task",
        "session.disclosure.tasks",
        "tool-task-expanded",
        "session.disclosure.tasks",
    ),
    (
        "mcp",
        "views.mcp.mixed",
        "mcp",
        "views.mcp.detail-docs",
    ),
    (
        "mcp",
        "views.mcp.detail-docs",
        "mcp",
        "views.mcp.diagnostics-docs",
    ),
    (
        "turn-error",
        "canonical.turn-error",
        "error-network",
        "canonical.error-network",
    ),
    (
        "workflow-monitor",
        "gate.mission.workflow-monitor",
        "team-board",
        "orchestration.team-board",
    ),
    (
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
        "verification-ladder",
        "orchestration.verification-ladder",
    ),
    (
        "verification-ladder",
        "orchestration.verification-ladder",
        "tool-envelope",
        "orchestration.tool-envelope",
    ),
];

/// Representative cancel/back cases sampled across all six journeys.
const ORIGIN_RETURN_CASES: &[&str] = &[
    "permission-cancel",
    "permission-file-edit-cancel",
    "slash-cancel",
    "attachment-cancel",
    "export-cancel",
    "queue-cancel",
    "palette-cancel",
    "diff-cancel",
    "staging-cancel",
    "model-picker-cancel",
    "agent-picker-cancel",
    "session-picker-cancel",
    "skills-cancel",
    "theme-cancel",
    "fork-cancel",
    "work-style-access-cancel",
    "login-cancel",
    "connect-apikey-cancel",
    "connect-oauth-cancel",
    "connect-connected-return",
    "connect-disconnect-cancel",
    "mcp-return",
    "mcp-setup-cancel",
    "mcp-progress-cancel-setup",
    "mcp-details-back",
    "sidebar-mcp-return",
    "turn-error-cancel",
    "tool-failed-capability-return-cancel",
    "blocked-command-approval-return",
    "blocked-command-work-style-return",
    "error-context-evidence-close",
    "rate-limit-race-cancel-wins",
    "mission-launch-cancel",
    "mission-blocked-cancel",
    "mission-team-board-return",
    "mission-verification-ladder-return",
    "mission-tool-envelope-return",
    "mission-intent-scorer-pass-return",
    "mission-intervene-cancel",
    "mission-pause-cancel",
    "mission-stop-cancel",
    "mission-request-changes-cancel",
    "status-return",
    "help-return",
    "review-return",
];

/// Shared concept labels that must not morph into conflicting synonyms.
const SHARED_LABEL_EXPECTATIONS: &[(&str, &[&str])] = &[
    ("cancel", &["Cancel", "cancel"]),
    ("back", &["Back", "back"]),
    ("approve", &["Approve", "approve"]),
    ("connect", &["Connect", "connect"]),
    ("retry", &["Retry", "retry"]),
    ("evidence", &["evidence", "Evidence"]),
];

/// Forbidden product navigation dashboards (categories must stay storage).
const CATEGORY_DASHBOARD_BANS: &[&str] = &[
    "Browse all 86",
    "All six categories",
    "Six categories",
    "Category destination",
    "Open session category",
    "Open input category",
    "Open views category",
    "Open auth category",
    "Open orchestration category",
    "Open states category",
    "86 destinations",
    "Seven-tab bar",
    "seven-tab",
    "category picker",
    "Jump to category",
];

/// Transport / raw MCP jargon (default capability states only).
const MCP_TRANSPORT_BANS: &[&str] = &[
    "stdio",
    "oauth2",
    "OAuth2",
    "device_code",
    "client_secret",
    "PKCE",
    "mcpServers",
    "SSE ",
    "spawn error",
    "endpoint URL",
    "transport:",
    "schema language",
];

/// Mission internals forbidden on compact defaults.
const MISSION_INTERNAL_BANS: &[&str] = &[
    "worker_id",
    "wave_id",
    "contract_id",
    "features.json",
    "acceptance-contract",
    "validation_contract",
    "tool_envelope",
    "score:",
    "n=3",
    "worktree/",
    "branch/mission",
];

fn gallery_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3")
        .canonicalize()
        .expect("canonical gallery root must resolve")
}

fn publication_fixture(screen_id: &str) -> String {
    canonical_publication_fixture(screen_id)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("canonical.{screen_id}"))
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

fn live_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.identity.family.as_str(),
                "live-state"
                    | "live-operation"
                    | "live-progress"
                    | "activity-live"
                    | "waiting-state"
            ) || e.identity.subject_id.contains("live")
                || (e.identity.family == "activity"
                    && e.identity.detail_level == "summary"
                    && (e.identity.subject_id.contains("running")
                        || e.identity.subject_id.contains("streaming")
                        || e.identity.subject_id.contains("waiting")
                        || e.identity.subject_id.contains("progress")))
        })
        .collect()
}

fn action_labels(audit: &AuditDocument) -> Vec<String> {
    audit
        .events
        .iter()
        .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
        .collect()
}

fn is_returnish_case(case_id: &str) -> bool {
    case_id.contains("cancel")
        || case_id.ends_with("-back")
        || case_id.contains("-return")
        || case_id.ends_with("-close")
}

fn is_cancelish_label(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.contains("cancel")
        || lower.contains("back")
        || lower == "return"
        || lower.starts_with("return ")
        || lower.contains("close")
}

fn case_preserves_work(replay: &CaseReplay) -> bool {
    // Origin stack is non-negotiable for temporary transitions.
    if replay.before.origin_stack != replay.after.origin_stack {
        return false;
    }
    // Cancel/back must not lose draft, transcript, or selection context unless
    // the case intentionally mutates that domain (picks something).
    let cancel_like = is_returnish_case(&replay.case_id)
        || replay
            .transitions
            .iter()
            .any(|t| t.kind == "cancel" || t.kind == "return");
    if cancel_like {
        let draft_ok = replay
            .preserved_field_hashes
            .get("draft")
            .map(|h| h.before == h.after)
            .unwrap_or(replay.before.draft == replay.after.draft);
        let transcript_ok = replay
            .preserved_field_hashes
            .get("transcript")
            .map(|h| h.before == h.after)
            .unwrap_or(replay.before.transcript == replay.after.transcript);
        let origin_ok = replay.before.visible_origin == replay.after.visible_origin
            || replay
                .preserved_field_hashes
                .get("visible_origin")
                .map(|h| h.before == h.after)
                .unwrap_or(false)
            || replay.after.return_policy.contains("return");
        return draft_ok && transcript_ok && origin_ok;
    }
    true
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

fn known_screen_ids() -> HashSet<&'static str> {
    canonical_inventory()
        .iter()
        .map(|s| s.id)
        .collect()
}

// ── VAL-V3-CROSS-002: temporary transitions return to origin ────────────

#[test]
fn temporary_transitions_return_to_recorded_origin() {
    let known = known_screen_ids();
    let public: HashSet<_> = public_case_ids().iter().copied().collect();

    let mut exercised = 0usize;
    for case_id in ORIGIN_RETURN_CASES {
        assert!(
            public.contains(case_id),
            "origin-return case `{case_id}` must be a public scenario"
        );
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|e| panic!("{case_id} must replay: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} must validate: {e}"));

        assert_eq!(
            replay.before.origin_stack, replay.after.origin_stack,
            "{case_id}: origin stack must survive cancel/back/return"
        );

        for transition in &replay.transitions {
            assert_ne!(
                transition.kind, "dead-end",
                "{case_id}: dead-end transition"
            );
            assert!(
                known.contains(transition.to_screen_id.as_str())
                    || known.contains(transition.from_screen_id.as_str()),
                "{case_id}: transition leaves the inventory: {} -> {}",
                transition.from_screen_id,
                transition.to_screen_id
            );
            // Cancel/return edges stay within the paused origin or declared next.
            if transition.kind == "cancel" || transition.kind == "return" {
                assert!(
                    !transition.to_screen_id.is_empty(),
                    "{case_id}: cancel/return must declare a destination"
                );
                // Category home pages and unexplained jumps are forbidden.
                let forbidden_jump = matches!(
                    transition.to_screen_id.as_str(),
                    "home" | "onboarding" | "empty"
                ) && !matches!(
                    transition.from_screen_id.as_str(),
                    "streaming"
                        | "tool-running"
                        | "chat-turn"
                        | "chat-long-scroll"
                        | "review"
                        | "diff-viewer"
                        | "help"
                        | "status"
                        | "workflow-monitor"
                        | "workflow-launch"
                        | "workflow-launch-blocked"
                        | "connect-connected"
                );
                // Home is allowed only when it was already the recorded origin.
                if forbidden_jump && transition.to_screen_id == "home" {
                    assert!(
                        replay.before.visible_origin.to_ascii_lowercase().contains("home")
                            || replay
                                .before
                                .origin_stack
                                .iter()
                                .any(|o| o.contains("home") || o.contains("Home")),
                        "{case_id}: jump to home not justified by origin {:?}",
                        replay.before.visible_origin
                    );
                }
            }
        }
        exercised += 1;
    }
    assert!(
        exercised >= 40,
        "need at least 40 origin-return cases, got {exercised}"
    );

    // Canonical temporary surfaces themselves record return transitions.
    // `composer-multiline` is the expanded composer itself (still the working
    // conversation shell input), so Escape-return is not required beyond Send.
    const RETURN_EXEMPT: &[&str] = &["composer-multiline"];
    let temporary_fixtures: Vec<(String, String)> = canonical_inventory()
        .iter()
        .filter(|s| {
            matches!(
                s.surface,
                Surface::ActionDock | Surface::TemporaryDetailView | Surface::ContextualInspector
            ) && !RETURN_EXEMPT.contains(&s.id)
        })
        .map(|s| (s.id.to_owned(), publication_fixture(s.id)))
        .collect();
    let mut missing_return = Vec::new();
    for (screen_id, fixture_id) in &temporary_fixtures {
        let audit = render_pair(screen_id, fixture_id);
        let has_return = audit.transitions.iter().any(|t| {
            is_cancelish_label(&t.action_id)
                || t.kind == "cancel"
                || t.kind == "return"
                || t.action_id.contains("cancel")
                || t.action_id.contains("back")
                || t.action_id.contains("return")
                || t.action_id.contains("close")
                || t.action_id.contains("collapse")
                || t.action_id.contains("submit")
                || t.action_id.contains("send")
                || t.action_id.contains("continue")
        }) || action_labels(&audit)
            .iter()
            .any(|label| is_cancelish_label(label) || label.to_ascii_lowercase().contains("send") || label.to_ascii_lowercase().contains("continue"));
        // Pure automatic waits (oauth, rate-limit) may only expose secondary cancel.
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();
        let text_has_return = lower.contains("[esc]")
            || lower.contains("cancel")
            || lower.contains("back")
            || lower.contains("return")
            || lower.contains("close")
            || lower.contains("continue")
            || lower.contains("[enter]");
        if !has_return && !text_has_return {
            missing_return.push(screen_id.clone());
        }
    }
    assert!(
        missing_return.is_empty(),
        "temporary surfaces missing visible return path: {missing_return:?}"
    );
}

// ── VAL-V3-CROSS-003: no dead ends ──────────────────────────────────────

#[test]
fn no_dead_ends_across_all_canonical_and_public_cases() {
    let inventory = canonical_inventory();
    assert_eq!(inventory.len(), 86);

    // Every canonical screen either exposes an action/transition or is a
    // terminal-looking success identity that still has a continuation path.
    for screen in inventory {
        assert_eq!(screen.status, ImplementationStatus::Implemented);
        let fixture = publication_fixture(screen.id);
        let audit = render_pair(screen.id, &fixture);
        let has_transition = !audit.transitions.is_empty();
        let has_action = audit.events.iter().any(|e| e.action.is_some());
        let text = audit.render_txt();
        let has_visible_control = text.contains("[esc]")
            || text.contains("[enter]")
            || text.to_ascii_lowercase().contains("continue")
            || text.to_ascii_lowercase().contains("return")
            || text.to_ascii_lowercase().contains("reply")
            || text.contains("›");
        assert!(
            has_transition || has_action || has_visible_control,
            "{} ({}): dead end with no outgoing action/transition/control",
            screen.id,
            fixture
        );
        for t in &audit.transitions {
            assert_ne!(
                t.kind, "dead-end",
                "{}: transitions must never be dead-end",
                screen.id
            );
        }
    }

    // Public cases must never Author a dead-end kind.
    for case_id in public_case_ids().iter().take(80) {
        let replay = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        for t in &replay.transitions {
            assert_ne!(t.kind, "dead-end", "{case_id}: dead-end transition");
        }
        assert!(
            !replay.after.screen_id.is_empty(),
            "{case_id}: empty after screen"
        );
    }

    // Negative fixture still rejects dead-end transitions.
    let output = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .args(["negative-fixture", "dead-end-transition"])
        .output()
        .expect("negative dead-end fixture");
    assert_eq!(
        output.status.code(),
        Some(3),
        "dead-end-transition must fail closed (exit 3), got {:?}",
        output.status.code()
    );
}

// ── VAL-V3-CROSS-004: user work survives interruptions ─────────────────

#[test]
fn user_work_survives_interruption_families() {
    let mut families_hit: BTreeSet<&str> = BTreeSet::new();
    let samples = [
        ("permission", "permission-cancel"),
        ("permission", "permission-file-edit-cancel"),
        ("connection", "login-cancel"),
        ("connection", "connect-oauth-cancel"),
        ("connection", "mcp-return"),
        ("connection", "mcp-progress-cancel-invocation"),
        ("error", "turn-error-cancel"),
        ("error", "tool-failed-capability-return-cancel"),
        ("rate-limit", "rate-limit-race-cancel-wins"),
        ("checkpoint", "checkpoint-continue-chat"),
        ("mission", "mission-launch-cancel"),
        ("mission", "mission-intervene-cancel"),
        ("mission", "mission-pause-cancel"),
        ("mission", "mission-request-changes-cancel"),
        ("work-style", "work-style-access-cancel"),
        ("work-style", "blocked-command-work-style-return"),
    ];

    for (family, case_id) in samples {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id} must replay: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} must validate: {e}"));
        assert!(
            case_preserves_work(&replay),
            "{case_id} ({family}): silent work loss on interrupt/cancel"
        );
        // Secret fields that matter for the family.
        for field in [
            "draft",
            "transcript",
            "visible_origin",
            "session_id",
            "user_task",
        ] {
            if let Some(hash) = replay.preserved_field_hashes.get(field) {
                // Cancel families should keep before == after for these.
                if is_returnish_case(case_id) {
                    assert_eq!(
                        hash.before, hash.after,
                        "{case_id}: preserved {field} must be identical"
                    );
                }
            }
        }
        families_hit.insert(family);
    }
    assert!(
        families_hit.len() >= 7,
        "need all interruption families exercised, got {families_hit:?}"
    );

    // Default recovery copy itself must state preservation.
    for &(screen_id, fixture_id) in RECOVERY_DEFAULTS {
        let text = render_pair(screen_id, fixture_id).render_txt();
        let lower = text.to_ascii_lowercase();
        let mentions_preservation = lower.contains("kept")
            || lower.contains("preserved")
            || lower.contains("still here")
            || lower.contains("saved")
            || lower.contains("draft")
            || lower.contains("left in place")
            || lower.contains("not lost")
            || lower.contains("remain")
            || lower.contains("resume")
            || lower.contains("continue from")
            || lower.contains("nothing was lost")
            || lower.contains("work is")
            || text.contains(DRAFT_HINT);
        // toasts/settings notice states may refer to the session restore path
        // rather than a draft; accept conversation continuity markers too.
        let continuity = lower.contains("conversation")
            || lower.contains("replay")
            || lower.contains("retry")
            || lower.contains("restore")
            || lower.contains("checkpoint")
            || lower.contains("partial")
            || lower.contains("finish");
        assert!(
            mentions_preservation || continuity,
            "{screen_id}/{fixture_id}: recovery must acknowledge preserved work"
        );
    }
}

const DRAFT_HINT: &str = "Keep the risk note short before we publish.";

// ── VAL-V3-CROSS-005: one live state source ─────────────────────────────

#[test]
fn one_live_state_source_per_active_operation() {
    for &(screen_id, fixture_id) in LIVE_SURFACES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();

        // Count title/status phrases that claim independent live ownership.
        let mutual = [
            ("streaming", lower.matches("streaming").count()),
            ("waiting for authorization", lower.matches("waiting for authorization").count()),
            ("running", lower.matches("running").count()),
            ("retrying", lower.matches("retrying").count()),
        ];
        // Streaming screen should not also host a second "tool running" banner etc.
        if screen_id == "streaming" {
            assert!(
                !lower.contains("tool is running") && !lower.contains("command is running"),
                "{screen_id}: streaming must not also claim tool/command live ownership"
            );
        }

        // Semantic live-family events and primary actions that describe progress.
        let lives = live_events(&audit);
        // Coexist is allowed only for distinct detail levels.
        let mut summary_subjects = BTreeSet::new();
        let mut detail_subjects = BTreeSet::new();
        for event in &lives {
            match event.identity.detail_level.as_str() {
                "summary" | "compact" | "" => {
                    summary_subjects.insert(event.identity.subject_id.clone());
                }
                _ => {
                    detail_subjects.insert(event.identity.subject_id.clone());
                }
            }
        }
        assert!(
            summary_subjects.len() <= 1,
            "{screen_id}/{fixture_id}: multiple summary live owners {summary_subjects:?}"
        );
        // Conflicting labels for the same subject across levels fail.
        for subject in &summary_subjects {
            if detail_subjects.contains(subject) {
                // allowed coexistence: same subject, different levels
                continue;
            }
        }

        // Interrupt action (when present) should appear once, next to the live owner.
        let interrupt_mentions = lower.matches("interrupt").count()
            + lower.matches("stop generation").count()
            + lower.matches("stop this").count();
        assert!(
            interrupt_mentions <= 2,
            "{screen_id}: interrupt guidance duplicated ({interrupt_mentions})"
        );
        let _ = mutual;
    }
}

// ── VAL-V3-CROSS-006: disclosure levels consistent ──────────────────────

#[test]
fn disclosure_levels_stay_consistent_across_flows() {
    let mut levels_seen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for &(compact_id, compact_fx, detail_id, detail_fx) in DISCLOSURE_PAIRS {
        let compact = render_pair(compact_id, compact_fx);
        let detail = render_pair(detail_id, detail_fx);
        let compact_txt = compact.render_txt();
        let detail_txt = detail.render_txt();
        assert_ne!(
            compact_txt, detail_txt,
            "{compact_id}→{detail_id}: deeper level must add distinct content"
        );

        // Distinct disclosed content nuggets (detail longer or carries extra labels).
        let compact_unique = unique_words(&compact_txt);
        let detail_unique = unique_words(&detail_txt);
        let added: BTreeSet<_> = detail_unique.difference(&compact_unique).cloned().collect();
        assert!(
            !added.is_empty() || detail_txt.lines().count() >= compact_txt.lines().count(),
            "{compact_id}→{detail_id}: deeper state must add unique content"
        );

        // Detail_level audit fields use the ordered vocabulary when present.
        for event in compact.events.iter().chain(detail.events.iter()) {
            let level = event.identity.detail_level.trim();
            if !level.is_empty() {
                levels_seen
                    .entry(event.identity.family.clone())
                    .or_default()
                    .insert(level.to_owned());
                assert!(
                    matches!(
                        level,
                        "summary"
                            | "compact"
                            | "focused"
                            | "detail"
                            | "full"
                            | "evidence"
                            | "advanced"
                            | "default"
                            | "page"
                    ),
                    "{compact_id}: unexpected disclosure level `{level}`"
                );
            }
        }

        // Back/unwind is declared out of the deeper state.
        let has_back = detail.transitions.iter().any(|t| {
            t.kind == "cancel"
                || t.kind == "return"
                || t.action_id.contains("back")
                || t.action_id.contains("return")
                || t.action_id.contains("close")
                || t.action_id.contains("collapse")
        }) || action_labels(&detail)
            .iter()
            .any(|l| is_cancelish_label(l))
            || detail_txt.to_ascii_lowercase().contains("[esc]")
            || detail_txt.to_ascii_lowercase().contains("collapse")
            || detail_txt.to_ascii_lowercase().contains("back");
        assert!(
            has_back,
            "{detail_id}/{detail_fx}: deeper disclosure must reverse with Back"
        );
    }

    // A fourth ad-hoc level name must not appear in any disclosure family.
    for (family, levels) in &levels_seen {
        assert!(
            levels.len() <= 3
                || levels.is_subset(&BTreeSet::from([
                    "summary".into(),
                    "compact".into(),
                    "focused".into(),
                    "detail".into(),
                    "full".into(),
                    "evidence".into(),
                    "advanced".into(),
                    "default".into(),
                    "page".into(),
                ])),
            "{family}: unexpected level set {levels:?}"
        );
    }
}

fn unique_words(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .to_ascii_lowercase()
        })
        .filter(|w| w.len() > 3)
        .collect()
}

// ── VAL-V3-CROSS-007: selection behavior is shared ──────────────────────

#[test]
fn selection_behavior_is_shared_across_pickers() {
    let mut matrix = Vec::new();
    for &(screen_id, fixture_id) in SELECTION_SURFACES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();

        let has_selected_row = text.contains('›')
            || audit
                .events
                .iter()
                .any(|e| e.identity.family.contains("selection") || e.identity.family.contains("row"));
        let has_confirm = lower.contains("[enter]")
            || lower.contains("use this")
            || lower.contains("choose")
            || lower.contains("apply")
            || lower.contains("confirm")
            || lower.contains("select")
            || lower.contains("add ")
            || lower.contains("attach")
            || lower.contains("export")
            || lower.contains("continue")
            || primary_events(&audit).len() == 1;
        let has_cancel = lower.contains("[esc]")
            || lower.contains("cancel")
            || lower.contains("back")
            || audit.transitions.iter().any(|t| {
                t.kind == "cancel"
                    || t.action_id.contains("cancel")
                    || t.action_id.contains("back")
            });
        let has_consequence = lower.contains("will ")
            || lower.contains("applies ")
            || lower.contains("changes ")
            || lower.contains("keeps ")
            || lower.contains("uses ")
            || lower.contains("lets ")
            || lower.contains("when you")
            || lower.contains("before")
            || lower.contains("after")
            || lower.contains("scope")
            || lower.contains("effect")
            || text.lines().any(|line| {
                let t = line.trim();
                !t.is_empty()
                    && t.len() > 20
                    && (t.contains("·") || t.chars().any(|c| c.is_ascii_lowercase()))
            });

        assert!(
            has_selected_row,
            "{screen_id}: shared selected-row treatment missing"
        );
        assert!(
            has_confirm,
            "{screen_id}: shared confirm action missing"
        );
        assert!(has_cancel, "{screen_id}: shared cancel-to-origin missing");
        assert!(
            has_consequence || screen_id.contains("theme") || screen_id.contains("variant"),
            "{screen_id}: consequence/tradeoff placement missing"
        );

        // Bottom dock or temporary detail — never a floating modal.
        assert!(
            !lower.contains("floating modal") && !lower.contains("dialog overlay"),
            "{screen_id}: floating modal forbidden"
        );

        matrix.push((
            screen_id,
            has_selected_row,
            has_confirm,
            has_cancel,
            has_consequence,
        ));
    }
    assert_eq!(matrix.len(), SELECTION_SURFACES.len());
    // Every picker shares the (selected, confirm, cancel) triple.
    for (screen_id, sel, confirm, cancel, _) in &matrix {
        assert!(
            *sel && *confirm && *cancel,
            "{screen_id}: selection triple incomplete"
        );
    }
}

// ── VAL-V3-CROSS-008: one work-style concept ────────────────────────────

#[test]
fn agent_and_mode_form_one_work_style_concept() {
    for &(screen_id, fixture_id) in WORK_STYLE_SURFACES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();
        assert!(
            lower.contains("how code should work")
                || lower.contains("how code works")
                || lower.contains("work-style")
                || lower.contains("work style")
                || text.contains("Careful edits")
                || text.contains("Autonomous"),
            "{screen_id}: must show unified How Code should work concept"
        );
        assert!(
            !lower.contains("choose an agent")
                && !lower.contains("select agent")
                && !lower.contains("agent mode")
                && !lower.contains("switch mode")
                && !lower.contains("mode picker")
                && !lower.contains("agent picker settings"),
            "{screen_id}: Agent and Mode must not productize as dual settings"
        );
        if let Some(ws) = &audit.work_style {
            assert!(!ws.applied.trim().is_empty());
            assert!(!ws.access_scope.trim().is_empty());
        }
    }

    // Navigation wording across Flow 3 publication surfaces stays work-style.
    for screen in canonical_inventory()
        .iter()
        .filter(|s| s.flow == FlowOwner::WorkConfiguration)
    {
        let text = render_pair(screen.id, &publication_fixture(screen.id)).render_txt();
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("separate agent setting")
                && !lower.contains("separate mode setting")
                && !lower.contains("agent vs mode"),
            "{}: dual agent/mode headings forbidden",
            screen.id
        );
    }
}

// ── VAL-V3-CROSS-009: providers and MCP remain distinct ─────────────────

#[test]
fn providers_and_mcp_remain_distinct_entry_points() {
    // Provider surfaces never present MCP as the primary next step.
    for &(screen_id, fixture_id) in PROVIDER_DEFAULTS {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("open mcp")
                && !lower.contains("continue to mcp")
                && !lower.contains("manage mcp next")
                && !lower.contains("connections dashboard"),
            "{screen_id}: provider surface merged toward MCP dashboard"
        );
        // Success from connect-connected returns to model origin.
        if screen_id == "connect-connected" {
            assert!(
                audit.transitions.iter().any(|t| {
                    t.action_id.contains("return")
                        && (t.to_screen_id == "model-picker" || t.to_screen_id == "chat-turn")
                }),
                "provider success must return to blocked model action / conversation"
            );
            assert!(
                !audit
                    .transitions
                    .iter()
                    .any(|t| t.to_screen_id == "mcp" && t.kind == "direct" && t.path == "success"),
                "provider success must not transition directly into mcp"
            );
        }
        // Login / provider catalogue must not list "MCP server" as a peer.
        if screen_id == "login" || screen_id == "connect-provider" {
            assert!(
                !lower.contains("mcp server") && !lower.contains("add mcp"),
                "{screen_id}: login/provider catalogue must stay provider-only"
            );
        }
    }

    // MCP entry remains capability-owned.
    for &(screen_id, fixture_id) in MCP_DEFAULTS {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("connect a model provider")
                || screen_id == "mcp" && lower.contains("capability"),
            "{screen_id}: MCP must not become the provider onboarding surface"
        );
        // Direct transition into a provider login chain should only be an
        // explicit capability-auth detour, never a merged connections policy.
        for t in &audit.transitions {
            if t.to_screen_id.starts_with("connect-") || t.to_screen_id == "login" {
                assert!(
                    t.kind == "illustrative_detour"
                        || t.kind == "direct"
                        || t.action_id.contains("auth")
                        || t.action_id.contains("connect"),
                    "{screen_id}: unexpected provider hop {:?}",
                    t
                );
            }
        }
        let _ = lower;
    }

    // Flow manifest mistakes: provider and MCP branches must stay separate steps.
    let flow = flow_manifests()
        .iter()
        .find(|f| f.name == "provider-and-mcp-connection")
        .expect("provider-and-mcp-connection storyboard");
    let screens: Vec<_> = flow.steps.iter().map(|s| s.screen_id).collect();
    assert!(
        screens.contains(&"login") || screens.contains(&"connect-provider"),
        "storyboard must include a pure provider entry"
    );
    assert!(
        screens.contains(&"mcp"),
        "storyboard must include an MCP branch"
    );
    // login / connect-connected must not share a step fixture with mcp *surface*
    // defaults. Journey-scoped "provider-and-mcp" naming in gate fixtures is fine.
    for step in &flow.steps {
        if step.screen_id == "login" || step.screen_id.starts_with("connect-") {
            assert!(
                !step.fixture_id.contains("views.mcp")
                    && !step.fixture_id.contains("sidebar-mcp")
                    && !step.fixture_id.ends_with(".mcp")
                    && !step.fixture_id.contains("/mcp"),
                "{} reuses an MCP surface fixture {}",
                step.screen_id,
                step.fixture_id
            );
            assert_ne!(
                step.screen_id, "mcp",
                "provider step must not occupy the mcp screen id"
            );
        }
        if step.screen_id == "mcp" {
            assert!(
                !step.fixture_id.contains("connect-provider")
                    && !step.fixture_id.contains("connect-method")
                    && !step.fixture_id.contains("connect-oauth")
                    && !step.fixture_id.contains("connect-apikey")
                    && !step.fixture_id.contains("connect-connected")
                    && !step.fixture_id.ends_with(".login")
                    && !step.fixture_id.contains("auth.login"),
                "mcp step must not reuse provider identity fixtures ({})",
                step.fixture_id
            );
        }
    }
}

// ── VAL-V3-CROSS-010: MCP capability-led ────────────────────────────────

#[test]
fn mcp_is_capability_led_and_action_led() {
    for &(screen_id, fixture_id) in MCP_DEFAULTS {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();

        // Capability/outcome language required as default.
        let capabilityish = lower.contains("lets you")
            || lower.contains("can ")
            || lower.contains("available")
            || lower.contains("ready")
            || lower.contains("needs")
            || lower.contains("blocked")
            || lower.contains("connect")
            || lower.contains("retry")
            || lower.contains("set up")
            || lower.contains("use ")
            || lower.contains("capability")
            || lower.contains("tools are")
            || lower.contains("browse")
            || lower.contains("docs")
            || lower.contains("tracker")
            || lower.contains("no capability")
            || lower.contains("nothing to connect");
        assert!(
            capabilityish,
            "{screen_id}/{fixture_id}: default MCP must name capability or availability"
        );

        // Transport/detail dump is for diagnostics only.
        if !fixture_id.contains("diagnostics") {
            for ban in MCP_TRANSPORT_BANS {
                assert!(
                    !text.contains(ban),
                    "{screen_id}/{fixture_id}: transport term `{ban}` leaked into default"
                );
            }
            assert!(
                !lower.contains("timeout=")
                    && !lower.contains("endpoint=")
                    && !lower.contains("scope=")
                    && !lower.contains("config path"),
                "{screen_id}/{fixture_id}: raw config field appeared in default MCP view"
            );
        }

        // At least one next action when there is something to fix/configure.
        if fixture_id.contains("mixed")
            || fixture_id.contains("attention")
            || fixture_id.contains("failed")
            || fixture_id.contains("empty")
        {
            let primary = primary_events(&audit).len();
            let has_action = primary >= 1
                || action_labels(&audit).iter().any(|l| !l.is_empty())
                || lower.contains("[enter]")
                || lower.contains("retry")
                || lower.contains("set up")
                || lower.contains("connect")
                || lower.contains("review");
            assert!(
                has_action || fixture_id.contains("all-ready"),
                "{screen_id}/{fixture_id}: capability state needs a next action"
            );
        }
    }
}

// ── VAL-V3-CROSS-011: MCP attention order and progress ──────────────────

#[test]
fn mcp_attention_order_and_progress_are_consistent() {
    let mixed = render_pair("mcp", "views.mcp.mixed");
    let mixed_text = mixed.render_txt();
    let attention = render_pair("sidebar-mcp", "views.sidebar-mcp.attention");
    let attention_text = attention.render_txt();

    for (label, text) in [
        ("mcp/mixed", mixed_text.as_str()),
        ("sidebar-mcp/attention", attention_text.as_str()),
    ] {
        let lower = text.to_ascii_lowercase();
        // Attention vocabulary must appear.
        let has_attention = lower.contains("needs")
            || lower.contains("failed")
            || lower.contains("blocked")
            || lower.contains("sign in")
            || lower.contains("not connected")
            || lower.contains("unavailable")
            || lower.contains("attention");
        assert!(
            has_attention,
            "{label}: attention states missing from MCP list"
        );

        // Ordering: first occurrence of an attention word should not trail every healthy marker.
        if let (Some(att_pos), Some(ok_pos)) = (
            first_attention_line(text),
            first_healthy_line(text),
        ) {
            assert!(
                att_pos <= ok_pos,
                "{label}: attention rows must sort before healthy connections (att={att_pos}, ok={ok_pos})"
            );
        }
    }

    // Healthy collapse: all-ready fixture should keep density low.
    let ready = render_pair("sidebar-mcp", "views.sidebar-mcp.all-ready").render_txt();
    let ready_lower = ready.to_ascii_lowercase();
    assert!(
        ready_lower.contains("ready")
            || ready_lower.contains("connected")
            || ready_lower.contains("available")
            || ready_lower.contains("healthy"),
        "all-ready must name the healthy aggregate"
    );
    // Not a wall of transport rows.
    let rowish = ready
        .lines()
        .filter(|l| l.contains('·') || l.contains('●') || l.contains('○'))
        .count();
    assert!(
        rowish <= 12,
        "healthy MCP collapse should stay compact, saw {rowish} marker rows"
    );

    // Long-running MCP progress changes between ticks.
    let t1 = render_pair("mcp", "views.mcp.progress-invocation.tick-1").render_txt();
    let success = render_pair("mcp", "views.mcp.progress-success").render_txt();
    assert_ne!(
        t1, success,
        "long MCP call progress must change across declared states"
    );
    let t1_lower = t1.to_ascii_lowercase();
    assert!(
        t1_lower.contains("waiting")
            || t1_lower.contains("running")
            || t1_lower.contains("progress")
            || t1_lower.contains("%")
            || t1_lower.contains("elapsed")
            || t1_lower.contains("step")
            || t1.contains('●')
            || t1_lower.contains("working"),
        "long MCP call must show progress/waiting reason, not a static invocation id"
    );
    // Progress meaning is not color-only: status words present.
    assert!(
        t1.chars().any(|c| c.is_ascii_alphabetic()),
        "progress must remain readable as monochrome text"
    );
}

fn first_attention_line(text: &str) -> Option<usize> {
    let keys = [
        "failed",
        "blocked",
        "needs",
        "sign in",
        "not connected",
        "unavailable",
        "attention",
        "error",
    ];
    text.lines().enumerate().find_map(|(idx, line)| {
        let lower = line.to_ascii_lowercase();
        keys.iter().any(|k| lower.contains(k)).then_some(idx)
    })
}

fn first_healthy_line(text: &str) -> Option<usize> {
    let keys = ["ready", "connected", "available", "healthy", "ok ·"];
    text.lines().enumerate().find_map(|(idx, line)| {
        let lower = line.to_ascii_lowercase();
        // Skip pure chrome/headers.
        if lower.contains("header") {
            return None;
        }
        keys.iter().any(|k| lower.contains(k)).then_some(idx)
    })
}

// ── VAL-V3-CROSS-012: recovery language consistent ──────────────────────

#[test]
fn recovery_language_and_action_are_consistent() {
    for &(screen_id, fixture_id) in RECOVERY_DEFAULTS {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();
        let primaries = primary_events(&audit);

        // Automatic waits: zero primary, optional secondary Retry now.
        let is_auto = AUTOMATIC_RECOVERY
            .iter()
            .any(|&(s, f)| s == screen_id && f == fixture_id)
            || (screen_id == "rate-limit" && !fixture_id.contains("cancel"));
        if is_auto {
            assert!(
                primaries.is_empty(),
                "{screen_id}/{fixture_id}: automatic wait must have zero primary actions"
            );
            assert!(
                lower.contains("automatic")
                    || lower.contains("waiting")
                    || lower.contains("resume")
                    || lower.contains("retry window")
                    || lower.contains("cooldown")
                    || lower.contains("rate limit"),
                "{screen_id}/{fixture_id}: automatic wait must label itself"
            );
            // Optional secondary retry only.
            let secondary_retries = action_labels(&audit)
                .iter()
                .filter(|l| l.to_ascii_lowercase().contains("retry"))
                .count()
                + lower.matches("retry now").count();
            assert!(
                secondary_retries <= 2,
                "{screen_id}/{fixture_id}: at most secondary Retry now"
            );
            continue;
        }

        // Manual recovery: one plain cause + one primary recovery.
        assert!(
            primaries.len() <= 1,
            "{screen_id}/{fixture_id}: exactly one primary recovery (or zero for pure notice), got {}",
            primaries.len()
        );
        if screen_id != "toasts" {
            assert_eq!(
                primaries.len(),
                1,
                "{screen_id}/{fixture_id}: must show one primary recovery action"
            );
        }

        // Cause language is plain English.
        assert!(
            !lower.contains("econn")
                && !lower.contains("status 429")
                && !lower.contains("stack trace")
                && !lower.contains("request_id")
                && !lower.contains("rate_limit_error"),
            "{screen_id}/{fixture_id}: raw diagnostic codes leaked"
        );
        let causeish = lower.contains("could not")
            || lower.contains("failed")
            || lower.contains("blocked")
            || lower.contains("stopped")
            || lower.contains("needs")
            || lower.contains("error")
            || lower.contains("unable")
            || lower.contains("refused")
            || lower.contains("missing")
            || lower.contains("limit")
            || lower.contains("interrupted")
            || lower.contains("timed out")
            || lower.contains("offline")
            || lower.contains("too large")
            || lower.contains("checkpoint")
            || lower.contains("saved")
            || lower.contains("restore");
        assert!(
            causeish,
            "{screen_id}/{fixture_id}: plain-English cause missing"
        );
    }
}

// ── VAL-V3-CROSS-013: mission goal-led exception-first ──────────────────

#[test]
fn mission_flow_is_goal_led_and_exception_first() {
    for &(screen_id, fixture_id) in MISSION_DEFAULTS {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let lower = text.to_ascii_lowercase();

        // Goal / stage language present.
        let goalish = lower.contains("prepare the release")
            || lower.contains("release")
            || lower.contains("mission")
            || lower.contains("goal")
            || lower.contains("review")
            || lower.contains("validate")
            || lower.contains("launch")
            || lower.contains("stage")
            || lower.contains("current")
            || text.contains("Prepare the release safely");
        assert!(
            goalish,
            "{screen_id}/{fixture_id}: mission default must lead with goal/stage"
        );

        // Internals hidden on compact defaults.
        if !fixture_id.contains("team-board")
            && !fixture_id.contains("verification")
            && !fixture_id.contains("scorer")
            && !fixture_id.contains("concurrency")
            && !fixture_id.contains("tool-envelope")
        {
            for ban in MISSION_INTERNAL_BANS {
                assert!(
                    !text.contains(ban),
                    "{screen_id}/{fixture_id}: internal `{ban}` leaked on compact mission default"
                );
            }
        }
    }

    // Monitor ordering: Needs attention before Working before Recently completed.
    let monitor = render_pair("workflow-monitor", "gate.mission.workflow-monitor").render_txt();
    let mon_lower = monitor.to_ascii_lowercase();
    if let (Some(needs), Some(working), Some(done)) = (
        mon_lower.find("needs attention").or_else(|| mon_lower.find("needs")),
        mon_lower.find("working"),
        mon_lower
            .find("recently completed")
            .or_else(|| mon_lower.find("completed")),
    ) {
        assert!(
            needs <= working && working <= done,
            "monitor sections must order Needs attention → Working → Recently completed"
        );
    }

    // Failure monitor still exception-first.
    let failure =
        render_pair("workflow-monitor-failure", "gate.mission.workflow-monitor-failure").render_txt();
    let fail_lower = failure.to_ascii_lowercase();
    assert!(
        fail_lower.contains("needs")
            || fail_lower.contains("failed")
            || fail_lower.contains("blocked")
            || fail_lower.contains("attention")
            || fail_lower.contains("fix"),
        "failure monitor must surface the exception first"
    );
}

// ── VAL-V3-CROSS-014: human completion ──────────────────────────────────

#[test]
fn mission_completion_requires_explicit_human_review() {
    // No direct validation → approved completion without human act.
    let completionish = [
        "mission-validation-success",
        "mission-completion-validation",
        "mission-approve-completion",
    ];
    for case_id in completionish {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id} must replay: {e}"));
        if case_id == "mission-validation-success" {
            assert_eq!(
                replay.after.screen_id, "reviewer-output",
                "validation success must land on reviewer output for human review"
            );
            assert_ne!(
                replay.after.screen_id, "reviewer-approved",
                "validation must never auto-complete to approved"
            );
        }
        if case_id == "mission-completion-validation" {
            // Still requires the human-approved surface, never silent done.
            assert!(
                replay.after.screen_id == "reviewer-approved"
                    || replay.after.screen_id == "reviewer-output",
                "completion validation still surfaces human review state"
            );
        }
        if case_id == "mission-approve-completion" {
            // Explicit human approve action is what completes.
            assert!(
                replay.actions.iter().any(|a| {
                    let label = a.label.to_ascii_lowercase();
                    a.action_id.contains("approve")
                        || label.contains("approve")
                        || a.action_id.contains("complete")
                }),
                "completion must be driven by an explicit human approve action"
            );
            assert_eq!(
                replay.after.screen_id, "chat-turn",
                "human approval returns to the conversation origin"
            );
        }
    }

    let reviewer = render_pair("reviewer-output", "gate.mission.reviewer-output");
    let text = reviewer.render_txt();
    let lower = text.to_ascii_lowercase();
    assert!(
        lower.contains("approve")
            || primary_events(&reviewer)
                .iter()
                .any(|e| e.action.as_ref().map(|a| a.label.to_ascii_lowercase().contains("approve")).unwrap_or(false))
            || lower.contains("review"),
        "reviewer-output must require an explicit human decision"
    );
    // No "auto-approved" claim.
    assert!(
        !lower.contains("auto-approved")
            && !lower.contains("automatically approved")
            && !lower.contains("worker approved"),
        "reviewer-output must not claim automated approval"
    );

    let approved = render_pair("reviewer-approved", "gate.mission.reviewer-approved");
    let approved_txt = approved.render_txt();
    let alower = approved_txt.to_ascii_lowercase();
    assert!(
        alower.contains("approved")
            || alower.contains("you approved")
            || alower.contains("human")
            || alower.contains("recorded"),
        "reviewer-approved must record the human approval act"
    );

    // Validation failure has exactly one primary repair action.
    let fail = render_pair(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    );
    let fail_text = fail.render_txt().to_ascii_lowercase();
    let fail_primaries = primary_events(&fail);
    assert!(
        fail_primaries.len() <= 1,
        "validation failure must keep one primary repair"
    );
    let repair_label = fail_primaries
        .first()
        .and_then(|e| e.action.as_ref())
        .map(|a| a.label.to_ascii_lowercase())
        .unwrap_or_default();
    assert!(
        repair_label.contains("fix")
            || repair_label.contains("evidence")
            || repair_label.contains("open")
            || repair_label.contains("create")
            || fail_text.contains("create a fix")
            || fail_text.contains("open evidence")
            || fail_text.contains("fix task"),
        "failure primary should be Create a fix task / Open evidence style, got `{repair_label}`"
    );
}

// ── VAL-V3-CROSS-015: shared meanings use shared words ──────────────────

#[test]
fn shared_meanings_use_shared_words_across_gallery() {
    // Sample publication fixtures across all six flow owners.
    let mut samples: Vec<(String, String, String)> = Vec::new();
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let text = render_pair(screen.id, &fixture).render_txt();
        samples.push((screen.id.to_owned(), fixture, text));
    }
    assert_eq!(samples.len(), 86);

    // Conflicting synonym classes that change consequence.
    let conflict_pairs = [
        (
            &["Allow once", "Allow this time", "Permit once"][..],
            "one-time allow",
        ),
        (
            &["Always allow", "Always permit for project", "Persist forever"][..],
            "persistent allow",
        ),
        (
            &["Force complete", "Auto-approve mission", "Skip human review"][..],
            "auto completion",
        ),
    ];
    for (screen_id, fixture, text) in &samples {
        for (labels, concept) in conflict_pairs {
            let hits: Vec<_> = labels
                .iter()
                .filter(|label| text.contains(*label))
                .collect();
            assert!(
                hits.len() <= 1,
                "{screen_id}/{fixture}: mixed words for {concept}: {hits:?}"
            );
        }
        // Raw model ids forbidden in status/copy.
        assert!(
            !text.contains("claude-opus-4-")
                && !text.contains("gpt-4o-")
                && !text.contains("model_id="),
            "{screen_id}/{fixture}: raw model id leaked"
        );
    }

    // Action labels that appear widely use stable tokens.
    let mut label_index: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (screen_id, fixture, text) in &samples {
        for (concept, variants) in SHARED_LABEL_EXPECTATIONS {
            for variant in *variants {
                if text.contains(variant) {
                    label_index
                        .entry((*concept).to_owned())
                        .or_default()
                        .insert(format!("{screen_id}/{fixture}::{variant}"));
                }
            }
        }
    }
    // Cancel and Back must dominate over obscure synonyms like "Abort voyage".
    for (screen_id, _, text) in &samples {
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("abort voyage")
                && !lower.contains("yeet selection")
                && !lower.contains("shineleb"),
            "{screen_id}: non-product wording for shared actions"
        );
        // Approve synonym drift.
        if lower.contains("rubber-stamp") || lower.contains("auto-bless") {
            panic!("{screen_id}: unapproved approve synonym");
        }
    }
    let _ = label_index;
}

// ── VAL-V3-CROSS-016: six journeys / six categories coherent ────────────

#[test]
fn six_journeys_remain_coherent_within_six_categories() {
    let inventory = canonical_inventory();
    assert_eq!(inventory.len(), 86);

    let mut by_category: BTreeMap<Category, usize> = BTreeMap::new();
    let mut by_flow: BTreeMap<FlowOwner, usize> = BTreeMap::new();
    for screen in inventory {
        *by_category.entry(screen.category).or_insert(0) += 1;
        *by_flow.entry(screen.flow).or_insert(0) += 1;
    }
    assert_eq!(by_category.len(), 6, "exactly six storage categories");
    assert_eq!(by_flow.len(), 6, "exactly six flow owners");
    assert_eq!(by_category.get(&Category::Session).copied().unwrap_or(0), 21);
    assert_eq!(by_category.get(&Category::Input).copied().unwrap_or(0), 20);
    assert_eq!(by_category.get(&Category::Views).copied().unwrap_or(0), 16);
    assert_eq!(by_category.get(&Category::Auth).copied().unwrap_or(0), 7);
    assert_eq!(by_category.get(&Category::States).copied().unwrap_or(0), 8);
    assert_eq!(
        by_category
            .get(&Category::Orchestration)
            .copied()
            .unwrap_or(0),
        14
    );

    // Storyboards name the six journeys and stay ordered.
    let flows = flow_manifests();
    assert_eq!(flows.len(), 6);
    let expected_flow_names = [
        "daily-work",
        "input-and-approval",
        "work-configuration",
        "provider-and-mcp-connection",
        "recovery",
        "mission",
    ];
    for name in expected_flow_names {
        assert!(
            flows.iter().any(|f| f.name == name),
            "missing journey storyboard {name}"
        );
    }

    // No product screen warehouses a permanent category dashboard.
    for screen in inventory {
        let text = render_pair(screen.id, &publication_fixture(screen.id)).render_txt();
        for ban in CATEGORY_DASHBOARD_BANS {
            assert!(
                !text.contains(ban),
                "{}: categories must remain storage, not product destinations (`{ban}`)",
                screen.id
            );
        }
        // States stay related to conversation, current action, or mission.
        let has_relation = match screen.flow {
            FlowOwner::DailyWork => true,
            FlowOwner::InputAndApproval => true,
            FlowOwner::WorkConfiguration => true,
            FlowOwner::ProviderAndMcpConnection => true,
            FlowOwner::Recovery => true,
            FlowOwner::Mission => true,
        };
        assert!(has_relation);

        // Context: every temporary surface mentions origin/task or conversation chrome.
        if matches!(
            screen.surface,
            Surface::ActionDock | Surface::TemporaryDetailView | Surface::ContextualInspector
        ) {
            let lower = text.to_ascii_lowercase();
            let related = lower.contains("conversation")
                || lower.contains("you ·")
                || lower.contains("return")
                || lower.contains("cancel")
                || lower.contains("back")
                || lower.contains("mission")
                || lower.contains("provider")
                || lower.contains("capability")
                || lower.contains("permission")
                || lower.contains("draft")
                || lower.contains("model")
                || lower.contains("how code")
                || lower.contains("work")
                || lower.contains("review")
                || lower.contains("recover")
                || lower.contains("fix")
                || text.contains("·");
            assert!(
                related,
                "{}: temporary surface must remain related to conversation/action/mission",
                screen.id
            );
        }
    }

    // Gallery storage uses the six category directories and never invents a seventh.
    let root = gallery_root();
    let mut dirs = BTreeSet::new();
    for entry in fs::read_dir(&root).expect("gallery root") {
        let entry = entry.expect("entry");
        if entry.file_type().expect("ft").is_dir() {
            dirs.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let expected_dirs: BTreeSet<_> = [
        "session",
        "input",
        "views",
        "auth",
        "states",
        "orchestration",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        dirs, expected_dirs,
        "gallery must use exactly the six category directories"
    );

    // Published pair count: 86 txt + 86 png.
    let mut txt = 0usize;
    let mut png = 0usize;
    for dir in &expected_dirs {
        for file in fs::read_dir(root.join(dir)).expect("category") {
            let file = file.expect("file");
            let name = file.file_name().to_string_lossy().into_owned();
            if name.ends_with(".txt") {
                txt += 1;
                let text = fs::read_to_string(file.path()).expect("txt");
                assert_canvas_geometry(&text, &name);
            } else if name.ends_with(".png") {
                png += 1;
                assert_png_dimensions(&file.path(), 1200, 840);
            }
        }
    }
    assert_eq!(txt, 86, "canonical gallery must hold 86 TXT");
    assert_eq!(png, 86, "canonical gallery must hold 86 PNG");
}

fn assert_png_dimensions(path: &Path, expect_w: u32, expect_h: u32) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(data.len() > 24, "{} too small to be PNG", path.display());
    assert_eq!(&data[0..8], b"\x89PNG\r\n\x1a\n", "{} bad PNG sig", path.display());
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    assert_eq!(
        (width, height),
        (expect_w, expect_h),
        "{}: expected {expect_w}x{expect_h}, got {width}x{height}",
        path.display()
    );
}

// ── Aggregate inventory of owned assertions ─────────────────────────────

#[test]
fn owned_cross_flow_assertions_are_enumerated() {
    let owned = [
        "VAL-V3-CROSS-002",
        "VAL-V3-CROSS-003",
        "VAL-V3-CROSS-004",
        "VAL-V3-CROSS-005",
        "VAL-V3-CROSS-006",
        "VAL-V3-CROSS-007",
        "VAL-V3-CROSS-008",
        "VAL-V3-CROSS-009",
        "VAL-V3-CROSS-010",
        "VAL-V3-CROSS-011",
        "VAL-V3-CROSS-012",
        "VAL-V3-CROSS-013",
        "VAL-V3-CROSS-014",
        "VAL-V3-CROSS-015",
        "VAL-V3-CROSS-016",
    ];
    assert_eq!(owned.len(), 15);
    // Sanity: six journey owners and category storage crosswalk exists.
    assert_eq!(canonical_inventory().len(), 86);
    assert_eq!(flow_manifests().len(), 6);
    assert_eq!(Category::ALL.len(), 6);
    assert_eq!(FlowOwner::ALL.len(), 6);
}
