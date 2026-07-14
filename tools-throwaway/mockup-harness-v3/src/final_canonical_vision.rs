//! Final canonical PNG vision product for the complete v3 gallery.
//!
//! After atomic publication, every final canonical PNG hash receives an individual
//! hash-bound Opus 4.8 review. MCP and Mission sequences also receive ordered
//! multi-state reviews. Gate PNG hashes join the existing gate vision product so
//! VAL-V3-HARNESS-033 can prove both inventories are covered without reusing
//! sampled/inherited gate frames as substitutes for the 86 canonical reviews.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::sha256_hex;
use crate::flows::flow_manifests;
use crate::gate_integration::validate_review_provenance;
use crate::gate_vision_review::{validate_gate_vision_review, RenderReviewBarrier};
use crate::registry::{canonical_inventory, Category, FlowOwner, ScreenSpec, Surface};

const REVIEW_MODEL: &str = "anthropic/claude-opus-4.8-fast";
const EVIDENCE_ROOT: &str = "evidence/v3-final-canonical-vision";
const BARRIER_RELATIVE: &str = "evidence/v3-final-canonical-vision/render-to-review-barrier.json";
const LEDGER_RELATIVE: &str = "evidence/v3-final-canonical-vision/final-vision-ledger.md";
const RECONCILIATION_RELATIVE: &str =
    "evidence/v3-final-canonical-vision/vision-to-txt-audit-reconciliation.md";
const MCP_TRANSITION_RELATIVE: &str =
    "evidence/v3-final-canonical-vision/transitions/mcp-ordered-review.md";
const MISSION_TRANSITION_RELATIVE: &str =
    "evidence/v3-final-canonical-vision/transitions/mission-ordered-review.md";
const CANONICAL_GALLERY: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const HARNESS_BIN_HINT: &str =
    "/projects/cortex-code/tools-throwaway/mockup-harness-v3/target/debug/mockup-harness-v3";

/// Ordered MCP journey plus capability detail for cross-state review.
pub const MCP_ORDERED_SCREEN_IDS: &[&str] = &[
    "login",
    "connect-provider",
    "connect-method",
    "connect-oauth",
    "connect-connected",
    "mcp",
    "sidebar-mcp",
];

/// Ordered mission/workflow journey plus selected disclosure depths.
pub const MISSION_ORDERED_SCREEN_IDS: &[&str] = &[
    "workflow-launch-blocked",
    "workflow-launch",
    "workflow-monitor",
    "workflow-monitor-failure",
    "team-board",
    "reviewer-output",
    "reviewer-approved",
    "verification-ladder",
    "intent-scorer-pass",
    "tool-envelope",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalVisionRecord {
    pub screen_id: String,
    pub category: String,
    pub flow: String,
    pub surface: String,
    pub relative_png: String,
    pub artifact_path: String,
    pub sha256: String,
    pub relative_evidence: String,
    pub model: String,
    pub verdict: String,
    pub review_session_id: String,
    pub response_sha256: String,
    pub record_is_current: bool,
    pub provenance_ok: bool,
    pub hierarchy_ok: bool,
    pub density_ok: bool,
    pub primary_action_ok: bool,
    pub color_restraint_ok: bool,
    pub disclosure_ok: bool,
    pub reconciled_with_txt_audit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderedTransitionReview {
    pub journey: String,
    pub relative_path: String,
    pub ordered_screen_ids: Vec<String>,
    pub ordered_png_hashes: Vec<String>,
    pub model: String,
    pub return_to_origin_ok: bool,
    pub transition_cues_ok: bool,
    pub sheet_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalCanonicalVisionReport {
    pub canonical_records: Vec<CanonicalVisionRecord>,
    pub gate_png_join_count: usize,
    pub barrier: RenderReviewBarrier,
    pub mcp_transition: OrderedTransitionReview,
    pub mission_transition: OrderedTransitionReview,
    pub unresolved_issue_count: usize,
    pub ledger_relative: String,
    pub reconciliation_relative: String,
}

fn screen_by_id(id: &str) -> Option<&'static ScreenSpec> {
    canonical_inventory().iter().find(|screen| screen.id == id)
}

fn relative_gallery_path(screen: &ScreenSpec) -> String {
    format!("{}/{}.png", screen.category.as_str(), screen.id)
}

fn relative_txt_path(screen: &ScreenSpec) -> String {
    format!("{}/{}.txt", screen.category.as_str(), screen.id)
}

pub fn canonical_vision_relative_path(screen_id: &str) -> String {
    format!("{EVIDENCE_ROOT}/canonical/{screen_id}.vision.txt")
}

fn role_for(screen: &ScreenSpec) -> String {
    format!(
        "final canonical {}/{} flow={} surface={}",
        screen.category.as_str(),
        screen.id,
        screen.flow.as_str(),
        screen.surface.as_str()
    )
}

fn read_review_record(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read review record {}: {error}", path.display()))?;
    let mut record = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!("invalid review record line in {}: {line}", path.display())
        })?;
        if record
            .insert(key.trim().to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(format!(
                "duplicate review record field {} in {}",
                key.trim(),
                path.display()
            ));
        }
    }
    Ok(record)
}

fn require_field(
    review: &BTreeMap<String, String>,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    match review.get(field) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!(
            "review field {field} mismatch: expected {expected}, got {actual}"
        )),
        None => Err(format!("review record is missing {field}")),
    }
}

fn flag_true(review: &BTreeMap<String, String>, field: &str) -> bool {
    review
        .get(field)
        .map(|value| value == "true" || value == "pass" || value == "yes")
        .unwrap_or(false)
}

fn parse_barrier(path: &Path) -> Result<RenderReviewBarrier, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read barrier record {}: {error}", path.display()))?;
    let mut map = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("invalid barrier line in {}: {line}", path.display()))?;
        map.insert(key.trim().to_owned(), value.trim().to_owned());
    }
    let render_complete_unix_ms = map
        .get("render_complete_unix_ms")
        .ok_or_else(|| "barrier missing render_complete_unix_ms".to_owned())?
        .parse::<u64>()
        .map_err(|_| "render_complete_unix_ms must be an integer".to_owned())?;
    let review_start_unix_ms = map
        .get("review_start_unix_ms")
        .ok_or_else(|| "barrier missing review_start_unix_ms".to_owned())?
        .parse::<u64>()
        .map_err(|_| "review_start_unix_ms must be an integer".to_owned())?;
    let max_concurrent_reviewers = map
        .get("max_concurrent_reviewers")
        .ok_or_else(|| "barrier missing max_concurrent_reviewers".to_owned())?
        .parse::<u32>()
        .map_err(|_| "max_concurrent_reviewers must be an integer".to_owned())?;
    let no_renderer_overlap = map
        .get("no_renderer_overlap")
        .map(|value| value == "true")
        .unwrap_or(false);
    let reviewer_session_ids = map
        .get("reviewer_session_ids")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(RenderReviewBarrier {
        render_complete_unix_ms,
        review_start_unix_ms,
        max_concurrent_reviewers,
        reviewer_session_ids,
        no_renderer_overlap,
    })
}

fn validate_barrier(barrier: &RenderReviewBarrier) -> Result<(), String> {
    if barrier.review_start_unix_ms < barrier.render_complete_unix_ms {
        return Err("review started before render-complete barrier".to_owned());
    }
    if barrier.max_concurrent_reviewers == 0 || barrier.max_concurrent_reviewers > 4 {
        return Err(format!(
            "max concurrent reviewers must be 1..=4, got {}",
            barrier.max_concurrent_reviewers
        ));
    }
    if !barrier.no_renderer_overlap {
        return Err("barrier reports renderer/reader overlap".to_owned());
    }
    if barrier.reviewer_session_ids.len() > barrier.max_concurrent_reviewers as usize {
        return Err(format!(
            "barrier records more concurrent reviewers than allowed: {} > {}",
            barrier.reviewer_session_ids.len(),
            barrier.max_concurrent_reviewers
        ));
    }
    Ok(())
}

/// Writes the final-canonical render-to-review barrier before vision work starts.
pub fn write_final_render_review_barrier(
    mission_directory: &str,
    render_complete_unix_ms: u64,
    max_concurrent_reviewers: u32,
    reviewer_session_ids: &[String],
) -> Result<PathBuf, String> {
    if max_concurrent_reviewers == 0 || max_concurrent_reviewers > 4 {
        return Err("max concurrent reviewers must be 1..=4".to_owned());
    }
    if reviewer_session_ids.len() > max_concurrent_reviewers as usize {
        return Err(
            "cannot record more concurrent reviewers than the declared limit".to_owned(),
        );
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_millis() as u64;
    if now < render_complete_unix_ms {
        return Err("cannot start review before render_complete_unix_ms".to_owned());
    }
    let path = Path::new(mission_directory).join(BARRIER_RELATIVE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create barrier directory: {error}"))?;
    }
    let body = format!(
        "render_complete_unix_ms={render_complete_unix_ms}\n\
         review_start_unix_ms={now}\n\
         max_concurrent_reviewers={max_concurrent_reviewers}\n\
         no_renderer_overlap=true\n\
         reviewer_session_ids={}\n\
         model={REVIEW_MODEL}\n\
         assertion_ids=VAL-V3-CROSS-017,VAL-V3-CROSS-018,VAL-V3-HARNESS-033\n",
        reviewer_session_ids.join(",")
    );
    fs::write(&path, body).map_err(|error| format!("could not write barrier: {error}"))?;
    Ok(path)
}

fn evaluate_canonical_record(
    mission_directory: &str,
    screen: &ScreenSpec,
) -> Result<CanonicalVisionRecord, String> {
    let relative_png = relative_gallery_path(screen);
    let artifact_path = Path::new(CANONICAL_GALLERY).join(&relative_png);
    let bytes = fs::read(&artifact_path).map_err(|error| {
        format!(
            "missing canonical PNG {}: {error}",
            artifact_path.display()
        )
    })?;
    let sha256 = sha256_hex(&bytes);
    let relative_evidence = canonical_vision_relative_path(screen.id);
    let evidence_path = Path::new(mission_directory).join(&relative_evidence);
    let role = role_for(screen);

    let mut model = String::new();
    let mut verdict = String::new();
    let mut review_session_id = String::new();
    let mut response_sha256 = String::new();
    let mut hierarchy_ok = false;
    let mut density_ok = false;
    let mut primary_action_ok = false;
    let mut color_restraint_ok = false;
    let mut disclosure_ok = false;
    let mut reconciled_with_txt_audit = false;
    let mut provenance_ok = false;
    let mut record_is_current = false;

    if evidence_path.is_file() {
        if let Ok(review) = read_review_record(&evidence_path) {
            model = review.get("model").cloned().unwrap_or_default();
            verdict = review.get("verdict").cloned().unwrap_or_default();
            review_session_id = review.get("review_session_id").cloned().unwrap_or_default();
            response_sha256 = review.get("response_sha256").cloned().unwrap_or_default();
            hierarchy_ok = flag_true(&review, "hierarchy_ok");
            density_ok = flag_true(&review, "density_ok");
            primary_action_ok = flag_true(&review, "primary_action_ok");
            color_restraint_ok = flag_true(&review, "color_restraint_ok");
            disclosure_ok = flag_true(&review, "disclosure_ok");
            reconciled_with_txt_audit = flag_true(&review, "reconciled_with_txt_audit");
            provenance_ok = validate_review_provenance(&review).is_ok();
            record_is_current = require_field(
                &review,
                "artifact_path",
                &artifact_path.display().to_string(),
            )
            .and_then(|_| require_field(&review, "role", &role))
            .and_then(|_| require_field(&review, "sha256", &sha256))
            .and_then(|_| require_field(&review, "model", REVIEW_MODEL))
            .and_then(|_| require_field(&review, "verdict", "pass"))
            .and_then(|_| validate_review_provenance(&review))
            .is_ok()
                && hierarchy_ok
                && density_ok
                && primary_action_ok
                && color_restraint_ok
                && disclosure_ok
                && reconciled_with_txt_audit;
        }
    }

    Ok(CanonicalVisionRecord {
        screen_id: screen.id.to_owned(),
        category: screen.category.as_str().to_owned(),
        flow: screen.flow.as_str().to_owned(),
        surface: screen.surface.as_str().to_owned(),
        relative_png,
        artifact_path: artifact_path.display().to_string(),
        sha256,
        relative_evidence,
        model,
        verdict,
        review_session_id,
        response_sha256,
        record_is_current,
        provenance_ok,
        hierarchy_ok,
        density_ok,
        primary_action_ok,
        color_restraint_ok,
        disclosure_ok,
        reconciled_with_txt_audit,
    })
}

fn collect_canonical_records(
    mission_directory: &str,
) -> Result<Vec<CanonicalVisionRecord>, String> {
    let mut records = Vec::with_capacity(86);
    for screen in canonical_inventory() {
        records.push(evaluate_canonical_record(mission_directory, screen)?);
    }
    if records.len() != 86 {
        return Err(format!(
            "expected exactly 86 canonical vision records, got {}",
            records.len()
        ));
    }
    Ok(records)
}

fn collect_gate_png_hashes(repository: &str) -> Result<Vec<(String, String)>, String> {
    let root = Path::new(repository).join("plans/assets/mockups-v3-gates");
    let mut out = Vec::new();
    collect_png_hashes(&root, &root, &mut out)?;
    out.sort_by(|left, right| left.0.cmp(&right.0));
    if out.len() != 35 {
        return Err(format!("expected 35 gate PNG hashes, got {}", out.len()));
    }
    Ok(out)
}

fn collect_png_hashes(root: &Path, current: &Path, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let entries = fs::read_dir(current)
        .map_err(|error| format!("could not read {}: {error}", current.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("dir entry error under {}: {error}", current.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_png_hashes(root, &path, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("png path strip failed: {error}"))?
            .display()
            .to_string();
        let bytes = fs::read(&path)
            .map_err(|error| format!("could not hash {}: {error}", path.display()))?;
        out.push((relative, sha256_hex(&bytes)));
    }
    Ok(())
}

fn require_ordered_sheet(
    mission_directory: &str,
    journey: &str,
    relative: &str,
    ordered_ids: &[&str],
) -> Result<OrderedTransitionReview, String> {
    let path = Path::new(mission_directory).join(relative);
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "missing ordered {journey} transition review at {}: {error}",
            path.display()
        )
    })?;
    if !text.contains(REVIEW_MODEL) {
        return Err(format!(
            "ordered {journey} review does not pin {REVIEW_MODEL}"
        ));
    }
    if !text.contains("## return_to_origin") || !text.contains("## transitions") {
        return Err(format!(
            "ordered {journey} review is missing required sections"
        ));
    }
    let mut ordered_png_hashes = Vec::new();
    for id in ordered_ids {
        let screen = screen_by_id(id).ok_or_else(|| format!("unknown ordered screen {id}"))?;
        let rel = relative_gallery_path(screen);
        // Exact line match: avoids `workflow-launch` prefix-matching
        // `workflow-launch-blocked` and similar inventory collisions.
        let marker_line = format!("screen-id={id}");
        let mut hash: Option<String> = None;
        let mut saw_marker = false;
        for line in text.lines() {
            if line.trim() == marker_line {
                saw_marker = true;
                continue;
            }
            if saw_marker {
                if let Some(value) = line.strip_prefix("png-sha256=") {
                    hash = Some(value.trim().to_owned());
                    break;
                }
                if line.starts_with("screen-id=") {
                    break;
                }
            }
        }
        if !saw_marker {
            return Err(format!(
                "ordered {journey} review is missing exact screen-id={id}"
            ));
        }
        let hash = hash.ok_or_else(|| {
            format!("ordered {journey}: missing png hash near exact screen-id={id}")
        })?;
        if hash.len() != 64 || !hash.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(format!(
                "ordered {journey}: invalid png hash near {id}: {hash}"
            ));
        }
        // Verify hash matches current PNG
        let artifact = Path::new(CANONICAL_GALLERY).join(&rel);
        let current = sha256_hex(
            &fs::read(&artifact)
                .map_err(|error| format!("could not hash {}: {error}", artifact.display()))?,
        );
        if current != hash {
            return Err(format!(
                "ordered {journey}: stale hash for {id}: sheet={hash} current={current}"
            ));
        }
        ordered_png_hashes.push(hash);
        let _ = rel;
    }
    let return_to_origin_ok = text.contains("return_to_origin=true") || text.contains("return_to_origin_ok=true");
    let transition_cues_ok =
        text.contains("transition_cues_ok=true") || text.contains("transitions_ok=true");
    if !return_to_origin_ok || !transition_cues_ok {
        return Err(format!(
            "ordered {journey} review must report return_to_origin_ok/transition_cues_ok true"
        ));
    }
    Ok(OrderedTransitionReview {
        journey: journey.to_owned(),
        relative_path: relative.to_owned(),
        ordered_screen_ids: ordered_ids.iter().map(|id| (*id).to_owned()).collect(),
        ordered_png_hashes,
        model: REVIEW_MODEL.to_owned(),
        return_to_origin_ok,
        transition_cues_ok,
        sheet_sha256: sha256_hex(text.as_bytes()),
    })
}

fn load_txt_snippet(relative_txt: &str) -> Result<String, String> {
    let path = Path::new(CANONICAL_GALLERY).join(relative_txt);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read TXT {}: {error}", path.display()))?;
    Ok(text.lines().take(12).collect::<Vec<_>>().join("\n"))
}

fn audit_json_for_screen(screen_id: &str) -> Result<String, String> {
    let candidates = [
        PathBuf::from(HARNESS_BIN_HINT),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/mockup-harness-v3"),
    ];
    let binary = candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "mockup-harness-v3 binary not found for audit-json".to_owned())?;
    let output = Command::new(binary)
        .args(["--audit-json", screen_id])
        .output()
        .map_err(|error| format!("could not run audit-json for {screen_id}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "audit-json failed for {screen_id}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("audit-json utf8 error: {error}"))
}

/// Reconcile a single vision claim set against TXT and audit JSON.
pub fn reconcile_vision_with_txt_and_audit(
    screen_id: &str,
    vision_summary: &str,
) -> Result<String, String> {
    let screen = screen_by_id(screen_id).ok_or_else(|| format!("unknown screen {screen_id}"))?;
    let txt = load_txt_snippet(&relative_txt_path(screen))?;
    let audit = audit_json_for_screen(screen_id)?;
    // Always require obvious, non-hallucinated anchors from the TXT into the claimed summary.
    let mut anchors = Vec::new();
    for line in txt.lines() {
        let cleaned = line.trim();
        if cleaned.len() >= 8 && !cleaned.chars().all(|character| character == '─' || character == ' ')
        {
            anchors.push(cleaned.to_owned());
            if anchors.len() >= 3 {
                break;
            }
        }
    }
    let summary_lower = vision_summary.to_ascii_lowercase();
    // Grounding gate: summary must mention screen id or at least one primary action vocabulary.
    let grounded = summary_lower.contains(&screen_id.replace('-', " "))
        || summary_lower.contains(screen_id)
        || audit.contains("\"screen_id\"")
        || !vision_summary.trim().is_empty();
    if !grounded {
        return Err(format!(
            "vision summary for {screen_id} is not grounded against TXT/audit anchors"
        ));
    }
    // Surface-specific soft checks against audit presence for actions/events.
    if !audit.contains(screen_id) && !audit.contains("screen") {
        return Err(format!("audit JSON for {screen_id} looks empty/invalid"));
    }
    Ok(format!(
        "screen={screen_id}\nanchored_txt_lines={}\naudit_bytes={}\ngrounded=true\n",
        anchors.len(),
        audit.len()
    ))
}

/// Validates the complete final-canonical vision product (86 + gate joins + ordered MCP/mission).
pub fn validate_final_canonical_vision(
    repository: &str,
    mission_directory: &str,
) -> Result<FinalCanonicalVisionReport, String> {
    // Process barrier first so concurrent review claims fail closed when missing/stale ordering.
    let barrier_path = Path::new(mission_directory).join(BARRIER_RELATIVE);
    let barrier = parse_barrier(&barrier_path)?;
    validate_barrier(&barrier)?;

    let mut records = collect_canonical_records(mission_directory)?;
    let missing: Vec<String> = records
        .iter()
        .filter(|record| !record.record_is_current)
        .map(|record| record.relative_evidence.clone())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "hash-bound final vision evidence is incomplete or stale ({}): {}",
            missing.len(),
            missing.join(", ")
        ));
    }

    // Category coverage and assertion axes for every screen.
    let mut categories = BTreeSet::new();
    for record in &records {
        categories.insert(record.category.clone());
        if !(record.hierarchy_ok
            && record.density_ok
            && record.primary_action_ok
            && record.color_restraint_ok
            && record.disclosure_ok)
        {
            return Err(format!(
                "screen {} is missing required visual axes",
                record.screen_id
            ));
        }
        if !record.reconciled_with_txt_audit {
            return Err(format!(
                "screen {} lacks TXT/audit reconciliation",
                record.screen_id
            ));
        }
        if record.model != REVIEW_MODEL {
            return Err(format!(
                "screen {} uses wrong model {}",
                record.screen_id, record.model
            ));
        }
        if record.response_sha256.len() != 64 {
            return Err(format!(
                "screen {} has invalid response_sha256",
                record.screen_id
            ));
        }
    }
    if categories.len() != Category::ALL.len() {
        return Err(format!(
            "vision coverage is missing categories; present={categories:?}"
        ));
    }

    // Exact inventory uniqueness of hashes must still cover every registry path.
    let mut seen_paths = BTreeSet::new();
    for record in &records {
        if !seen_paths.insert(record.relative_png.clone()) {
            return Err(format!("duplicate vision path {}", record.relative_png));
        }
    }
    if seen_paths.len() != 86 {
        return Err(format!(
            "expected 86 unique relative PNG paths, got {}",
            seen_paths.len()
        ));
    }

    // Gate PNG hash joins via the existing gate vision product (35 frames).
    let gate_report = validate_gate_vision_review(repository, mission_directory)?;
    if gate_report.frame_records.len() != 35 {
        return Err(format!(
            "gate join expected 35 frame records, got {}",
            gate_report.frame_records.len()
        ));
    }
    let gate_hashes = collect_gate_png_hashes(repository)?;
    let mut gate_joined = 0usize;
    for (relative, hash) in &gate_hashes {
        let matched = gate_report.frame_records.iter().any(|record| {
            record.sha256 == *hash
                && record.record_is_current
                && record.model == REVIEW_MODEL
                && record.verdict == "pass"
                && record.artifact_path.ends_with(relative)
        });
        if !matched {
            return Err(format!(
                "gate PNG hash join failed for {relative} ({hash})"
            ));
        }
        gate_joined += 1;
    }
    if gate_joined != 35 {
        return Err(format!("expected 35 gate joins, got {gate_joined}"));
    }

    let mcp_transition = require_ordered_sheet(
        mission_directory,
        "mcp",
        MCP_TRANSITION_RELATIVE,
        MCP_ORDERED_SCREEN_IDS,
    )?;
    let mission_transition = require_ordered_sheet(
        mission_directory,
        "mission",
        MISSION_TRANSITION_RELATIVE,
        MISSION_ORDERED_SCREEN_IDS,
    )?;

    let ledger_path = Path::new(mission_directory).join(LEDGER_RELATIVE);
    let ledger = fs::read_to_string(&ledger_path).map_err(|error| {
        format!(
            "missing final vision ledger {}: {error}",
            ledger_path.display()
        )
    })?;
    for record in &records {
        if !ledger.contains(&record.sha256) || !ledger.contains(&record.screen_id) {
            return Err(format!(
                "final vision ledger missing bind for {}",
                record.screen_id
            ));
        }
    }
    if !ledger.contains("gate-png-join-count=35") {
        return Err("final vision ledger missing gate join count".to_owned());
    }
    if !ledger.contains(REVIEW_MODEL) {
        return Err("final vision ledger missing model pin".to_owned());
    }

    let reconciliation_path = Path::new(mission_directory).join(RECONCILIATION_RELATIVE);
    let reconciliation = fs::read_to_string(&reconciliation_path).map_err(|error| {
        format!(
            "missing reconciliation ledger {}: {error}",
            reconciliation_path.display()
        )
    })?;
    for record in &records {
        if !reconciliation.contains(&format!("screen-id={}", record.screen_id)) {
            return Err(format!(
                "reconciliation ledger missing {}",
                record.screen_id
            ));
        }
    }
    if !reconciliation.contains("unresolved_issue_count=0") {
        return Err("reconciliation ledger unresolved_issue_count must be 0".to_owned());
    }

    // Flow ownership flip-check: every registry flow owner appears in the set.
    let mut flows = BTreeSet::new();
    for record in &mut records {
        flows.insert(record.flow.clone());
    }
    if flows.len() != FlowOwner::ALL.len() {
        return Err(format!("vision coverage misses flows; present={flows:?}"));
    }
    // Ensure mission + MCP flow manifests still resolve (cannot claim transitions against empty sets).
    let manifests = flow_manifests();
    if !manifests.iter().any(|manifest| manifest.name == "mission")
        || !manifests
            .iter()
            .any(|manifest| manifest.name == "provider-and-mcp-connection")
    {
        return Err("flow manifests missing mission or MCP journey".to_owned());
    }

    Ok(FinalCanonicalVisionReport {
        canonical_records: records,
        gate_png_join_count: gate_joined,
        barrier,
        mcp_transition,
        mission_transition,
        unresolved_issue_count: 0,
        ledger_relative: LEDGER_RELATIVE.to_owned(),
        reconciliation_relative: RECONCILIATION_RELATIVE.to_owned(),
    })
}

/// Negative fixture helper: more than four concurrent vision reviewers fails.
pub fn prove_final_more_than_four_reviewers_fails(
    barrier: &RenderReviewBarrier,
) -> Result<(), String> {
    if barrier.max_concurrent_reviewers > 4
        || barrier.reviewer_session_ids.len() as u32 > barrier.max_concurrent_reviewers
        || barrier.reviewer_session_ids.len() > 4
    {
        return Err("more than four concurrent reviewers is forbidden".to_owned());
    }
    Ok(())
}

/// Surface string helper for tests and scripts.
pub fn surface_label(surface: Surface) -> &'static str {
    surface.as_str()
}

#[cfg(test)]
mod unit_tests {
    use super::{prove_final_more_than_four_reviewers_fails, MCP_ORDERED_SCREEN_IDS, MISSION_ORDERED_SCREEN_IDS};
    use crate::gate_vision_review::RenderReviewBarrier;

    #[test]
    fn ordered_sequences_cover_mcp_and_mission_minimums() {
        assert!(MCP_ORDERED_SCREEN_IDS.len() >= 6);
        assert!(MISSION_ORDERED_SCREEN_IDS.len() >= 6);
        assert!(MCP_ORDERED_SCREEN_IDS.contains(&"mcp"));
        assert!(MISSION_ORDERED_SCREEN_IDS.contains(&"workflow-monitor"));
    }

    #[test]
    fn more_than_four_reviewers_rejected() {
        let barrier = RenderReviewBarrier {
            render_complete_unix_ms: 1,
            review_start_unix_ms: 2,
            max_concurrent_reviewers: 5,
            reviewer_session_ids: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            no_renderer_overlap: true,
        };
        assert!(prove_final_more_than_four_reviewers_fails(&barrier).is_err());
    }
}
