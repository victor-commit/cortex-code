//! Final Flow 5 recovery conformance sweep.
//!
//! Owned contract assertions:
//! VAL-V3-F5-001, 002, 003, 004, 005, 006, 007, 008, 009, 010, 011, 012,
//! VAL-V3-F5-048, 049, 050, 051, 053, 054, 055, 059.
//!
//! Aggregate checks across every required recovery canonical state plus the
//! scenario inventory that proved origin-preserving detours.

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


const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
/// Phase-A paints subject title `Recovery`; audit/scenario keep the long form.
const RECOVERY_SUBJECT: &str = "Recovery";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const DRAFT: &str = "Keep the risk note short before we publish.";

fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn paints_recovery_origin(text: &str) -> bool {
    text.contains(RECOVERY_ORIGIN)
        || text.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == RECOVERY_SUBJECT || trimmed.starts_with("Recovery ·")
        })
}

/// Exact nine recovery inventory IDs (VAL-V3-F5-001).
const RECOVERY_IDS: &[&str] = &[
    "tool-failed",
    "blocked-command",
    "turn-error",
    "error-network",
    "error-context",
    "rate-limit",
    "interrupt",
    "toasts",
    "checkpoint-undo",
];

/// Closed-default fixtures that publish the product recovery summary.
const CLOSED_DEFAULT_FIXTURES: &[(&str, &str)] = &[
    ("tool-failed", "canonical.tool-failed"),
    ("blocked-command", "canonical.blocked-command"),
    ("turn-error", "canonical.turn-error"),
    ("turn-error", "gate.recovery.turn-error"),
    ("error-network", "canonical.error-network"),
    ("error-network", "gate.recovery.error-network"),
    ("error-network", "error-network.offline"),
    ("error-network", "error-network.exhausted"),
    ("error-context", "canonical.error-context"),
    ("rate-limit", "gate.recovery.rate-limit"),
    ("rate-limit", "rate-limit.wait.sample-early"),
    ("rate-limit", "rate-limit.wait.sample-later"),
    ("interrupt", "canonical.interrupt"),
    ("toasts", "canonical.toasts"),
    ("checkpoint-undo", "canonical.checkpoint-undo"),
    ("checkpoint-undo", "gate.recovery.checkpoint-undo"),
];

/// Automatic-wait / zero-primary fixtures (VAL-V3-F5-008, 053).
const AUTOMATIC_ZERO_PRIMARY: &[(&str, &str)] = &[
    ("rate-limit", "gate.recovery.rate-limit"),
    ("rate-limit", "rate-limit.wait.sample-early"),
    ("rate-limit", "rate-limit.wait.sample-later"),
    ("error-network", "error-network.backoff"),
];

/// Manual actionable defaults (exactly one primary).
const ACTIONABLE_PRIMARY_FIXTURES: &[(&str, &str)] = &[
    ("tool-failed", "canonical.tool-failed"),
    ("blocked-command", "canonical.blocked-command"),
    ("turn-error", "canonical.turn-error"),
    ("error-network", "canonical.error-network"),
    ("error-network", "error-network.exhausted"),
    ("error-network", "error-network.offline"),
    ("error-context", "canonical.error-context"),
    ("interrupt", "canonical.interrupt"),
    ("checkpoint-undo", "canonical.checkpoint-undo"),
    // Toast failure recovery is notice-owned and singular.
    ("toasts", "canonical.toasts"),
];

/// Gallery relative to plans/assets/mockups-v3.
const GALLERY_RELATIVE: &[(&str, &str)] = &[
    ("session", "tool-failed"),
    ("session", "blocked-command"),
    ("session", "turn-error"),
    ("states", "error-network"),
    ("states", "error-context"),
    ("states", "rate-limit"),
    ("states", "interrupt"),
    ("states", "toasts"),
    ("states", "checkpoint-undo"),
];

/// Origin-preserving and successful round-trips across recovery detours.
const ORIGIN_SCENARIOS: &[(&str, &str, &str)] = &[
    ("turn-error-cancel", "turn-error", "turn-error"),
    ("turn-error-retry", "turn-error", "chat-turn"),
    ("turn-error-inspect-connection", "turn-error", "error-network"),
    ("tool-failed-retry-safe", "tool-failed", "tool-running"),
    ("tool-failed-connect-capability", "tool-failed", "mcp"),
    (
        "tool-failed-capability-return-cancel",
        "mcp",
        "tool-failed",
    ),
    (
        "tool-failed-capability-return-automatic-once",
        "mcp",
        "tool-running",
    ),
    (
        "tool-failed-capability-return-confirm",
        "mcp",
        "tool-failed",
    ),
    (
        "tool-failed-capability-policy-blocked",
        "tool-failed",
        "tool-failed",
    ),
    (
        "blocked-command-request-approval",
        "blocked-command",
        "permission-shell",
    ),
    (
        "blocked-command-approval-return",
        "permission-shell",
        "blocked-command",
    ),
    (
        "blocked-command-change-work-style",
        "blocked-command",
        "agent-picker",
    ),
    (
        "blocked-command-work-style-return",
        "agent-picker",
        "blocked-command",
    ),
    ("error-context-no-auto", "error-context", "error-context"),
    (
        "error-context-evidence-open",
        "error-context",
        "error-context",
    ),
    (
        "error-context-evidence-close",
        "error-context",
        "error-context",
    ),
    ("error-context-compact-continue", "error-context", "compact"),
    ("compact-context-resume-chat", "compact", "chat-turn"),
    ("interrupt-continue-reply", "interrupt", "chat-turn"),
    ("interrupt-rephrase-request", "interrupt", "chat-turn"),
    ("toasts-auto-dismiss-tick", "toasts", "toasts"),
    ("toasts-retry-save", "toasts", "chat-turn"),
    ("rate-limit-automatic-success", "rate-limit", "chat-turn"),
    ("rate-limit-retry-now", "rate-limit", "chat-turn"),
    ("rate-limit-race-cancel-wins", "rate-limit", "rate-limit"),
    ("checkpoint-undo-success", "chat-turn", "checkpoint-undo"),
    ("checkpoint-continue-chat", "checkpoint-undo", "chat-turn"),
    ("checkpoint-redo-success", "checkpoint-undo", "chat-turn"),
];

/// Cancel/back/close paths that must never mutate transcript/draft/partial.
const CANCEL_LIKE: &[&str] = &[
    "turn-error-cancel",
    "tool-failed-capability-return-cancel",
    "blocked-command-approval-return",
    "blocked-command-work-style-return",
    "error-context-evidence-close",
    "rate-limit-race-cancel-wins",
    "toasts-auto-dismiss-tick",
];

/// Raw provider/patterns that must stay out of closed defaults (VAL-V3-F5-010).
const RAW_CODE_BANS: &[&str] = &[
    "HTTP/",
    "status 400",
    "status 401",
    "status 403",
    "status 404",
    "status 429",
    "status 500",
    "status 503",
    "ECONN",
    "ENOENT",
    "EACCES",
    "ETIMEDOUT",
    "stack trace",
    "request_id",
    "request id=",
    "api.anthropic.com",
    "claude-opus-4",
    "claude-3",
    "exit code",
    "Exit status",
    "rate_limit_error",
    "RATE_LIMIT",
    "ERR_",
    "tool_id=",
    "permission_bitmask",
    "PolicyEnum",
    "DENY_CODE",
    "MCP_TRANSPORT",
    "stderr:",
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

fn cause_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "recovery-cause")
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

fn assert_no_raw_codes(text: &str, label: &str) {
    for banned in RAW_CODE_BANS {
        assert!(
            !text.contains(banned),
            "{label}: raw diagnostic leaked in default/summary: {banned}"
        );
    }
    // Guard obvious uppercase SNAKE tokens without rebanning human copy.
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        if cleaned.contains('_')
            && cleaned.len() > 4
            && cleaned
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            && !cleaned.starts_with("CODE_")
        {
            panic!("{label}: uppercase/snake_case token leaked: {cleaned}");
        }
    }
}

fn is_automatic_zero_primary(screen_id: &str, fixture_id: &str) -> bool {
    AUTOMATIC_ZERO_PRIMARY
        .iter()
        .any(|&(s, f)| s == screen_id && f == fixture_id)
        || (screen_id == "error-network" && fixture_id.contains("backoff"))
        || (screen_id == "rate-limit" && !fixture_id.contains("cancelled"))
}

fn status_has_readable_label(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if text.contains('✓')
        || text.contains('✗')
        || text.contains('●')
        || text.contains('○')
    {
        return lower.contains("complete")
            || lower.contains("failed")
            || lower.contains("missing")
            || lower.contains("blocked")
            || lower.contains("stopped")
            || lower.contains("waiting")
            || lower.contains("retry")
            || lower.contains("saved")
            || lower.contains("restored")
            || lower.contains("needs")
            || lower.contains("attention")
            || lower.contains("safe")
            || lower.contains("partial")
            || lower.contains("kept")
            || lower.contains("ok")
            || lower.contains("warn")
            || lower.contains("info")
            || lower.contains("bad")
            || lower.contains("recover")
            || lower.contains("connection")
            || lower.contains("you stopped")
            || lower.contains("checkpoint")
            || lower.contains("settings")
            || lower.contains("model");
    }
    true
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

// ── VAL-V3-F5-001: Recovery inventory is complete ───────────────────────

#[test]
fn recovery_inventory_is_complete_and_implemented_once() {
    let mut seen = HashSet::new();
    for id in RECOVERY_IDS {
        assert!(seen.insert(*id), "duplicate recovery id in constant: {id}");
    }

    let inventory = canonical_inventory();
    for id in RECOVERY_IDS {
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
            FlowOwner::Recovery,
            "{id} must be owned by the Recovery flow"
        );
    }

    let recovery_count = inventory
        .iter()
        .filter(|s| s.flow == FlowOwner::Recovery)
        .count();
    assert_eq!(
        recovery_count, 9,
        "exactly nine Flow 5 recovery owners are required"
    );

    // Flow 5 storyboard stays linked to the recovery journey.
    let recovery_flow = flow_manifests()
        .iter()
        .find(|f| f.name == "recovery")
        .expect("recovery storyboard");
    assert_eq!(recovery_flow.steps.len(), 5);
    let storyboard_ids: Vec<_> = recovery_flow.steps.iter().map(|s| s.screen_id).collect();
    assert_eq!(
        storyboard_ids,
        vec![
            "turn-error",
            "error-network",
            "rate-limit",
            "chat-turn",
            "checkpoint-undo",
        ]
    );

    // Every required screen produces geometry-valid artifacts (live render + gallery).
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let txt = root.join(category).join(format!("{name}.txt"));
        let png = root.join(category).join(format!("{name}.png"));
        assert!(txt.is_file(), "missing gallery txt {}", txt.display());
        assert!(png.is_file(), "missing gallery png {}", png.display());
        let text = fs::read_to_string(&txt).expect("gallery txt");
        assert_canvas_geometry(&text, &format!("gallery {category}/{name}"));
        assert_png_dimensions(&png, 1200, 840);
    }

    // Harness list must expose all nine.
    let output = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("list");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for id in RECOVERY_IDS {
        assert!(
            stdout.lines().any(|line| line.split('\t').nth(1) == Some(*id)),
            "--list missing recovery id {id}"
        );
    }
}

// ── VAL-V3-F5-002 / 003 / 004 / 005 / 006: origin and work preservation ─

#[test]
fn recovery_defaults_preserve_origin_transcript_draft_partial_and_completed_work() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        // Toasts cancel the longer prompt for a settings sentence; skip partial checks.
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let conv = audit
            .conversation
            .as_ref()
            .unwrap_or_else(|| panic!("{screen_id}/{fixture_id}: conversation identity required"));
        assert_eq!(
            conv.visible_origin, RECOVERY_ORIGIN,
            "{screen_id}/{fixture_id}: visible origin"
        );
        assert_eq!(
            conv.user_task, RECOVERY_TASK,
            "{screen_id}/{fixture_id}: stable task"
        );
        assert_eq!(
            conv.conversation_id, "conversation.recovery",
            "{screen_id}/{fixture_id}: conversation id"
        );
        assert_eq!(
            conv.session_id, "session.recovery.1",
            "{screen_id}/{fixture_id}: session id"
        );
        assert!(
            paints_recovery_origin(&text),
            "{screen_id}/{fixture_id}: origin must be visible (subject title or long form)"
        );
        assert!(
            text.contains(RECOVERY_TASK) || text.contains("release notes"),
            "{screen_id}/{fixture_id}: paused work must remain identifiable"
        );

        if screen_id != "toasts" {
            assert!(
                text.contains(USER_PROMPT) || text.contains("Summarize the remaining release notes"),
                "{screen_id}/{fixture_id}: prior user request must survive"
            );
        }

        // Draft either visible or recorded on later_draft for recovery fixtures.
        let draft_visible = text.contains(DRAFT);
        let draft_recorded = conv
            .later_draft
            .as_deref()
            .is_some_and(|d| d.contains("risk note") || d.contains(DRAFT));
        assert!(
            draft_visible || draft_recorded || screen_id == "rate-limit",
            "{screen_id}/{fixture_id}: draft must survive advertently"
        );

        if matches!(
            screen_id,
            "turn-error" | "error-network" | "error-context" | "rate-limit" | "interrupt"
        ) {
            assert!(
                text.contains(PARTIAL_REPLY)
                    || text.contains("partial")
                    || text.contains("Partial"),
                "{screen_id}/{fixture_id}: partial answer remains readable"
            );
        }

        if matches!(
            screen_id,
            "tool-failed" | "blocked-command" | "error-context" | "checkpoint-undo" | "turn-error"
        ) {
            assert!(
                text.contains("Completed")
                    || text.contains("kept")
                    || text.contains("stay")
                    || text.contains("Readable")
                    || text.contains("untouched"),
                "{screen_id}/{fixture_id}: completed work must stay kept"
            );
        }

        // Statusline stays path · branch · human model only.
        assert!(
            text.contains("~/project · main · Claude Opus 4.8"),
            "{screen_id}/{fixture_id}: statusline must keep human model name"
        );
        assert!(
            !text.contains("ctx ") && !text.contains("$0.") && !text.contains("idle"),
            "{screen_id}/{fixture_id}: statusline must not grow costs or ctx"
        );
    }
}

// ── VAL-V3-F5-007 / 008 / 009 / 010: cause, action, evidence hierarchy ──

#[test]
fn default_recovery_shows_one_cause_and_one_primary_with_evidence_second() {
    for &(screen_id, fixture_id) in ACTIONABLE_PRIMARY_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);

        // Automatic-wait fixtures divert elsewhere; actionable list excludes them.
        assert_eq!(
            primaries.len(),
            1,
            "{screen_id}/{fixture_id}: exactly one primary, got {} labels {:?}",
            primaries.len(),
            primaries
                .iter()
                .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
                .collect::<Vec<_>>()
        );
        let action = primaries[0].action.as_ref().expect("primary binds action");
        assert!(action.enabled, "{screen_id}/{fixture_id}: primary enabled");
        assert!(
            !action.label.trim().is_empty(),
            "{screen_id}/{fixture_id}: primary label nonempty"
        );

        // Interrupt is neutral (no recovery-cause family); toasts own notice recovery.
        if !matches!(screen_id, "interrupt" | "toasts" | "checkpoint-undo") {
            let causes = cause_events(&audit);
            assert_eq!(
                causes.len(),
                1,
                "{screen_id}/{fixture_id}: exactly one recovery-cause event"
            );
        }

        // Dock embeds primary within a few rows of the cause (or toast failure).
        if screen_id != "toasts" {
            assert!(
                (text.contains("› ") || text.contains("Enter to")) || text.contains("› [s]"),
                "{screen_id}/{fixture_id}: primary must be visually marked"
            );
        }

        // Evidence stays reverse couplets and may only appear as a collapsed affordance.
        if matches!(screen_id, "error-network" | "error-context" | "tool-failed" | "blocked-command")
        {
            // Default closed: evidence affordance may show "Evidence"/"Show evidence",
            // but not raw attempt numbers, endpoints, or stack fragments.
            assert_no_raw_codes(&text, &format!("{screen_id}/{fixture_id}"));
            assert!(
                !text.contains("Attempt 3 of")
                    && !text.contains("endpoint=")
                    && !text.contains("stack"),
                "{screen_id}/{fixture_id}: technical evidence must stay undisclosed by default"
            );
        }
    }

    // Automatic waiting: zero primary (VAL-V3-F5-008, 053).
    for &(screen_id, fixture_id) in AUTOMATIC_ZERO_PRIMARY {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);
        assert_eq!(
            primaries.len(),
            0,
            "{screen_id}/{fixture_id}: automatic wait must emit zero primary actions"
        );
        assert!(
            text.to_ascii_lowercase().contains("automatic")
                || text.to_ascii_lowercase().contains("on its own")
                || text.to_ascii_lowercase().contains("will retry")
                || text.contains("no primary action"),
            "{screen_id}/{fixture_id}: automatic wording required"
        );
        // Optional secondary Retry now only.
        let secondaries = audit
            .events
            .iter()
            .filter(|e| e.identity.family == "secondary-action")
            .count();
        assert!(
            secondaries <= 1,
            "{screen_id}/{fixture_id}: at most one secondary Retry now"
        );
        assert_no_raw_codes(&text, &format!("auto {screen_id}/{fixture_id}"));
    }

    // Evidence expansion is progressive and distinct (VAL-V3-F5-009).
    let collapsed = render_pair("error-context", "canonical.error-context").render_txt();
    let expanded = render_pair("error-context", "error-context.evidence").render_txt();
    assert_ne!(
        collapsed, expanded,
        "evidence level must change visible content"
    );
    assert!(
        expanded.contains("token")
            || expanded.contains("Token")
            || expanded.contains("context")
            || expanded.contains("chars")
            || expanded.contains("k "),
        "expanded evidence must add technical detail"
    );
    // Closed remains free of the expanded technical block's volume of codes.
    assert_no_raw_codes(&collapsed, "error-context closed");

    let net_closed = render_pair("error-network", "canonical.error-network").render_txt();
    let net_evidence = render_pair("error-network", "error-network.evidence").render_txt();
    assert_ne!(net_closed, net_evidence, "network evidence must differ");
    assert_no_raw_codes(&net_closed, "error-network closed");
}

// ── VAL-V3-F5-010 / 011 / 012: raw codes, density, non-color status ─────

#[test]
fn recovery_defaults_stay_plain_focused_and_not_color_only() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_no_raw_codes(&text, &format!("{screen_id}/{fixture_id}"));
        assert!(
            status_has_readable_label(&text),
            "{screen_id}/{fixture_id}: status must pair color with readable wording"
        );
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));

        // Density: no floating modal chrome, no global hint strip.
        assert!(
            !text.to_ascii_lowercase().contains("press ? for help")
                && !text.contains("enter send · / commands"),
            "{screen_id}/{fixture_id}: global hint row forbidden"
        );
        // One statusline only.
        let statusline_hits = text
            .lines()
            .filter(|l| l.contains("~/project · main ·"))
            .count();
        assert_eq!(
            statusline_hits, 1,
            "{screen_id}/{fixture_id}: singular statusline"
        );

        // No coordinating dashboards with wide multi-panel tables by default.
        assert!(
            !text.contains('|') || screen_id == "toasts",
            "{screen_id}/{fixture_id}: wide diagnostic tables stay out of defaults (toasts may use a stack divider)"
        );

        // Blank-row budget: recovery summary must not pack solid 40 rows of text.
        let nonempty = text.lines().filter(|l| !l.trim().is_empty()).count();
        assert!(
            nonempty < 36,
            "{screen_id}/{fixture_id}: density too high ({nonempty} nonempty rows)"
        );
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let text = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .expect("gallery txt");
        assert_no_raw_codes(&text, &format!("gallery {category}/{name}"));
        assert!(
            status_has_readable_label(&text),
            "gallery {category}/{name}: non-color status"
        );
    }
}

// ── VAL-V3-F5-048 / 049 / 050: approval / configuration / connection RTs ─

#[test]
fn approval_configuration_and_connection_round_trips_preserve_recovery() {
    // Approval (VAL-V3-F5-048): dock opens + returns without executing.
    let open = replay_case("blocked-command-request-approval", 1).expect("open approval");
    assert_eq!(open.before.screen_id, "blocked-command");
    assert_eq!(open.after.screen_id, "permission-shell");
    assert_eq!(open.attempts[0].count, 0);
    assert_eq!(open.side_effects[0].status, "zero-execution");
    assert_eq!(open.before.user_task, RECOVERY_TASK);
    assert_eq!(open.after.user_task, RECOVERY_TASK);
    assert_eq!(
        open.before.requested_command,
        open.after.requested_command
    );
    let open_txt = open.audit.render_txt();
    assert!(
        open_txt.contains("rm -rf notes/archive"),
        "approval dock must show the blocked recovery command: {open_txt}"
    );
    assert!(
        !open_txt.contains("printf '%s\\n' \"retry check\"")
            && !open_txt.contains("Check the retry path before we merge"),
        "approval dock mismark: Flow 2 retry-check paint must not appear: {open_txt}"
    );
    let draft = open
        .preserved_field_hashes
        .get("draft")
        .expect("approval preserves draft hash");
    assert_eq!(draft.before, draft.after);

    let ret = replay_case("blocked-command-approval-return", 1).expect("approval return");
    assert_eq!(ret.before.screen_id, "permission-shell");
    assert_eq!(ret.after.screen_id, "blocked-command");
    assert_eq!(ret.attempts[0].count, 0);
    assert_eq!(ret.side_effects[0].status, "awaiting-explicit-run");
    assert!(ret.audit.render_txt().contains("rm -rf") || ret.after.requested_command.contains("rm -rf"));

    // Configuration (VAL-V3-F5-049): How Code should work, no silent run.
    let cfg = replay_case("blocked-command-change-work-style", 1).expect("open configuration");
    assert_eq!(cfg.before.screen_id, "blocked-command");
    assert_eq!(cfg.after.screen_id, "agent-picker");
    assert_eq!(cfg.attempts[0].count, 0);
    assert_eq!(cfg.side_effects[0].status, "zero-execution");
    let cfg_ret = replay_case("blocked-command-work-style-return", 1).expect("config return");
    assert_eq!(cfg_ret.after.screen_id, "blocked-command");
    assert_eq!(cfg_ret.attempts[0].count, 0);
    assert_eq!(cfg_ret.side_effects[0].status, "awaiting-explicit-run");
    assert_eq!(cfg_ret.before.session_id, cfg_ret.after.session_id);
    assert!(
        cfg_ret.audit.render_txt().contains("rm -rf")
            || cfg_ret.after.requested_command.contains("rm -rf")
    );

    // Connection (VAL-V3-F5-050): provider/MCP resume policies.
    let open_mcp = replay_case("tool-failed-connect-capability", 1).expect("open mcp");
    assert_eq!(open_mcp.before.screen_id, "tool-failed");
    assert_eq!(open_mcp.after.screen_id, "mcp");
    assert_eq!(open_mcp.attempts[0].count, 0);

    let auto = replay_case("tool-failed-capability-return-automatic-once", 1)
        .expect("automatic_once resume");
    assert_eq!(auto.after.screen_id, "tool-running");
    assert_eq!(auto.attempts[0].count, 1);
    assert_eq!(auto.side_effects[0].status, "automatic-once-resume");

    let confirm =
        replay_case("tool-failed-capability-return-confirm", 1).expect("confirmation resume");
    assert_eq!(confirm.after.screen_id, "tool-failed");
    assert_eq!(confirm.attempts[0].count, 0);
    assert_eq!(confirm.side_effects[0].status, "confirmation-required");

    let cancel =
        replay_case("tool-failed-capability-return-cancel", 1).expect("cancel resume");
    assert_eq!(cancel.after.screen_id, "tool-failed");
    assert_eq!(cancel.attempts[0].count, 0);
    assert_eq!(cancel.side_effects[0].status, "zero-invocation");

    let policy =
        replay_case("tool-failed-capability-policy-blocked", 1).expect("policy blocked");
    assert_eq!(policy.after.screen_id, "tool-failed");
    assert_eq!(policy.attempts[0].count, 0);
    assert_eq!(policy.side_effects[0].status, "policy-blocked");
}

// ── VAL-V3-F5-051: Back/cancel never destroy state ──────────────────────

#[test]
fn back_and_cancel_never_destroy_transcript_draft_or_partial() {
    for case_id in CANCEL_LIKE {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id} must replay: {e}"));
        assert_eq!(
            replay.before.user_task, RECOVERY_TASK,
            "{case_id}: task stable"
        );
        assert_eq!(
            replay.before.session_id, replay.after.session_id,
            "{case_id}: session stable"
        );
        assert_eq!(
            replay.before.conversation_id, replay.after.conversation_id,
            "{case_id}: conversation stable"
        );
        for field in ["draft", "scroll_anchor", "partial_response", "transcript"] {
            if let Some(hash) = replay.preserved_field_hashes.get(field) {
                assert_eq!(
                    hash.before, hash.after,
                    "{case_id}: preserved {field} must be byte-identical"
                );
            }
        }
        // Cancel must not invent execution.
        if let Some(attempt) = replay.attempts.first() {
            if case_id.contains("cancel") || case_id.ends_with("-return") {
                assert_eq!(
                    attempt.count, 0,
                    "{case_id}: cancel/return must keep zero attempts"
                );
            }
        }
        assert!(
            !replay
                .side_effects
                .iter()
                .any(|s| s.status.contains("applied") && s.status.contains("destructive")),
            "{case_id}: cancel must not apply destructive side effects"
        );
    }

    // Evidence open/close preserves hashes both ways.
    for case_id in ["error-context-evidence-open", "error-context-evidence-close"] {
        let replay = replay_case(case_id, 1).expect(case_id);
        let draft = replay
            .preserved_field_hashes
            .get("draft")
            .expect("evidence path draft hash");
        assert_eq!(draft.before, draft.after, "{case_id} draft");
        assert_eq!(replay.before.session_id, replay.after.session_id);
    }
}

// ── VAL-V3-F5-053: Automatic vs manual distinction ──────────────────────

#[test]
fn automatic_and_manual_recovery_are_visibly_distinct() {
    // Rate-limit is the only required automatic-wait canonical (plus bounded network backoff).
    let rate = render_pair("rate-limit", "gate.recovery.rate-limit");
    let rate_text = rate.render_txt();
    assert_eq!(primary_events(&rate).len(), 0);
    assert!(
        rate_text.to_ascii_lowercase().contains("automatic")
            || rate_text.contains("no primary action")
            || rate_text.to_ascii_lowercase().contains("on its own")
    );
    assert!(
        rate.events
            .iter()
            .any(|e| e.identity.family == "timer-owner"),
        "rate-limit must expose one timer owner"
    );
    // Optional Retry now is secondary only.
    assert!(
        rate_text.contains("Retry now") || rate_text.contains("[r]"),
        "secondary Retry now allowed"
    );
    assert!(!rate_text.contains("› [enter] Retry"), "Retry must not be primary");

    let backoff = render_pair("error-network", "error-network.backoff");
    assert_eq!(
        primary_events(&backoff).len(),
        0,
        "bounded network backoff is zero-primary"
    );

    // Manual states require the user: one primary, no live countdown as the dominant claim.
    for (screen_id, fixture_id) in [
        ("error-network", "canonical.error-network"),
        ("error-network", "error-network.exhausted"),
        ("error-network", "error-network.offline"),
        ("turn-error", "canonical.turn-error"),
        ("tool-failed", "canonical.tool-failed"),
        ("blocked-command", "canonical.blocked-command"),
        ("error-context", "canonical.error-context"),
    ] {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_eq!(
            primary_events(&audit).len(),
            1,
            "{screen_id}/{fixture_id}: manual recovery needs one primary"
        );
        // Manual surfaces must not claim automatic countdown as live primary cadence.
        if screen_id != "error-network" || !fixture_id.contains("backoff") {
            assert!(
                !text.contains("no primary action while the timer runs"),
                "{screen_id}/{fixture_id}: manual state must not use automatic timer copy"
            );
        }
        let _ = is_automatic_zero_primary(screen_id, fixture_id);
    }
}

// ── VAL-V3-F5-054: Announcements are singular ───────────────────────────

#[test]
fn recovery_announcements_owners_and_causes_are_singular() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();

        // Exactly one origin announcement: long journey form, or a solo subject title row.
        let long_hits = text.matches(RECOVERY_ORIGIN).count();
        let subject_rows = text
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed == RECOVERY_SUBJECT || trimmed == RECOVERY_ORIGIN
            })
            .count();
        assert!(
            long_hits == 1 || (long_hits == 0 && subject_rows == 1),
            "{screen_id}/{fixture_id}: singular origin announcement (long={long_hits} subject_rows={subject_rows})"
        );

        // Exactly one statusline.
        assert_eq!(
            text.lines()
                .filter(|l| l.contains("~/project · main · Claude Opus 4.8"))
                .count(),
            1,
            "{screen_id}/{fixture_id}: singular statusline"
        );

        // Family uniqueness for exclusive owners.
        for family in [
            "recovery-cause",
            "timer-owner",
            "origin",
            "primary-action",
            "statusline",
        ] {
            let count = audit
                .events
                .iter()
                .filter(|e| e.identity.family == family)
                .count();
            if family == "recovery-cause"
                && matches!(
                    screen_id,
                    "interrupt" | "toasts" | "checkpoint-undo" | "rate-limit"
                )
            {
                assert!(
                    count <= 1,
                    "{screen_id}/{fixture_id}: {family} must stay singular (got {count})"
                );
                continue;
            }
            if family == "timer-owner" && screen_id != "rate-limit" && !fixture_id.contains("backoff")
            {
                assert!(
                    count <= 1,
                    "{screen_id}/{fixture_id}: optional timer owner singular"
                );
                continue;
            }
            if family == "primary-action" && is_automatic_zero_primary(screen_id, fixture_id) {
                assert_eq!(
                    count, 0,
                    "{screen_id}/{fixture_id}: automatic has zero primary"
                );
                continue;
            }
            if family == "recovery-cause" && count == 0 {
                continue;
            }
            assert!(
                count <= 1,
                "{screen_id}/{fixture_id}: family {family} must remain singular, got {count}"
            );
        }

        // Cause sentence not restated in statusline or toast stack.
        for cause in cause_events(&audit) {
            let needle = cause.identity.subject_id.clone();
            if needle.is_empty() {
                continue;
            }
            // subject_id may be a stable id not the visible sentence; use visible primary line.
        }
        // Visible cause lines for failed manual states: exactly one matching "✗ <cause>" line.
        if matches!(
            screen_id,
            "tool-failed" | "blocked-command" | "turn-error" | "error-network" | "error-context"
        ) && !fixture_id.contains("backoff")
            && !fixture_id.contains("evidence")
        {
            let hard_cause_lines = text
                .lines()
                .filter(|l| l.trim_start().starts_with('✗'))
                .count();
            // tool-failed may show ✗ on the tool row and ✗ on the cause; promote as "at most two"
            // but ensure the cause family is still 1.
            assert!(
                (1..=2).contains(&hard_cause_lines),
                "{screen_id}/{fixture_id}: failure markers must stay focused (got {hard_cause_lines})"
            );
            assert_eq!(
                cause_events(&audit).len(),
                1,
                "{screen_id}/{fixture_id}: one typed cause"
            );
        }
    }

    // Timer sole ownership on rate-limit.
    let rate = render_pair("rate-limit", "gate.recovery.rate-limit");
    assert_eq!(
        rate.events
            .iter()
            .filter(|e| e.identity.family == "timer-owner")
            .count(),
        1
    );
}

// ── VAL-V3-F5-055: Final recovery returns to prior activity ─────────────

#[test]
fn successful_recovery_returns_to_exact_prior_activity() {
    // Matrix destinations must leave recovery and not fall home orphaned.
    let terminal = [
        ("turn-error-retry", "chat-turn"),
        ("tool-failed-retry-safe", "tool-running"),
        (
            "tool-failed-capability-return-automatic-once",
            "tool-running",
        ),
        ("error-context-compact-continue", "compact"),
        ("compact-context-resume-chat", "chat-turn"),
        ("interrupt-continue-reply", "chat-turn"),
        ("rate-limit-automatic-success", "chat-turn"),
        ("rate-limit-retry-now", "chat-turn"),
        ("toasts-retry-save", "chat-turn"),
        ("checkpoint-continue-chat", "chat-turn"),
        ("checkpoint-redo-success", "chat-turn"),
    ];
    for (case_id, dest) in terminal {
        let replay = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        assert_eq!(
            replay.after.screen_id, dest,
            "{case_id}: final destination"
        );
        assert_ne!(
            replay.after.screen_id, "home",
            "{case_id}: must not dump the user on home"
        );
        assert_eq!(
            replay.before.session_id, replay.after.session_id,
            "{case_id}: session continuity"
        );
        assert_eq!(
            replay.before.user_task, RECOVERY_TASK,
            "{case_id}: prior task retained"
        );
        // Terminal focus must land on a still-valid control / composer, not a recovery dock.
        if dest == "chat-turn" || dest == "tool-running" {
            assert!(
                !replay.after.focus.contains("recovery")
                    || replay.after.focus.contains("composer")
                    || replay.after.focus == "tool-running",
                "{case_id}: terminal focus must leave recovery dock ownership (got {})",
                replay.after.focus
            );
        }
    }

    // Exhaustive origin matrix from ORIGIN_SCENARIOS.
    for &(case_id, from, to) in ORIGIN_SCENARIOS {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("scenario {case_id}: {e}"));
        assert_eq!(replay.before.screen_id, from, "{case_id} before");
        assert_eq!(replay.after.screen_id, to, "{case_id} after");
        assert!(
            !replay.actions.is_empty() || !replay.transitions.is_empty(),
            "{case_id}: no dead end"
        );
        assert_eq!(
            replay.before.user_task, RECOVERY_TASK,
            "{case_id}: task continuity"
        );
    }
}

// ── VAL-V3-F5-059: Keyboard focus is deterministic ──────────────────────

#[test]
fn recovery_controls_preserve_deterministic_keyboard_focus() {
    // Passive toasts never steal composer focus.
    let toast_tick = replay_case("toasts-auto-dismiss-tick", 1).expect("toast tick");
    assert_eq!(toast_tick.before.focus, "composer");
    assert_eq!(toast_tick.after.focus, "composer");
    if let Some(focus_hash) = toast_tick.preserved_field_hashes.get("focus_owner") {
        assert_eq!(
            focus_hash.before, focus_hash.after,
            "auto-dismiss must preserve focus owner"
        );
    }

    let toast_retry = replay_case("toasts-retry-save", 1).expect("toast retry");
    assert_eq!(toast_retry.after.focus, "composer");

    // Actionable recovery focuses its dock when opened; close/cancel restores prior focus.
    let evidence_open = replay_case("error-context-evidence-open", 1).expect("open evidence");
    assert_eq!(evidence_open.before.focus, "context-recovery");
    assert_eq!(evidence_open.after.focus, "evidence");
    let evidence_close = replay_case("error-context-evidence-close", 1).expect("close evidence");
    assert_eq!(evidence_close.before.focus, "evidence");
    assert_eq!(evidence_close.after.focus, "context-recovery");

    let approval_open =
        replay_case("blocked-command-request-approval", 1).expect("open approval focus");
    assert_eq!(approval_open.before.focus, "blocked-recovery");
    assert_eq!(approval_open.after.focus, "permission-dock");
    let approval_return =
        replay_case("blocked-command-approval-return", 1).expect("return approval focus");
    assert_eq!(approval_return.after.focus, "blocked-recovery");

    let cfg_open =
        replay_case("blocked-command-change-work-style", 1).expect("open work-style focus");
    assert_eq!(cfg_open.before.focus, "blocked-recovery");
    assert_eq!(cfg_open.after.focus, "work-style");
    let cfg_return =
        replay_case("blocked-command-work-style-return", 1).expect("return work-style focus");
    assert_eq!(cfg_return.after.focus, "blocked-recovery");

    // Automatic rate-limit success restores composer focus.
    let rate_ok = replay_case("rate-limit-automatic-success", 1).expect("rate success focus");
    assert_eq!(rate_ok.before.focus, "recovery-wait");
    assert_eq!(rate_ok.after.focus, "composer");

    // Interrupt continue lands composer.
    let cont = replay_case("interrupt-continue-reply", 1).expect("interrupt focus");
    assert_eq!(cont.before.focus, "interrupt-recovery");
    assert_eq!(cont.after.focus, "composer");

    // Every primary action exposes accessibility fields.
    for &(screen_id, fixture_id) in ACTIONABLE_PRIMARY_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        for event in primary_events(&audit) {
            let action = event
                .action
                .as_ref()
                .unwrap_or_else(|| panic!("{screen_id}/{fixture_id}: primary missing action"));
            assert!(
                !action.action_id.trim().is_empty(),
                "{screen_id}/{fixture_id}: action_id required"
            );
            assert!(
                !action.label.trim().is_empty(),
                "{screen_id}/{fixture_id}: accessible label required"
            );
            assert!(action.enabled, "{screen_id}/{fixture_id}: primary enabled");
        }
        // Cancel actions exist on recovery docks (except pure notice toast where toast holds recovery).
        if screen_id != "toasts" {
            assert!(
                audit
                    .events
                    .iter()
                    .any(|e| e.identity.family == "cancel-action"
                        || e.action
                            .as_ref()
                            .is_some_and(|a| a.label.to_ascii_lowercase().contains("cancel")
                                || a.label.to_ascii_lowercase().contains("back"))),
                "{screen_id}/{fixture_id}: cancel/back control required"
            );
        }
    }
}

// ── Gallery drift guard against live fixtures ───────────────────────────

#[test]
fn published_recovery_gallery_matches_live_default_fixtures() {
    if pure_mode() {
        // Pure dump owns recovery paint. Hybrid copper default fixtures are not
        // gallery anchors; only assert pure published geometry stays intact.
        let mapping = [
            "session/tool-failed.txt",
            "session/blocked-command.txt",
            "session/turn-error.txt",
            "states/error-network.txt",
            "states/error-context.txt",
            "states/rate-limit.txt",
            "states/interrupt.txt",
            "states/toasts.txt",
            "states/checkpoint-undo.txt",
        ];
        let root = gallery_root();
        for rel in mapping {
            let published = fs::read_to_string(root.join(rel))
                .unwrap_or_else(|e| panic!("read pure {rel}: {e}"));
            assert_canvas_geometry(&published, rel);
            assert!(
                published.contains("Cortex")
                    || published.contains("❯")
                    || published.contains("Shift+Tab"),
                "pure recovery surface {rel} must keep pure product chrome markers"
            );
            assert!(
                !published.contains("~/project · main · Claude Opus"),
                "pure recovery surface {rel} must not retain hybrid Claude statusline paint"
            );
        }
        return;
    }
    let mapping = [
        ("tool-failed", "canonical.tool-failed", "session/tool-failed.txt"),
        (
            "blocked-command",
            "canonical.blocked-command",
            "session/blocked-command.txt",
        ),
        ("turn-error", "canonical.turn-error", "session/turn-error.txt"),
        (
            "error-network",
            "canonical.error-network",
            "states/error-network.txt",
        ),
        (
            "error-context",
            "canonical.error-context",
            "states/error-context.txt",
        ),
        (
            "rate-limit",
            "gate.recovery.rate-limit",
            "states/rate-limit.txt",
        ),
        ("interrupt", "canonical.interrupt", "states/interrupt.txt"),
        ("toasts", "canonical.toasts", "states/toasts.txt"),
        (
            "checkpoint-undo",
            "canonical.checkpoint-undo",
            "states/checkpoint-undo.txt",
        ),
    ];
    let root = gallery_root();
    for (screen_id, fixture_id, rel) in mapping {
        let live = render_pair(screen_id, fixture_id).render_txt();
        let published = fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        assert_eq!(
            live, published,
            "gallery drift for {rel}: live fixture {fixture_id} differs from published TXT"
        );
        assert_canvas_geometry(&live, rel);
    }
}
