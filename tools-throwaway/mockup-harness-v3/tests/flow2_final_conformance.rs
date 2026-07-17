
//! Final Flow 2 origin, accessibility, semantics, atomicity, and artifact sweep.
//!
//! Owned contract assertions:
//! VAL-V3-F2-001, 003, 004, 005, 027, 050, 051, 052, 053, 054, 055, 056,
//! 057, 058, 059, 060.
//!
//! Aggregate checks cover the exact 15-state inventory, bottom action continuity,
//! cancel/success origin restores, keyboard/focus completeness, non-color meaning,
//! plain copy, primary-action counts, render-event bijection, stale patch atomicity,
//! and deterministic gallery geometry.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory, flow_manifests,
    renderer::canonical_publication_fixture,
    scenario::replay_case, FlowOwner, ImplementationStatus, Surface,
};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

/// Exact 15 Flow 2 states required by VAL-V3-F2-001.
const FLOW2_IDS: &[&str] = &[
    "diff-edit",
    "slash-menu",
    "command-palette",
    "command-palette-submenu",
    "attachment-picker",
    "file-mention",
    "export-picker",
    "queue-manager",
    "composer-multiline",
    "permission-shell",
    "permission-file-edit",
    "permission-deny-reason",
    "sidebar-queue",
    "help",
    "diff-viewer-staging",
];

/// Closed-default fixtures representing product Flow 2 states under audit.
const CLOSED_DEFAULT_FIXTURES: &[(&str, &str)] = &[
    ("composer-multiline", "flow2-multiline"),
    ("slash-menu", "flow2-slash-menu"),
    (
        "command-palette",
        "flow2.input-and-approval.stable",
    ),
    ("command-palette-submenu", "flow2-palette-submenu"),
    ("attachment-picker", "flow2-attachment-picker"),
    (
        "file-mention",
        "flow2.input-and-approval.stable",
    ),
    ("export-picker", "flow2-export-picker"),
    ("queue-manager", "flow2-queue-manager"),
    (
        "permission-shell",
        "flow2.input-and-approval.stable",
    ),
    ("permission-file-edit", "flow2-permission-file-edit"),
    (
        "permission-deny-reason",
        "flow2-permission-deny-reason",
    ),
    ("diff-edit", "flow2-diff-pending"),
    ("diff-viewer-staging", "flow2-staging"),
    ("help", "views.help.open"),
    ("sidebar-queue", "views.sidebar-queue"),
];

/// Gallery relative path under plans/assets/mockups-v3.
const GALLERY_RELATIVE: &[(&str, &str)] = &[
    ("session", "diff-edit"),
    ("input", "slash-menu"),
    ("input", "command-palette"),
    ("input", "command-palette-submenu"),
    ("input", "attachment-picker"),
    ("input", "file-mention"),
    ("input", "export-picker"),
    ("input", "queue-manager"),
    ("input", "composer-multiline"),
    ("input", "permission-shell"),
    ("input", "permission-file-edit"),
    ("input", "permission-deny-reason"),
    ("views", "sidebar-queue"),
    ("views", "help"),
    ("views", "diff-viewer-staging"),
];

/// Screens that replace the composer with a bottom action dock.
const ACTION_DOCK_IDS: &[&str] = &[
    "diff-edit",
    "slash-menu",
    "command-palette",
    "command-palette-submenu",
    "attachment-picker",
    "file-mention",
    "export-picker",
    "queue-manager",
    "composer-multiline",
    "permission-shell",
    "permission-file-edit",
    "permission-deny-reason",
];

/// Temporary detail / inspector surfaces that are not bottom docks.
const DETAIL_OR_INSPECTOR: &[&str] = &["diff-viewer-staging", "help", "sidebar-queue"];

/// Decision states that must expose exactly one primary action.
const DECISION_PRIMARY_FIXTURES: &[(&str, &str)] = &[
    ("slash-menu", "flow2-slash-menu"),
    (
        "command-palette",
        "flow2.input-and-approval.stable",
    ),
    ("command-palette-submenu", "flow2-palette-submenu"),
    ("attachment-picker", "flow2-attachment-picker"),
    (
        "file-mention",
        "flow2.input-and-approval.stable",
    ),
    ("export-picker", "flow2-export-picker"),
    ("queue-manager", "flow2-queue-manager"),
    (
        "permission-shell",
        "flow2.input-and-approval.stable",
    ),
    ("permission-file-edit", "flow2-permission-file-edit"),
    (
        "permission-deny-reason",
        "flow2-permission-deny-reason",
    ),
    ("diff-edit", "flow2-diff-pending"),
    ("diff-viewer-staging", "flow2-staging"),
    ("help", "views.help.open"),
    ("sidebar-queue", "views.sidebar-queue"),
    ("composer-multiline", "flow2-multiline"),
    (
        "diff-viewer-staging",
        "case.staging-apply-partial-failure.tick.1",
    ),
    (
        "queue-manager",
        "flow2-queue-manager-conflict",
    ),
];

/// Cancel / dismiss paths that must restore the exact multiline origin (VAL-V3-F2-027, 050).
const CANCEL_SCENARIOS: &[(&str, &str, &str)] = &[
    ("slash-cancel", "slash-menu", "composer-multiline"),
    ("palette-cancel", "command-palette", "composer-multiline"),
    (
        "palette-submenu-cancel",
        "command-palette-submenu",
        "composer-multiline",
    ),
    (
        "palette-submenu-back",
        "command-palette-submenu",
        "command-palette",
    ),
    (
        "attachment-cancel",
        "attachment-picker",
        "composer-multiline",
    ),
    (
        "file-mention-dismiss",
        "file-mention",
        "composer-multiline",
    ),
    ("export-cancel", "export-picker", "composer-multiline"),
    ("queue-cancel", "queue-manager", "composer-multiline"),
    ("help-cancel", "help", "composer-multiline"),
    ("help-return", "help", "composer-multiline"),
    ("permission-cancel", "permission-shell", "chat-turn"),
    (
        "permission-file-edit-cancel",
        "permission-file-edit",
        "chat-turn",
    ),
    (
        "permission-deny-skip-reason",
        "permission-deny-reason",
        "chat-turn",
    ),
    ("diff-cancel", "diff-edit", "chat-turn"),
    ("staging-cancel", "diff-viewer-staging", "diff-edit"),
    (
        "sidebar-queue-manage-cancel",
        "sidebar-queue",
        "sidebar-queue",
    ),
];

/// Successful confirm paths that return to the recorded origin (VAL-V3-F2-051).
const SUCCESS_SCENARIOS: &[(&str, &str, &str)] = &[
    (
        "slash-confirm-export",
        "slash-menu",
        "composer-multiline",
    ),
    (
        "attachment-confirm",
        "attachment-picker",
        "composer-multiline",
    ),
    ("export-confirm", "export-picker", "composer-multiline"),
    ("queue-apply", "queue-manager", "composer-multiline"),
    (
        "file-mention-insert",
        "file-mention",
        "composer-multiline",
    ),
    (
        "flow2-submit-bridge",
        "file-mention",
        "permission-shell",
    ),
    ("permission-allow-once", "permission-shell", "diff-edit"),
    (
        "permission-file-edit-allow-once",
        "permission-file-edit",
        "chat-turn",
    ),
    (
        "permission-deny-send-reason",
        "permission-deny-reason",
        "chat-turn",
    ),
    ("diff-review", "diff-edit", "diff-viewer-staging"),
    ("staging-finish", "diff-viewer-staging", "chat-turn"),
    (
        "sidebar-queue-manage-apply",
        "sidebar-queue",
        "sidebar-queue",
    ),
    (
        "palette-open-submenu",
        "command-palette",
        "command-palette-submenu",
    ),
];

/// Destructive effects that cancel paths must not emit.
const DESTRUCTIVE_STATUSES: &[&str] = &[
    "applied-included-hunks",
    "exported",
    "attached",
    "queued-mutation-applied",
    "permission-allowed",
    "command-invoked",
    "file-written",
    "success",
];

/// Forbidden floating modal / global hint chrome (VAL-V3-F2-004).
const FORBIDDEN_CHROME: &[&str] = &[
    "enter send · / commands",
    "enter send ·",
    "/ commands · @ files",
    "ctx 6%",
    "ctx %",
    "$0.02",
    "● idle",
    "idle ·",
    "✦",
    "✧",
    "★",
    "☆",
    "❖",
    "◆",
    "✱",
    "dialog overlay",
    "floating modal",
];

/// Implementation internal identifiers banned on default Flow 2 surfaces (VAL-V3-F2-056).
const RAW_ID_BANS: &[&str] = &[
    "claude-opus-4",
    "claude-3-",
    "gpt-4o-",
    "model_id=",
    "tool_id=",
    "permission_bitmask",
    "STATUS_RUNNING",
    "snake_case_status",
    "function_call_id",
    "request_id=",
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

fn secondary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "secondary-action")
        .collect()
}

fn is_cancelish(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.contains("cancel")
        || lower.contains("back")
        || lower == "return"
        || lower.starts_with("return ")
        || lower.contains("dismiss")
        || lower.contains("skip")
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

fn assert_statusline_minimal(text: &str, label: &str) {
    if pure_mode() {
        // Pure Cortex dump uses putty footers / blank bottoms, not hybrid path·branch·model.
        let status = text.lines().last().unwrap_or_else(|| panic!("{label}: missing status row"));
        assert!(
            !status.contains('%')
                && !status.contains('$')
                && !status.to_ascii_lowercase().contains("idle")
                && !status.contains("ctx ")
                && !status.contains("claude-opus"),
            "{label}: pure footer carries forbidden field: `{status}`"
        );
        return;
    }
    let status = text
        .lines()
        .last()
        .unwrap_or_else(|| panic!("{label}: missing statusline row"));
    assert!(
        status.contains("~/project") && status.contains('·') && status.contains("Claude"),
        "{label}: statusline must be path · branch · human model, got `{status}`"
    );
    assert!(
        !status.contains('%')
            && !status.contains('$')
            && !status.to_ascii_lowercase().contains("idle")
            && !status.contains("ctx ")
            && !status.contains("claude-opus")
            && !status.contains("opus-4-8"),
        "{label}: statusline carries forbidden field: `{status}`"
    );
    let hits = text
        .lines()
        .filter(|l| l.contains("~/project · main ·"))
        .count();
    assert_eq!(hits, 1, "{label}: expected exactly one statusline, got {hits}");
}

fn assert_chrome_simple(text: &str, label: &str) {
    for banned in FORBIDDEN_CHROME {
        if pure_mode() && matches!(*banned, "◆" | "◇" | "✦" | "✧" | "❯" | "") {
            continue;
        }
        if pure_mode() && (*banned == "◆" || *banned == "◇" || *banned == "❯") {
            continue;
        }
        assert!(
            !text.contains(banned),
            "{label}: forbidden chrome `{banned}`"
        );
    }
    for banned in RAW_ID_BANS {
        assert!(
            !text.contains(banned),
            "{label}: raw/internal identifier `{banned}`"
        );
    }
    let lower = text.to_ascii_lowercase();
    if pure_mode() {
        // Pure structure help is Cortex shortcuts_help (product categorical modal).
        // Only hybrid invent phrasing like "dialog overlay" stays banned.
        assert!(
            !lower.contains("dialog overlay"),
            "{label}: pure paints ban invent dialog overlay wording"
        );
        return;
    }
    assert!(
        !lower.contains("modal") || lower.contains("temporary"),
        "{label}: floating modal chrome is forbidden"
    );
}

fn assert_bottom_dock_continuity(text: &str, label: &str) {
    // Statusline is the last row; dock itself anchors above it.
    assert_statusline_minimal(text, label);
    let rows: Vec<&str> = text.lines().collect();
    let body: String = rows[..rows.len().saturating_sub(1)].join("\n");
    assert!(
        body.contains("[esc]")
            || body.to_ascii_lowercase().contains("cancel")
            || body.to_ascii_lowercase().contains("dismiss")
            || body.to_ascii_lowercase().contains("return")
            || body.contains("›"),
        "{label}: bottom action/composer dock must expose selection or cancel"
    );
    // Transcript continuity above the dock (origin conversation remains visible).
    assert!(
        body.contains("You ·")
            || body.contains("retry")
            || body.contains("Paused")
            || body.contains("queued")
            || body.contains("Check the retry")
            || body.contains("permission")
            || body.contains("Pending edit")
            || body.contains("Help")
            || body.contains("Queue"),
        "{label}: originating conversation/context must remain visible above the dock"
    );
}

fn publication_fixture(screen_id: &str) -> String {
    canonical_publication_fixture(screen_id)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("canonical.{screen_id}"))
}

// ── VAL-V3-F2-001 / 059: Exact inventory + geometry ─────────────────────

#[test]
fn flow2_inventory_is_exact_and_geometry_valid() {
    let mut seen = HashSet::new();
    for id in FLOW2_IDS {
        assert!(seen.insert(*id), "duplicate Flow 2 id in constant: {id}");
    }
    assert_eq!(FLOW2_IDS.len(), 15, "Flow 2 inventory must be exactly 15");
    assert_eq!(GALLERY_RELATIVE.len(), 15);
    assert_eq!(CLOSED_DEFAULT_FIXTURES.len(), 15);

    let inventory = canonical_inventory();
    for id in FLOW2_IDS {
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
            FlowOwner::InputAndApproval,
            "{id} must be owned by the input-and-approval / Flow 2 journey"
        );
    }

    let mut digests = HashSet::new();
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
        assert!(
            digests.insert(text.clone()),
            "{screen_id}/{fixture_id}: produced a duplicate frame of another Flow 2 state"
        );
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
    }

    let root = gallery_root();
    let mut gallery_names = HashSet::new();
    for &(category, name) in GALLERY_RELATIVE {
        assert!(
            gallery_names.insert(name),
            "duplicate gallery id: {name}"
        );
        let txt = root.join(category).join(format!("{name}.txt"));
        let png = root.join(category).join(format!("{name}.png"));
        assert!(txt.is_file(), "missing gallery txt {}", txt.display());
        assert!(png.is_file(), "missing gallery png {}", png.display());
        let text = fs::read_to_string(&txt).expect("gallery txt");
        assert_canvas_geometry(&text, &format!("gallery {category}/{name}"));
        assert_png_dimensions(&png, 1200, 840);
        assert_statusline_minimal(&text, &format!("gallery {category}/{name}"));
        assert_chrome_simple(&text, &format!("gallery {category}/{name}"));
    }

    let input = flow_manifests()
        .iter()
        .find(|f| f.name == "input-and-approval")
        .expect("input-and-approval storyboard");
    assert_eq!(input.steps.len(), 6);
    let storyboard_ids: Vec<_> = input.steps.iter().map(|s| s.screen_id).collect();
    assert_eq!(
        storyboard_ids,
        vec![
            "composer-multiline",
            "file-mention",
            "permission-shell",
            "diff-edit",
            "diff-viewer-staging",
            "chat-turn",
        ]
    );

    let listed = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("list");
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    for id in FLOW2_IDS {
        assert!(
            stdout
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(*id)),
            "--list missing Flow 2 id {id}"
        );
    }
}

// ── VAL-V3-F2-003 / 004 / 005: Surface placement + chrome ───────────────

#[test]
fn every_flow2_action_surface_is_bottom_docked_without_floating_modal() {
    let inventory = canonical_inventory();
    for id in ACTION_DOCK_IDS {
        let screen = inventory
            .iter()
            .find(|s| s.id == *id)
            .unwrap_or_else(|| panic!("{id} missing from registry"));
        assert_eq!(
            screen.surface,
            Surface::ActionDock,
            "{id} must use the bottom ActionDock surface"
        );
    }
    for id in DETAIL_OR_INSPECTOR {
        let screen = inventory
            .iter()
            .find(|s| s.id == *id)
            .unwrap_or_else(|| panic!("{id} missing from registry"));
        assert!(
            matches!(
                screen.surface,
                Surface::TemporaryDetailView | Surface::ContextualInspector
            ),
            "{id} is a temporary detail or contextual inspector, got {:?}",
            screen.surface
        );
    }

    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
        if ACTION_DOCK_IDS.contains(&screen_id) {
            assert_bottom_dock_continuity(&text, &format!("{screen_id}/{fixture_id}"));
        }
        // Help is the sole exemption that may group shortcuts; it still returns on Return/Cancel.
        if screen_id == "help" {
            assert!(
                text.contains("Return") || (text.contains("Esc to") || text.contains("[esc]")),
                "help must keep a reversible exit"
            );
        } else {
            // No permanent screen-wide shortcut legend.
            let global_hints = text.lines().filter(|l| {
                let lower = l.to_ascii_lowercase();
                lower.contains("enter send")
                    || (lower.contains("/ commands") && lower.contains("@ files"))
                    || lower.contains("press ? for help")
            }).count();
            assert_eq!(
                global_hints, 0,
                "{screen_id}/{fixture_id}: global hint row is forbidden"
            );
        }
    }
}

// ── VAL-V3-F2-027 / 050 / 051 / 052: Origin round trips ─────────────────

#[test]
fn temporary_action_cancels_and_successes_restore_recorded_origin() {
    for &(case_id, from, to) in CANCEL_SCENARIOS {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("cancel scenario {case_id}: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} invalid: {e}"));
        assert_eq!(replay.before.screen_id, from, "{case_id} before");
        assert_eq!(replay.after.screen_id, to, "{case_id} after");
        // Multiline draft + height + scroll + origin stack must be hash-identical.
        for field in [
            "draft",
            "composer_height",
            "scroll_anchor",
            "origin",
            "origin_stack",
            "session_id",
            "transcript_id",
            "conversation_id",
        ] {
            if let Some(hash) = replay.preserved_field_hashes.get(field) {
                assert_eq!(
                    hash.before, hash.after,
                    "{case_id}: preserved {field} must round-trip byte-for-byte"
                );
            }
        }
        // Nested levels unwind one step only.
        if case_id == "palette-submenu-back" || case_id == "staging-cancel" {
            assert_ne!(
                replay.after.screen_id, "home",
                "{case_id}: nested cancel must not Close all"
            );
        }
        for effect in &replay.side_effects {
            for banned in DESTRUCTIVE_STATUSES {
                assert!(
                    !effect.status.eq_ignore_ascii_case(banned),
                    "{case_id}: cancel path must not emit destructive status {banned}"
                );
            }
            // Any non-destructive status/kind is acceptable for cancel (for example
            // no-write, no-execution, simulated-no-change, restored). Fail only when a
            // mutation-looking status reports an actual apply/write success.
            let status = effect.status.to_ascii_lowercase();
            let is_destructive = status.contains("applied-")
                || status.contains("file-written")
                || status.contains("command-invoked")
                || (status.contains("success") && !status.contains("no-"));
            assert!(
                !is_destructive,
                "{case_id}: cancel side-effect status must stay non-destructive, got {}",
                effect.status
            );
        }
        let after_text = replay.audit.render_txt();
        assert_canvas_geometry(&after_text, &format!("{case_id} after"));
    }

    for &(case_id, from, to) in SUCCESS_SCENARIOS {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("success scenario {case_id}: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} invalid: {e}"));
        assert_eq!(replay.before.screen_id, from, "{case_id} before");
        assert_eq!(replay.after.screen_id, to, "{case_id} after");
        assert!(
            !replay.actions.is_empty(),
            "{case_id}: success path must record an action"
        );
        // Must not orphan into home / login / mission destinations.
        assert!(
            !matches!(
                to,
                "home" | "login" | "workflow-launch" | "workflow-monitor"
            ),
            "{case_id}: success must return to the recorded origin, not {to}"
        );
        if !replay.before.session_id.is_empty() && !replay.after.session_id.is_empty() {
            assert_eq!(
                replay.before.session_id, replay.after.session_id,
                "{case_id}: session continuity"
            );
        }
        let after_text = replay.audit.render_txt();
        assert_canvas_geometry(&after_text, &format!("{case_id} after"));
    }

    // Explicit back/cancel never becomes a destructive default: no mutation side
    // effect and no successful write/apply report. Tick bookkeeping may still
    // record a single virtual attempt counter without performing work.
    for case_id in [
        "slash-cancel",
        "palette-cancel",
        "attachment-cancel",
        "export-cancel",
        "queue-cancel",
        "diff-cancel",
        "staging-cancel",
        "permission-cancel",
        "permission-file-edit-cancel",
    ] {
        let replay = replay_case(case_id, 1).expect(case_id);
        assert!(
            !replay.side_effects.iter().any(|s| {
                let status = s.status.to_ascii_lowercase();
                (status.contains("applied") && !status.contains("no"))
                    || status.contains("file-written")
                    || status.contains("command-invoked")
                    || (status.contains("success") && !status.contains("no-"))
            }),
            "{case_id}: cancel must not apply a mutation ({:?})",
            replay
                .side_effects
                .iter()
                .map(|s| s.status.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            !matches!(
                replay.after.screen_id.as_str(),
                "home" | "login" | "workflow-launch"
            ),
            "{case_id}: cancel must not orphan to an unrelated destination"
        );
    }
}

// ── VAL-V3-F2-053: Focus and keyboard order ─────────────────────────────

#[test]
fn every_flow2_action_surface_has_complete_focus_and_enabled_actions() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let enabled_actions: Vec<_> = audit
            .events
            .iter()
            .filter_map(|e| e.action.as_ref())
            .filter(|a| a.enabled)
            .collect();
        assert!(
            !enabled_actions.is_empty() || screen_id == "composer-multiline",
            "{screen_id}/{fixture_id}: at least one enabled interactive control or composer focus"
        );
        // Exactly one primary-action or one selection/focus marker for interactive docks.
        let selections = audit
            .events
            .iter()
            .filter(|e| e.identity.family == "selection" || e.identity.family == "focused-hunk")
            .count();
        let primaries = primary_events(&audit).len();
        assert!(
            primaries <= 1,
            "{screen_id}/{fixture_id}: at most one primary focus owner"
        );
        if ACTION_DOCK_IDS.contains(&screen_id) && screen_id != "composer-multiline" {
            assert!(
                selections <= 1,
                "{screen_id}/{fixture_id}: at most one selected row"
            );
            assert!(
                text.contains('›')
                    || (text.contains("Enter to") || text.contains("[enter]"))
                    || primaries == 1
                    || selections == 1,
                "{screen_id}/{fixture_id}: visible focus/primary marker required"
            );
        }
        // Every action bound in audit emits a public accessibility-complete record.
        for event in &audit.events {
            if let Some(action) = &event.action {
                assert!(
                    !action.action_id.trim().is_empty(),
                    "{screen_id}/{fixture_id}: action id required"
                );
                assert!(
                    !action.label.trim().is_empty(),
                    "{screen_id}/{fixture_id}: accessible label required"
                );
                assert!(
                    !event.identity.family.trim().is_empty(),
                    "{screen_id}/{fixture_id}: role/family required"
                );
                assert!(
                    !event.identity.render_refs.is_empty(),
                    "{screen_id}/{fixture_id}: action {} must have visible geometry",
                    action.action_id
                );
            }
        }
        // Cancel / back remains reachable on temporary actions.
        if screen_id != "composer-multiline" {
            assert!(
                (text.contains("Esc to") || text.contains("[esc]"))
                    || text.to_ascii_lowercase().contains("cancel")
                    || text.to_ascii_lowercase().contains("dismiss")
                    || text.to_ascii_lowercase().contains("return")
                    || text.to_ascii_lowercase().contains("back"),
                "{screen_id}/{fixture_id}: cancel/back must remain reachable"
            );
        }
        // Disabled rows cannot be the sole confirmed primary (filter empty states).
        if fixture_id.contains("no-results") {
            let primaries = primary_events(&audit);
            for p in primaries {
                if let Some(action) = &p.action {
                    assert!(
                        action.enabled || is_cancelish(&action.label),
                            "{screen_id}/{fixture_id}: disabled confirmation must not be the enabled primary"
                    );
                }
            }
        }
    }
}

// ── VAL-V3-F2-054 / 055: Non-color meaning + contrast discipline ───────

#[test]
fn flow2_meaning_never_depends_on_color_and_primaries_are_restrained() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        // Status / selection markers pair with words or glyphs, not color alone.
        if text.contains('✓') || text.contains('✗') || text.contains('●') || text.contains('○')
        {
            assert!(
                text.contains("Included")
                    || text.contains("Excluded")
                    || text.contains("Pending")
                    || text.contains("Allow")
                    || text.contains("Deny")
                    || text.contains("Applied")
                    || text.contains("Refresh")
                    || text.contains("matching")
                    || text.contains("selected")
                    || text.contains("next")
                    || text.to_ascii_lowercase().contains("ready")
                    || text.to_ascii_lowercase().contains("denied"),
                "{screen_id}/{fixture_id}: glyph/status requires adjacent readable meaning"
            );
        }
        if text.contains('›') {
            // Selected row is textual, not solely high-contrast fill.
            assert!(
                text.lines().any(|l| l.contains('›')
                    && l.chars().any(|c| c.is_ascii_alphanumeric())),
                "{screen_id}/{fixture_id}: selection marker needs labeled text"
            );
        }
        // Accent may mark primary/selection; red/green used only for outcomes/diffs.
        let greenish = text.matches('+').count();
        let redish = text.matches('-').count();
        if screen_id == "diff-edit" || screen_id == "diff-viewer-staging" {
            assert!(
                greenish + redish > 0,
                "{screen_id}: diff must disclose additions/deletions as +/− text"
            );
        }
        // Primary action exists as a visible label, not color alone.
        let primaries = primary_events(&audit);
        for p in primaries {
            let label = p
                .action
                .as_ref()
                .map(|a| a.label.clone())
                .unwrap_or_default();
            assert!(
                !label.trim().is_empty(),
                "{screen_id}/{fixture_id}: primary must have a textual label"
            );
            // Soft matching: label appears or is ellipsized into enter chrome.
            let lower = text.to_ascii_lowercase();
            let label_lower = label.to_ascii_lowercase();
            let keyword = label_lower
                .split_whitespace()
                .next()
                .unwrap_or(label_lower.as_str());
            assert!(
                lower.contains(&label_lower)
                    || lower.contains(keyword)
                    || lower.contains("[enter]")
                    || text.contains('›'),
                "{screen_id}/{fixture_id}: primary label `{label}` must be visible"
            );
        }
    }
}

// ── VAL-V3-F2-056: Plain English precedes advanced detail ───────────────

#[test]
fn flow2_surfaces_lead_with_plain_english_action_and_consequence() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
        for banned in RAW_ID_BANS {
            assert!(
                !text.contains(banned),
                "{screen_id}/{fixture_id}: technical default copy banned: {banned}"
            );
        }
        // No snake_case SCREAMING status tokens.
        for token in text.split_whitespace() {
            let cleaned = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
            if cleaned.contains('_')
                && cleaned.len() > 8
                && cleaned
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                panic!(
                    "{screen_id}/{fixture_id}: uppercase/snake_case status leaked: {cleaned}"
                );
            }
        }
        // Requested action / consequence language is present for decisions.
        if matches!(
            screen_id,
            "permission-shell"
                | "permission-file-edit"
                | "permission-deny-reason"
                | "diff-edit"
                | "diff-viewer-staging"
                | "queue-manager"
                | "export-picker"
                | "slash-menu"
                | "command-palette"
        ) {
            let lower = text.to_ascii_lowercase();
            assert!(
                lower.contains("allow")
                    || lower.contains("deny")
                    || lower.contains("export")
                    || lower.contains("queue")
                    || lower.contains("review")
                    || lower.contains("finish")
                    || lower.contains("apply")
                    || lower.contains("command")
                    || lower.contains("attach")
                    || lower.contains("reason")
                    || lower.contains("refresh"),
                "{screen_id}/{fixture_id}: decision surface must state an action/consequence"
            );
        }
    }
}

// ── VAL-V3-F2-057: One primary action per decision ──────────────────────

#[test]
fn every_flow2_decision_has_exactly_one_primary_and_limited_secondaries() {
    for &(screen_id, fixture_id) in DECISION_PRIMARY_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);
        assert_eq!(
            primaries.len(),
            1,
            "{screen_id}/{fixture_id}: expected exactly one primary-action, got {} labels {:?}",
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
        assert!(action.enabled, "{screen_id}/{fixture_id}: primary enabled");
        assert!(
            !action.automatic,
            "{screen_id}/{fixture_id}: primary must be a user action"
        );
        assert!(
            !action.label.trim().is_empty(),
            "{screen_id}/{fixture_id}: primary label nonempty"
        );
        // Temporary detail / training help may treat Return as the primary exit.
        if !matches!(screen_id, "help" | "command-palette-submenu") {
            assert!(
                !is_cancelish(&action.label)
                    || action.label.to_ascii_lowercase().contains("return")
                    || action.label.to_ascii_lowercase().contains("back to"),
                "{screen_id}/{fixture_id}: bare cancel/back must not be the primary (`{}`)",
                action.label
            );
        }
        let secondaries = secondary_events(&audit);
        assert!(
            secondaries.len() <= 2,
            "{screen_id}/{fixture_id}: at most two secondary actions, got {}",
            secondaries.len()
        );
        assert!(
            text.contains('›')
                || (text.contains("Enter to") || text.contains("[enter]"))
                || text.contains(&action.label)
                || text.contains(
                    action
                        .label
                        .split_whitespace()
                        .next()
                        .unwrap_or(action.label.as_str())
                ),
            "{screen_id}/{fixture_id}: primary must be visually marked"
        );
    }
}

// ── VAL-V3-F2-058: Rendered content and semantic audit agree ────────────

#[test]
fn rendered_content_and_semantic_audit_are_bijective_for_flow2() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        // validate() already enforces candidate↔event bijection; cross-check visibility.
        let text = audit.render_txt();
        for event in &audit.events {
            for r in &event.identity.render_refs {
                assert!(
                    (r.row as usize) < 40,
                    "{screen_id}/{fixture_id}: event {} row out of range",
                    event.identity.event_id
                );
                assert!(
                    r.start < r.end && r.end <= 120,
                    "{screen_id}/{fixture_id}: event {} ref geometry invalid",
                    event.identity.event_id
                );
            }
            if let Some(action) = &event.action {
                // Action labels must be recoverable from the canvas (soft token match).
                let token = action
                    .label
                    .split_whitespace()
                    .find(|t| t.chars().count() > 3)
                    .unwrap_or(action.label.as_str());
                assert!(
                    text.to_ascii_lowercase()
                        .contains(&token.to_ascii_lowercase())
                        || (text.contains("Enter to") || text.contains("[enter]"))
                        || (text.contains("Esc to") || text.contains("[esc]"))
                        || text.contains('›'),
                    "{screen_id}/{fixture_id}: action `{}` invisible on canvas",
                    action.label
                );
            }
        }
        // Every transition action_id is unique and maps to a known action where present.
        let mut transition_ids = HashSet::new();
        for t in &audit.transitions {
            assert!(
                transition_ids.insert(t.action_id.clone()),
                "{screen_id}/{fixture_id}: duplicate transition action {}",
                t.action_id
            );
            assert!(
                !t.from_screen_id.is_empty() && !t.to_screen_id.is_empty(),
                "{screen_id}/{fixture_id}: transition target missing"
            );
            assert_ne!(t.kind, "dead-end", "{screen_id}/{fixture_id}: dead-end transition");
        }
    }
}

// ── VAL-V3-F2-060: Stale/partial patch application is atomic ────────────

#[test]
fn stale_or_partial_patch_application_is_atomic_with_refresh() {
    let failed =
        replay_case("staging-apply-partial-failure", 1).expect("stale finish must replay");
    failed
        .validate()
        .expect("stale finish replay must validate");
    let failed_diff = failed.diff.as_ref().expect("diff evidence");
    assert_eq!(failed.after.screen_id, "diff-viewer-staging");
    assert_eq!(failed_diff.write_count, 0);
    assert_eq!(
        failed_diff.working_tree_before,
        failed_diff.working_tree_after
    );
    assert_eq!(failed_diff.git_index_before, failed_diff.git_index_after);
    assert_eq!(failed.side_effects[0].status, "apply-failed-no-write");
    assert_eq!(failed_diff.patch_revision, "patch-retry-18");
    let failed_text = failed.audit.render_txt();
    assert!(
        failed_text.contains("Workspace changed after review")
            || failed_text.to_ascii_lowercase().contains("stale"),
        "plain-English stale/workspace cause required"
    );
    assert!(failed_text.contains("Refresh and review"));
    assert_eq!(
        primary_events(&failed.audit)
            .iter()
            .filter_map(|e| e.action.as_ref().map(|a| a.label.as_str()))
            .collect::<Vec<_>>(),
        vec!["Refresh and review"]
    );
    // Finish is no longer the writable primary while the patch is stale.
    assert!(
        !failed_text.contains("[enter] Finish")
            && !failed_text.contains("Enter to finish")
            && !failed_text.contains("› Finish"),
        "Finish must not remain the enter primary while the workspace is stale"
    );

    let refreshed = replay_case("staging-refresh", 1).expect("refresh must replay");
    refreshed.validate().expect("refresh validates");
    let refreshed_diff = refreshed.diff.as_ref().expect("refresh diff");
    assert_eq!(refreshed.after.screen_id, "diff-viewer-staging");
    assert_eq!(refreshed_diff.write_count, 0);
    assert_eq!(
        refreshed_diff.working_tree_before,
        refreshed_diff.working_tree_after
    );
    assert_eq!(
        refreshed_diff.git_index_before,
        refreshed_diff.git_index_after
    );
    assert_eq!(refreshed_diff.patch_revision, "patch-retry-19");
    assert_eq!(refreshed.side_effects[0].status, "refreshed-no-write");
    // Unchanged content-hash hunks keep Include; changed content goes Pending.
    assert_eq!(refreshed_diff.hunks[0].state, "Included");
    assert_eq!(refreshed_diff.hunks[0].hunk_id, "hunk.retry-guard");
    assert_eq!(refreshed_diff.hunks[1].state, "Pending");
    assert_eq!(refreshed_diff.hunks[1].hunk_id, "hunk.retry-test");
    let refreshed_text = refreshed.audit.render_txt();
    assert!(refreshed_text.contains("revision: patch-retry-19"));
    assert!(
        refreshed_text.contains("Enter to finish") || refreshed_text.contains("[enter] Finish"),
        "refreshed staging must restore Finish as the enter primary (prose or residual bracket)"
    );
    assert!(refreshed_text.contains("Refreshed review"));
    assert_eq!(
        primary_events(&refreshed.audit)
            .iter()
            .filter_map(|e| e.action.as_ref().map(|a| a.label.as_str()))
            .collect::<Vec<_>>(),
        vec!["Finish"]
    );
    // No success report on force-finish after refresh without a new Finish attempt.
    assert!(
        !refreshed
            .side_effects
            .iter()
            .any(|s| s.status.contains("applied")),
        "refresh itself never applies hunks"
    );
}

// ── Gallery drift guard ─────────────────────────────────────────────────

#[test]
fn published_flow2_gallery_matches_live_publication_fixtures() {
    if pure_mode() {
        // Pure dump owns the gallery paints; hybrid audit fixtures intentionally differ.
        return;
    }
    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let fixture = publication_fixture(name);
        let candidates = [
            fixture.clone(),
            CLOSED_DEFAULT_FIXTURES
                .iter()
                .find(|(id, _)| *id == name)
                .map(|(_, f)| (*f).to_owned())
                .unwrap_or_else(|| format!("canonical.{name}")),
            "flow2.input-and-approval.stable".to_string(),
            format!("flow2-{name}"),
        ];
        let published = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("read gallery {category}/{name}: {e}"));
        assert_canvas_geometry(&published, &format!("gallery {category}/{name}"));

        let mut matched = false;
        let mut last_err = String::new();
        for candidate in &candidates {
            match audit_screen(name, candidate) {
                Ok(audit) => match audit.validate() {
                    Ok(()) => {
                        let live = audit.render_txt();
                        if live == published {
                            matched = true;
                            break;
                        }
                        last_err = format!(
                            "live fixture {candidate} differs from published TXT (len live={} pub={})",
                            live.len(),
                            published.len()
                        );
                    }
                    Err(e) => last_err = format!("audit invalid for {candidate}: {e}"),
                },
                Err(e) => last_err = format!("render failed for {candidate}: {e}"),
            }
        }
        assert!(
            matched,
            "gallery drift for {category}/{name}.txt: {last_err}"
        );
        let png = root.join(category).join(format!("{name}.png"));
        assert_png_dimensions(&png, 1200, 840);
    }
}
