//! Isolated, fail-closed validator fixtures for the v3 harness.
//!
//! Fixtures construct invalid evidence only below a disposable temporary root.
//! They never render or mutate canonical, gate, or frozen-reference artifacts.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::audit::{
    audit_screen, validate_visible_text, validate_vision_hash, AuditDocument, SemanticCandidate,
    SemanticEvent,
};
use crate::chrome::components::{
    validate_key_placement, validate_primary_actions, validate_status_copy,
};
use crate::chrome::layout::StatusLine;
use crate::chrome::tokens::{
    validate_glyphs, validate_text_safety, validate_token_hex, ColorToken,
};
use crate::flows::validate_flow_manifests;
use crate::registry::{
    canonical_inventory, validate_alias, validate_inventory, validate_surface_labels,
};
use crate::renderer::run_renderer_negative_fixture;

const CANONICAL_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
const FROZEN_MANIFEST: &str =
    "/projects/cortex-code/tools-throwaway/mockup-harness-v3/baselines/frozen-references.sha256";

macro_rules! fixture {
    ($id:literal, $validator:literal, $screen:literal, $identity:literal, $diagnostic:literal) => {
        FixtureSpec {
            id: $id,
            command: concat!("negative-fixture ", $id),
            validator: $validator,
            screen_id: $screen,
            offending_identity: $identity,
            expected_diagnostic: $diagnostic,
        }
    };
}

/// Public, stable fixture inventory. Each entry doubles as the negative-fixture
/// matrix row requested by the validation contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureSpec {
    pub id: &'static str,
    pub command: &'static str,
    pub validator: &'static str,
    pub screen_id: &'static str,
    pub offending_identity: &'static str,
    pub expected_diagnostic: &'static str,
}

pub const REQUIRED_FIXTURE_IDS: &[&str] = &[
    "orphan-candidate",
    "duplicate-candidate",
    "unknown-candidate",
    "orphan-event",
    "duplicate-event",
    "unknown-event",
    "missing-event",
    "duplicate-activity-owner",
    "inaccessible-action",
    "unknown-action",
    "duplicate-action",
    "dead-end-transition",
    "unknown-transition",
    "missing-surface",
    "duplicate-surface",
    "unknown-surface",
    "token-hex-mismatch",
    "misplaced-key",
    "color-only-status",
    "decorative-glyph",
    "text-width-control-hazard",
    "raw-model-id",
    "leaked-secret",
    "hidden-primary-action",
    "duplicate-primary-action",
    "automatic-wait-primary-action",
    "inventory-without-renderer",
    "renderer-outside-inventory",
    "duplicate-canonical-id",
    "unknown-flow-id",
    "missing-flow-metadata",
    "direct-edge-mismatch",
    "canonical-flow-membership-mismatch",
    "alias-normalization-mismatch",
    "stale-vision-hash",
    "unsafe-cleanup",
    "publication-failure",
    "permission-page-gap",
    "permission-page-overlap",
    "permission-page-wrong-offset",
    "permission-page-fabricated-notation",
];

const FIXTURE_MANIFEST: &[FixtureSpec] = &[
    fixture!(
        "orphan-candidate",
        "audit",
        "permission-shell",
        "candidate.orphan",
        "semantic identity is not bound"
    ),
    fixture!(
        "duplicate-candidate",
        "audit",
        "permission-shell",
        "candidate.surface.permission-shell",
        "duplicate semantic candidate identity"
    ),
    fixture!(
        "unknown-candidate",
        "audit",
        "permission-shell",
        "candidate.unknown",
        "unknown candidate"
    ),
    fixture!(
        "orphan-event",
        "audit",
        "permission-shell",
        "event.orphan",
        "orphan semantic event"
    ),
    fixture!(
        "duplicate-event",
        "audit",
        "permission-shell",
        "event.surface.permission-shell",
        "duplicate semantic event identity"
    ),
    fixture!(
        "unknown-event",
        "audit",
        "permission-shell",
        "event.unknown",
        "unknown semantic event"
    ),
    fixture!(
        "missing-event",
        "audit",
        "permission-shell",
        "candidate.statusline.main",
        "missing semantic event"
    ),
    fixture!(
        "duplicate-activity-owner",
        "audit",
        "permission-shell",
        "owner.same",
        "duplicate activity owner"
    ),
    fixture!(
        "inaccessible-action",
        "audit",
        "permission-shell",
        "action.permission.allow-once",
        "inaccessible action"
    ),
    fixture!(
        "unknown-action",
        "audit",
        "permission-shell",
        "action.unknown",
        "transition references an unknown action"
    ),
    fixture!(
        "duplicate-action",
        "audit",
        "permission-shell",
        "action.permission.allow-once",
        "duplicate action identity"
    ),
    fixture!(
        "dead-end-transition",
        "audit",
        "permission-shell",
        "action.permission.allow-once",
        "transition reaches a dead end"
    ),
    fixture!(
        "unknown-transition",
        "audit",
        "permission-shell",
        "not-a-screen",
        "unknown transition"
    ),
    fixture!(
        "missing-surface",
        "surface",
        "home",
        "surface.missing",
        "missing surface"
    ),
    fixture!(
        "duplicate-surface",
        "surface",
        "home",
        "conversation-shell",
        "duplicate surface"
    ),
    fixture!(
        "unknown-surface",
        "surface",
        "home",
        "floating-modal",
        "unknown surface"
    ),
    fixture!(
        "token-hex-mismatch",
        "tokens",
        "home",
        "accent:#000000",
        "token/hex mismatch"
    ),
    fixture!(
        "misplaced-key",
        "key-placement",
        "permission-shell",
        "key.global-help",
        "misplaced key"
    ),
    fixture!(
        "color-only-status",
        "status-copy",
        "tool-calls",
        "status.failed",
        "color-only status"
    ),
    fixture!(
        "decorative-glyph",
        "glyph-policy",
        "home",
        "✦",
        "unsupported glyph"
    ),
    fixture!(
        "text-width-control-hazard",
        "text-safety",
        "home",
        "control-escape",
        "text width/control hazard"
    ),
    fixture!(
        "raw-model-id",
        "statusline",
        "home",
        "anthropic/claude-opus",
        "human-readable display name"
    ),
    fixture!(
        "leaked-secret",
        "secret-scan",
        "home",
        "fixture-secret-value",
        "leaked secret"
    ),
    fixture!(
        "hidden-primary-action",
        "primary-action",
        "permission-shell",
        "primary-action",
        "hidden primary action"
    ),
    fixture!(
        "duplicate-primary-action",
        "primary-action",
        "permission-shell",
        "primary-action",
        "duplicate primary action"
    ),
    fixture!(
        "automatic-wait-primary-action",
        "primary-action",
        "rate-limit",
        "action.recovery.retry-window",
        "automatic wait exposes a primary action"
    ),
    fixture!(
        "inventory-without-renderer",
        "renderer-map",
        "home",
        "home",
        "implemented canonical ID lacks renderer"
    ),
    fixture!(
        "renderer-outside-inventory",
        "renderer-map",
        "home",
        "outside-the-contract",
        "renderer outside canonical inventory"
    ),
    fixture!(
        "duplicate-canonical-id",
        "registry",
        "home",
        "home",
        "duplicate canonical ID"
    ),
    fixture!(
        "unknown-flow-id",
        "flow-manifest",
        "daily-work",
        "not-a-canonical-state",
        "unknown canonical screen"
    ),
    fixture!(
        "missing-flow-metadata",
        "flow-manifest",
        "daily-work",
        "acceptance_checks",
        "missing acceptance checks"
    ),
    fixture!(
        "direct-edge-mismatch",
        "flow-manifest",
        "daily-work",
        "home",
        "undeclared direct edge"
    ),
    fixture!(
        "canonical-flow-membership-mismatch",
        "flow-manifest",
        "daily-work",
        "chat-turn",
        "canonical/flow membership mismatch"
    ),
    fixture!(
        "alias-normalization-mismatch",
        "alias",
        "home",
        "Home",
        "alias normalization mismatch"
    ),
    fixture!(
        "stale-vision-hash",
        "vision-evidence",
        "home",
        "vision-hash-old",
        "stale vision hash"
    ),
    fixture!(
        "unsafe-cleanup",
        "renderer-cleanup",
        "home",
        "outside-staging",
        "unsafe renderer cleanup path"
    ),
    fixture!(
        "publication-failure",
        "renderer-publication",
        "home",
        "published",
        "injected staged validation failure"
    ),
    fixture!(
        "permission-page-gap",
        "permission-paging",
        "permission-shell",
        "page-offset-45",
        "gap or overlap"
    ),
    fixture!(
        "permission-page-overlap",
        "permission-paging",
        "permission-shell",
        "page-offset-43",
        "gap or overlap"
    ),
    fixture!(
        "permission-page-wrong-offset",
        "permission-paging",
        "permission-shell",
        "previous-offset-0",
        "incorrect adjacent previous or next offsets"
    ),
    fixture!(
        "permission-page-fabricated-notation",
        "permission-paging",
        "permission-shell",
        "fabricated-page-notation",
        "does not reconstruct exact execution bytes"
    ),
];

/// Returns the complete matrix in deterministic contract order.
pub const fn fixture_manifest() -> &'static [FixtureSpec] {
    FIXTURE_MANIFEST
}

/// Drives one named negative case through the public harness CLI. Each case
/// uses a disposable fixture root, returns its production rejection unchanged,
/// and proves protected roots and cleanup fencing remain intact.
pub fn run_public_negative_fixture(id: &str) -> Result<(), String> {
    verify_fixture_manifest()?;
    let spec = FIXTURE_MANIFEST
        .iter()
        .find(|spec| spec.id == id)
        .ok_or_else(|| format!("unknown negative fixture: {id}"))?;
    crate::verify_frozen_references(Path::new("/projects/cortex-code"))?;
    let protected_before = protected_manifest()?;
    let fixture = FixtureRoot::new()?;
    let fixture_path = fixture.path.clone();
    let rejection = run_fixture(spec, &fixture_path);
    reset_fixture_root(&fixture_path)?;
    let fixture_root_listing = list_fixture_root(&fixture_path)?;
    if !fixture_root_listing.is_empty() {
        return Err(format!(
            "negative fixture root was not reset: {}",
            fixture_root_listing.join(", ")
        ));
    }
    drop(fixture);

    let protected_after = protected_manifest()?;
    crate::verify_frozen_references(Path::new("/projects/cortex-code"))?;
    if protected_before != protected_after {
        return Err("negative fixtures changed canonical, gate, or frozen manifests".to_owned());
    }
    rejected_fixture(spec, rejection)
}

fn run_fixture(spec: &FixtureSpec, root: &Path) -> Result<(), String> {
    match spec.id {
        "orphan-candidate" => audit_rejection(|document| {
            document.candidates[0].identity.screen_id = "not-the-rendered-screen".to_owned();
        }),
        "duplicate-candidate" => audit_rejection(|document| {
            document.candidates.push(document.candidates[0].clone());
        }),
        "unknown-candidate" | "orphan-event" => audit_rejection(|document| {
            let mut event = document.events[0].clone();
            event.identity.candidate_id = "candidate.unknown".to_owned();
            event.identity.event_id = if spec.id == "orphan-event" {
                "event.orphan".to_owned()
            } else {
                "event.unknown-candidate".to_owned()
            };
            document.events.push(event);
        }),
        "duplicate-event" => audit_rejection(|document| {
            document.events.push(document.events[0].clone());
        }),
        "unknown-event" => audit_rejection(|document| {
            document.events[0].identity.event_id = "event.unknown".to_owned();
        }),
        "missing-event" => audit_rejection(|document| {
            document.events.pop();
        }),
        "duplicate-activity-owner" => audit_rejection(add_duplicate_activity_owner),
        "inaccessible-action" => audit_rejection(|document| {
            document
                .events
                .iter_mut()
                .find_map(|event| event.action.as_mut())
                .expect("fixture action must exist")
                .enabled = false;
        }),
        "unknown-action" => audit_rejection(|document| {
            document.transitions[0].action_id = "action.unknown".to_owned();
        }),
        "duplicate-action" => audit_rejection(|document| {
            let action = document
                .events
                .iter()
                .find_map(|event| event.action.clone())
                .expect("fixture action must exist");
            document
                .events
                .iter_mut()
                .find(|event| event.action.is_none())
                .expect("fixture target event must exist")
                .action = Some(action);
        }),
        "dead-end-transition" => audit_rejection(|document| {
            document.transitions[0].kind = "dead-end".to_owned();
        }),
        "unknown-transition" => audit_rejection(|document| {
            document.transitions[0].to_screen_id = "not-a-screen".to_owned();
        }),
        "missing-surface" => validate_surface_labels(&[]),
        "duplicate-surface" => {
            validate_surface_labels(&["conversation-shell", "conversation-shell"])
        }
        "unknown-surface" => validate_surface_labels(&["floating-modal"]),
        "token-hex-mismatch" => validate_token_hex(ColorToken::Accent, "#000000"),
        "misplaced-key" => validate_key_placement("key.global-help", false),
        "color-only-status" => validate_status_copy("status.failed", ""),
        "decorative-glyph" => validate_glyphs("✦"),
        "text-width-control-hazard" => validate_text_safety(&format!("{}\u{1b}", "x".repeat(120))),
        "raw-model-id" => StatusLine::new("~/project", "main", "anthropic/claude-opus").map(|_| ()),
        "leaked-secret" => validate_visible_text("token fixture-secret-value"),
        "hidden-primary-action" => validate_primary_actions(0, false),
        "duplicate-primary-action" => validate_primary_actions(2, false),
        "automatic-wait-primary-action" => validate_primary_actions(1, true),
        "inventory-without-renderer"
        | "renderer-outside-inventory"
        | "unsafe-cleanup"
        | "publication-failure" => run_renderer_negative_fixture(spec.id, root),
        "permission-page-gap"
        | "permission-page-overlap"
        | "permission-page-wrong-offset"
        | "permission-page-fabricated-notation" => run_permission_paging_fixture(spec.id),
        "duplicate-canonical-id" => {
            let mut inventory = canonical_inventory().to_vec();
            inventory[1].id = "home";
            inventory[1].flow = crate::registry::FlowOwner::DailyWork;
            validate_inventory(&inventory)
        }
        "unknown-flow-id" => {
            let mut flows = crate::flows::flow_manifests().to_vec();
            flows[0].steps[0].screen_id = "not-a-canonical-state";
            validate_flow_manifests(&flows, canonical_inventory())
        }
        "missing-flow-metadata" => {
            let mut flows = crate::flows::flow_manifests().to_vec();
            flows[0].steps[0].acceptance_checks = &[];
            validate_flow_manifests(&flows, canonical_inventory())
        }
        "direct-edge-mismatch" => {
            let mut flows = crate::flows::flow_manifests().to_vec();
            flows[0].steps[0].expected_next_frame = "home";
            validate_flow_manifests(&flows, canonical_inventory())
        }
        "canonical-flow-membership-mismatch" => {
            let mut flows = crate::flows::flow_manifests().to_vec();
            flows[0].steps[0].screen_id = "chat-turn";
            validate_flow_manifests(&flows, canonical_inventory())
        }
        "alias-normalization-mismatch" => validate_alias("Home", "home"),
        "stale-vision-hash" => validate_vision_hash("vision-hash-old", "vision-hash-current"),
        other => Err(format!("unknown negative fixture: {other}")),
    }
}

fn run_permission_paging_fixture(id: &str) -> Result<(), String> {
    let mut pages = crate::scenario::canonical_permission_pages();
    match id {
        "permission-page-gap" => pages[1].byte_offset = 45,
        "permission-page-overlap" => pages[1].byte_offset = 43,
        "permission-page-wrong-offset" => pages[2].previous_offset = Some(0),
        "permission-page-fabricated-notation" => pages[1].page_notation = "fabricated".to_owned(),
        other => return Err(format!("unknown permission paging fixture: {other}")),
    }
    crate::scenario::validate_permission_traversal(&pages)
}

fn audit_rejection(mutate: impl FnOnce(&mut AuditDocument)) -> Result<(), String> {
    let mut document = audit_screen("permission-shell", "negative-fixture")?;
    mutate(&mut document);
    document.validate()
}

fn add_duplicate_activity_owner(document: &mut AuditDocument) {
    let mut first_candidate = document.candidates[0].clone();
    first_candidate.identity.candidate_id = "candidate.activity.first".to_owned();
    first_candidate.identity.event_id = "event.activity.first".to_owned();
    first_candidate.identity.family = "activity".to_owned();
    first_candidate.identity.subject_id = "owner.same".to_owned();
    let mut first_event = SemanticEvent {
        identity: first_candidate.identity.clone(),
        action: None,
    };
    first_event.identity.event_id = "event.activity.first".to_owned();

    let mut second_candidate = first_candidate.clone();
    second_candidate.identity.candidate_id = "candidate.activity.second".to_owned();
    second_candidate.identity.event_id = "event.activity.second".to_owned();
    let second_event = SemanticEvent {
        identity: second_candidate.identity.clone(),
        action: None,
    };

    document.candidates.extend([
        SemanticCandidate {
            identity: first_candidate.identity,
        },
        second_candidate,
    ]);
    document.events.extend([first_event, second_event]);
}

fn rejected_fixture(spec: &FixtureSpec, rejected: Result<(), String>) -> Result<(), String> {
    let error = rejected.err().ok_or_else(|| {
        format!(
            "fixture {} unexpectedly accepted its invalid input",
            spec.id
        )
    })?;
    if !error.contains(spec.expected_diagnostic) {
        return Err(format!(
            "fixture {} screen {} offending identity {} rejected with unexpected diagnostic: {}",
            spec.id, spec.screen_id, spec.offending_identity, error
        ));
    }
    Err(format!(
        "fixture {} screen {} offending identity {}: {}",
        spec.id, spec.screen_id, spec.offending_identity, error
    ))
}

fn verify_fixture_manifest() -> Result<(), String> {
    let ids = FIXTURE_MANIFEST
        .iter()
        .map(|fixture| fixture.id)
        .collect::<Vec<_>>();
    if ids != REQUIRED_FIXTURE_IDS {
        return Err("negative fixture manifest does not match required coverage order".to_owned());
    }
    let unique = ids.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != ids.len()
        || FIXTURE_MANIFEST.iter().any(|fixture| {
            fixture.command.is_empty()
                || fixture.validator.is_empty()
                || fixture.screen_id.is_empty()
                || fixture.offending_identity.is_empty()
                || fixture.expected_diagnostic.is_empty()
        })
    {
        return Err("negative fixture manifest is incomplete".to_owned());
    }
    Ok(())
}

fn protected_manifest() -> Result<String, String> {
    let mut entries = Vec::new();
    for root in [
        Path::new(CANONICAL_ROOT),
        Path::new(GATE_ROOT),
        Path::new(FROZEN_MANIFEST),
    ] {
        collect_manifest_entry(root, root, &mut entries)?;
    }
    entries.sort();
    Ok(entries.join("\n"))
}

fn collect_manifest_entry(
    root: &Path,
    path: &Path,
    entries: &mut Vec<String>,
) -> Result<(), String> {
    if path.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|error| format!("could not list protected root {}: {error}", path.display()))?
        {
            let entry =
                entry.map_err(|error| format!("could not read protected root entry: {error}"))?;
            collect_manifest_entry(root, &entry.path(), entries)?;
        }
        return Ok(());
    }
    if !path.is_file() {
        return Err(format!(
            "protected manifest path is not a regular file: {}",
            path.display()
        ));
    }
    let relative = path
        .strip_prefix(root.parent().unwrap_or(root))
        .unwrap_or(path)
        .display()
        .to_string();
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read protected manifest path {}: {error}",
            path.display()
        )
    })?;
    entries.push(format!("{relative}\t{}", crate::audit::sha256_hex(&bytes)));
    Ok(())
}

fn reset_fixture_root(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root)
        .map_err(|error| format!("could not reset fixture root {}: {error}", root.display()))?
    {
        let entry =
            entry.map_err(|error| format!("could not inspect fixture root entry: {error}"))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| format!("could not inspect fixture root type: {error}"))?
            .is_dir()
        {
            fs::remove_dir_all(&path).map_err(|error| {
                format!(
                    "could not reset fixture directory {}: {error}",
                    path.display()
                )
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| {
                format!("could not reset fixture file {}: {error}", path.display())
            })?;
        }
    }
    Ok(())
}

fn list_fixture_root(root: &Path) -> Result<Vec<String>, String> {
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("could not list fixture root {}: {error}", root.display()))?
        .map(|entry| {
            entry
                .map_err(|error| format!("could not inspect fixture root entry: {error}"))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

static FIXTURE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct FixtureRoot {
    path: PathBuf,
}

impl FixtureRoot {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "mockup-harness-v3-negative-fixtures-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|error| {
            format!(
                "could not create isolated fixture root {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { path })
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
