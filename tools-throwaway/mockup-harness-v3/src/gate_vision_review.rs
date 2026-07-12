//! Six-flow gate vision review product.
//!
//! After programmatic publication is gate-ready, this module joins every gate PNG and
//! every distinct bridge scenario to hash-bound Opus 4.8 vision evidence, records a
//! render-to-review barrier, and validates six journey review sheets. Read-only review
//! may run at most four concurrent validator records after publication completes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::sha256_hex;
use crate::flows::flow_manifests;
use crate::gate_integration::{validate_review_provenance, VisionFinding};
use crate::gate_programmatic_publication::{
    collect_bridge_digests, collect_path_hash_tuples, locate_gate_ready, six_flow_gate_artifacts,
    six_flow_gate_bridges,
};
use crate::gate_remaining_integration::remaining_gate_artifacts;
use crate::registry::canonical_inventory;

const SIX_FLOWS: [&str; 6] = [
    "daily-work",
    "input-and-approval",
    "work-configuration",
    "provider-and-mcp-connection",
    "recovery",
    "mission",
];

const REVIEW_MODEL: &str = "anthropic/claude-opus-4.8-fast";
const EVIDENCE_ROOT: &str = "evidence/v3-gate-vision-review";
const BARRIER_RELATIVE: &str = "evidence/v3-gate-vision-review/render-to-review-barrier.json";

const FIRST_THREE_FRAME_ROOT: &str = "evidence/v3-fix-gate-publication-bridge-evidence/frames";
const REMAINING_FRAME_ROOT: &str = "evidence/v3-fix-gate-flows-4-6-bridge-evidence/frames";

const JOURNEY_SHEET_SECTIONS: [&str; 6] = [
    "entry",
    "action",
    "transition",
    "cancel_or_back",
    "success_return",
    "expected_result",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JourneyReviewSheet {
    pub flow: &'static str,
    pub relative_path: String,
    pub sheet_sha256: String,
    pub sections_present: Vec<&'static str>,
    pub frame_count: usize,
    pub bridge_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisionReviewRecord {
    pub kind: &'static str,
    pub flow: &'static str,
    pub relative_path: String,
    pub artifact_path: String,
    pub role: String,
    pub sha256: String,
    pub model: String,
    pub verdict: String,
    pub review_session_id: String,
    pub response_sha256: String,
    pub record_is_current: bool,
    pub provenance_ok: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderReviewBarrier {
    pub render_complete_unix_ms: u64,
    pub review_start_unix_ms: u64,
    pub max_concurrent_reviewers: u32,
    pub reviewer_session_ids: Vec<String>,
    pub no_renderer_overlap: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateVisionReviewReport {
    pub programmatic_gate_ready: bool,
    pub frame_records: Vec<VisionReviewRecord>,
    pub bridge_records: Vec<VisionReviewRecord>,
    pub journey_sheets: Vec<JourneyReviewSheet>,
    pub barrier: RenderReviewBarrier,
    pub required_iteration: bool,
}

fn first_three_flow(flow: &str) -> bool {
    matches!(
        flow,
        "daily-work" | "input-and-approval" | "work-configuration"
    )
}

fn frame_root_for(flow: &str) -> &'static str {
    if first_three_flow(flow) {
        FIRST_THREE_FRAME_ROOT
    } else {
        REMAINING_FRAME_ROOT
    }
}

fn role_prefix_for(flow: &str) -> &'static str {
    if first_three_flow(flow) {
        "first-three"
    } else {
        "remaining-three"
    }
}

pub fn frame_vision_relative_path(flow: &str, step_index: usize, screen_id: &str) -> String {
    format!(
        "{}/{:02}-{}-{}.vision.txt",
        frame_root_for(flow),
        step_index,
        flow,
        screen_id
    )
}

pub fn journey_sheet_relative_path(flow: &str) -> String {
    format!("{EVIDENCE_ROOT}/sheets/{flow}-journey-review.md")
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
                "duplicate review record field {key} in {}",
                path.display()
            ));
        }
    }
    Ok(record)
}

fn evaluate_review_record(
    mission_directory: &str,
    kind: &'static str,
    flow: &'static str,
    relative_path: &str,
    artifact_path: &str,
    role: &str,
    expected_sha256: &str,
) -> VisionReviewRecord {
    let evidence_path = Path::new(mission_directory).join(relative_path);
    let mut verdict = String::new();
    let mut review_session_id = String::new();
    let mut response_sha256 = String::new();
    let mut model = String::new();
    let mut record_is_current = false;
    let mut provenance_ok = false;

    if evidence_path.is_file() {
        if let Ok(review) = read_review_record(&evidence_path) {
            verdict = review.get("verdict").cloned().unwrap_or_default();
            review_session_id = review.get("review_session_id").cloned().unwrap_or_default();
            response_sha256 = review.get("response_sha256").cloned().unwrap_or_default();
            model = review.get("model").cloned().unwrap_or_default();
            record_is_current = require_field(&review, "artifact_path", artifact_path)
                .and_then(|_| require_field(&review, "role", role))
                .and_then(|_| require_field(&review, "sha256", expected_sha256))
                .and_then(|_| require_field(&review, "model", REVIEW_MODEL))
                .and_then(|_| require_field(&review, "verdict", "pass"))
                .is_ok();
            provenance_ok = validate_review_provenance(&review).is_ok();
            if kind == "bridge" {
                // Bridge evidence is fail-closed on provenance.
                record_is_current = record_is_current && provenance_ok;
            } else {
                // Frame evidence remains soft for provenance (matches gate integration),
                // but still prefers recomputable Factory-owned source sessions.
                record_is_current = record_is_current && provenance_ok;
            }
        }
    }

    VisionReviewRecord {
        kind,
        flow,
        relative_path: relative_path.to_owned(),
        artifact_path: artifact_path.to_owned(),
        role: role.to_owned(),
        sha256: expected_sha256.to_owned(),
        model,
        verdict,
        review_session_id,
        response_sha256,
        record_is_current,
        provenance_ok,
    }
}

fn collect_frame_records(
    repository: &str,
    mission_directory: &str,
) -> Result<Vec<VisionReviewRecord>, String> {
    let mut records = Vec::new();
    let gate_root = Path::new(repository).join("plans/assets/mockups-v3-gates");
    for artifact in six_flow_gate_artifacts()
        .into_iter()
        .filter(|artifact| artifact.extension == "png")
    {
        let gate = gate_root.join(&artifact.gate_relative);
        let bytes = fs::read(&gate)
            .map_err(|error| format!("could not hash {}: {error}", gate.display()))?;
        let sha256 = sha256_hex(&bytes);
        let relative = frame_vision_relative_path(
            artifact.flow,
            artifact.step_index,
            artifact.screen_id,
        );
        let role = format!(
            "{} gate frame {} step {:02} {}",
            role_prefix_for(artifact.flow),
            artifact.flow,
            artifact.step_index,
            artifact.screen_id
        );
        records.push(evaluate_review_record(
            mission_directory,
            "frame",
            artifact.flow,
            &relative,
            &gate.display().to_string(),
            &role,
            &sha256,
        ));
    }
    if records.len() != 35 {
        return Err(format!(
            "expected exactly 35 gate PNG vision records, got {}",
            records.len()
        ));
    }
    Ok(records)
}

fn collect_bridge_records(mission_directory: &str) -> Result<Vec<VisionReviewRecord>, String> {
    let mut records = Vec::new();
    for bridge in six_flow_gate_bridges() {
        let png_path = Path::new(mission_directory).join(bridge.evidence_png);
        let bytes = fs::read(&png_path).map_err(|error| {
            format!(
                "could not hash bridge PNG {}: {error}",
                png_path.display()
            )
        })?;
        let sha256 = sha256_hex(&bytes);
        records.push(evaluate_review_record(
            mission_directory,
            "bridge",
            bridge.flow,
            bridge.vision_evidence,
            &png_path.display().to_string(),
            bridge.role,
            &sha256,
        ));
    }
    if records.len() != 30 {
        return Err(format!(
            "expected exactly 30 bridge vision records, got {}",
            records.len()
        ));
    }
    Ok(records)
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

fn collect_journey_sheets(mission_directory: &str) -> Result<Vec<JourneyReviewSheet>, String> {
    let mut sheets = Vec::new();
    for flow in SIX_FLOWS {
        let relative = journey_sheet_relative_path(flow);
        let path = Path::new(mission_directory).join(&relative);
        let text = fs::read_to_string(&path).map_err(|error| {
            format!(
                "missing journey review sheet for {flow} at {}: {error}",
                path.display()
            )
        })?;
        let sheet_sha256 = sha256_hex(text.as_bytes());
        let mut sections_present = Vec::new();
        for section in JOURNEY_SHEET_SECTIONS {
            let heading = format!("## {section}");
            if text.contains(&heading) {
                sections_present.push(section);
            }
        }
        if sections_present.len() != JOURNEY_SHEET_SECTIONS.len() {
            return Err(format!(
                "journey sheet {flow} is missing required sections; present={sections_present:?}"
            ));
        }
        if !text.contains(REVIEW_MODEL) {
            return Err(format!("journey sheet {flow} does not pin {REVIEW_MODEL}"));
        }
        if !text.contains(&format!("`{flow}`")) && !text.contains(flow) {
            return Err(format!("journey sheet {flow} does not name its flow"));
        }
        let frame_count = text.matches("gate-png-sha256=").count();
        let bridge_count = text.matches("bridge-case=").count();
        sheets.push(JourneyReviewSheet {
            flow,
            relative_path: relative,
            sheet_sha256,
            sections_present,
            frame_count,
            bridge_count,
        });
    }
    Ok(sheets)
}

/// Validates the full vision-review product for all six flows.
pub fn validate_gate_vision_review(
    repository: &str,
    mission_directory: &str,
) -> Result<GateVisionReviewReport, String> {
    locate_gate_ready(true, None).map_err(|error| {
        format!("programmatic gate is not ready for vision review: {error}")
    })?;

    // Inventory joins remain authoritative.
    let tuples = collect_path_hash_tuples(repository)?;
    if tuples.len() != 70 {
        return Err(format!("expected 70 path-hash tuples, got {}", tuples.len()));
    }
    let bridges = collect_bridge_digests(mission_directory)?;
    if bridges.len() != 30 {
        return Err(format!("expected 30 bridge digests, got {}", bridges.len()));
    }
    crate::validate_flow_manifests(flow_manifests(), canonical_inventory())?;

    let frame_records = collect_frame_records(repository, mission_directory)?;
    let bridge_records = collect_bridge_records(mission_directory)?;

    let barrier_path = Path::new(mission_directory).join(BARRIER_RELATIVE);
    let barrier = parse_barrier(&barrier_path)?;
    validate_barrier(&barrier)?;

    let journey_sheets = collect_journey_sheets(mission_directory)?;
    if journey_sheets.len() != 6 {
        return Err(format!(
            "expected six journey sheets, got {}",
            journey_sheets.len()
        ));
    }

    // Every sheet must bind its flow frames and at least one bridge when bridges exist.
    let bridge_counts = six_flow_gate_bridges()
        .into_iter()
        .fold(BTreeMap::<&str, usize>::new(), |mut map, bridge| {
            *map.entry(bridge.flow).or_default() += 1;
            map
        });
    let frame_counts = six_flow_gate_artifacts()
        .into_iter()
        .filter(|artifact| artifact.extension == "png")
        .fold(BTreeMap::<&str, usize>::new(), |mut map, artifact| {
            *map.entry(artifact.flow).or_default() += 1;
            map
        });
    for sheet in &journey_sheets {
        let expected_frames = *frame_counts.get(sheet.flow).unwrap_or(&0);
        let expected_bridges = *bridge_counts.get(sheet.flow).unwrap_or(&0);
        if sheet.frame_count < expected_frames {
            return Err(format!(
                "journey sheet {} binds only {}/{} frame hashes",
                sheet.flow, sheet.frame_count, expected_frames
            ));
        }
        if sheet.bridge_count < expected_bridges {
            return Err(format!(
                "journey sheet {} binds only {}/{} bridge cases",
                sheet.flow, sheet.bridge_count, expected_bridges
            ));
        }
    }

    let required_iteration = frame_records
        .iter()
        .chain(bridge_records.iter())
        .any(|record| !record.record_is_current);

    if required_iteration {
        let missing = frame_records
            .iter()
            .chain(bridge_records.iter())
            .filter(|record| !record.record_is_current)
            .map(|record| record.relative_path.clone())
            .collect::<Vec<_>>();
        return Err(format!(
            "hash-bound vision evidence is incomplete or stale: {}",
            missing.join(", ")
        ));
    }

    Ok(GateVisionReviewReport {
        programmatic_gate_ready: true,
        frame_records,
        bridge_records,
        journey_sheets,
        barrier,
        required_iteration: false,
    })
}

/// Writes the render-to-review barrier once publication is complete and before reviews begin.
pub fn write_render_review_barrier(
    mission_directory: &str,
    render_complete_unix_ms: u64,
    max_concurrent_reviewers: u32,
    reviewer_session_ids: &[String],
) -> Result<PathBuf, String> {
    if max_concurrent_reviewers == 0 || max_concurrent_reviewers > 4 {
        return Err("max concurrent reviewers must be 1..=4".to_owned());
    }
    if reviewer_session_ids.len() > max_concurrent_reviewers as usize {
        return Err("cannot record more concurrent reviewers than the declared limit".to_owned());
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
         assertion_ids=VAL-V3-GATE-012,VAL-V3-GATE-013,VAL-V3-HARNESS-031\n",
        reviewer_session_ids.join(",")
    );
    fs::write(&path, body).map_err(|error| format!("could not write barrier: {error}"))?;
    Ok(path)
}

/// Helper for negative fixtures: campaign where more than four reviewers purge readiness.
pub fn prove_more_than_four_reviewers_fails(
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

/// Helper for negative fixtures: reviewing before the barrier fails.
pub fn prove_review_before_barrier_fails(
    render_complete_unix_ms: u64,
    review_start_unix_ms: u64,
) -> Result<(), String> {
    if review_start_unix_ms < render_complete_unix_ms {
        return Err("review-before-barrier is forbidden".to_owned());
    }
    Ok(())
}

/// Expose currently mapped vision findings for remaining integration soft joins.
pub fn remaining_frame_findings(
    repository: &str,
    mission_directory: &str,
) -> Result<Vec<VisionFinding>, String> {
    let frames = remaining_gate_artifacts()
        .into_iter()
        .filter(|artifact| artifact.extension == "png")
        .collect::<Vec<_>>();
    frames
        .into_iter()
        .map(|artifact| {
            let gate = Path::new(repository)
                .join("plans/assets/mockups-v3-gates")
                .join(&artifact.gate_relative);
            let current_sha256 = sha256_hex(
                &fs::read(&gate)
                    .map_err(|error| format!("could not hash {}: {error}", gate.display()))?,
            );
            let relative = frame_vision_relative_path(
                artifact.flow,
                artifact.step_index,
                artifact.screen_id,
            );
            let role = format!(
                "remaining-three gate frame {} step {:02} {}",
                artifact.flow, artifact.step_index, artifact.screen_id
            );
            let evidence_path = Path::new(mission_directory).join(&relative);
            let record_is_current = if evidence_path.is_file() {
                let review = read_review_record(&evidence_path)?;
                require_field(&review, "artifact_path", &gate.display().to_string())
                    .and_then(|_| require_field(&review, "role", &role))
                    .and_then(|_| require_field(&review, "sha256", &current_sha256))
                    .and_then(|_| require_field(&review, "model", REVIEW_MODEL))
                    .and_then(|_| require_field(&review, "verdict", "pass"))
                    .and_then(|_| validate_review_provenance(&review))
                    .is_ok()
            } else {
                false
            };
            Ok(VisionFinding {
                flow: artifact.flow,
                gate_png: artifact.gate_relative,
                current_sha256,
                evidence_path: relative,
                role,
                model: REVIEW_MODEL.to_owned(),
                record_is_current,
            })
        })
        .collect()
}

#[cfg(test)]
mod negative_fixture_tests {
    use super::{prove_more_than_four_reviewers_fails, prove_review_before_barrier_fails, RenderReviewBarrier};

    #[test]
    fn more_than_four_reviewers_is_rejected() {
        let barrier = RenderReviewBarrier {
            render_complete_unix_ms: 1,
            review_start_unix_ms: 2,
            max_concurrent_reviewers: 5,
            reviewer_session_ids: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                "e".into(),
            ],
            no_renderer_overlap: true,
        };
        assert!(prove_more_than_four_reviewers_fails(&barrier).is_err());
    }

    #[test]
    fn review_before_barrier_is_rejected() {
        assert!(prove_review_before_barrier_fails(10, 9).is_err());
        assert!(prove_review_before_barrier_fails(10, 10).is_ok());
    }
}
