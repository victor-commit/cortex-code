use std::path::Path;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use mockup_harness_v3::gate_programmatic_publication::locate_gate_ready;
use mockup_harness_v3::gate_vision_review::{
    frame_vision_relative_path, journey_sheet_relative_path, prove_more_than_four_reviewers_fails,
    prove_review_before_barrier_fails, validate_gate_vision_review, write_render_review_barrier,
    RenderReviewBarrier,
};

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

static PREFLIGHT: Once = Once::new();

fn ensure_gate_ready_preflight() {
    PREFLIGHT.call_once(|| {
        locate_gate_ready(true, None).expect("programmatic readiness predicate must accept true");
        assert!(
            Path::new(MISSION_DIRECTORY)
                .join("evidence/v3-gate-programmatic-publication/publication-ledger.md")
                .is_file(),
            "vision review requires prior programmatic gate publication ledger"
        );
    });
}

#[test]
fn every_gate_png_has_hash_bound_opus_vision_evidence() {
    ensure_gate_ready_preflight();
    let report = validate_gate_vision_review(REPOSITORY, MISSION_DIRECTORY)
        .expect("complete hash-bound vision product is required");
    assert_eq!(report.frame_records.len(), 35);
    assert!(report.frame_records.iter().all(|record| {
        record.record_is_current
            && record.provenance_ok
            && record.model == "anthropic/claude-opus-4.8-fast"
            && record.verdict == "pass"
            && !record.review_session_id.is_empty()
            && record.response_sha256.len() == 64
            && !record.artifact_path.is_empty()
            && !record.role.is_empty()
    }));
}

#[test]
fn every_bridge_and_journey_sheet_is_current_under_review_barrier() {
    ensure_gate_ready_preflight();
    let report = validate_gate_vision_review(REPOSITORY, MISSION_DIRECTORY)
        .expect("complete bridge/journey vision product is required");
    assert_eq!(report.bridge_records.len(), 30);
    assert!(report.bridge_records.iter().all(|record| {
        record.record_is_current
            && record.provenance_ok
            && record.model == "anthropic/claude-opus-4.8-fast"
            && record.verdict == "pass"
    }));
    assert_eq!(report.journey_sheets.len(), 6);
    assert!(report.barrier.max_concurrent_reviewers <= 4);
    assert!(report.barrier.no_renderer_overlap);
    assert!(report.barrier.review_start_unix_ms >= report.barrier.render_complete_unix_ms);
    for sheet in report.journey_sheets {
        assert_eq!(sheet.sections_present.len(), 6);
        assert!(sheet.frame_count > 0);
        assert!(sheet.bridge_count > 0);
    }
}

#[test]
fn frame_and_journey_paths_follow_established_evidence_layout() {
    assert_eq!(
        frame_vision_relative_path("daily-work", 1, "home"),
        "evidence/v3-fix-gate-publication-bridge-evidence/frames/01-daily-work-home.vision.txt"
    );
    assert_eq!(
        frame_vision_relative_path("mission", 6, "reviewer-approved"),
        "evidence/v3-fix-gate-flows-4-6-bridge-evidence/frames/06-mission-reviewer-approved.vision.txt"
    );
    assert_eq!(
        journey_sheet_relative_path("recovery"),
        "evidence/v3-gate-vision-review/sheets/recovery-journey-review.md"
    );
}

#[test]
fn more_than_four_reviewers_and_review_before_barrier_are_rejected() {
    let barrier = RenderReviewBarrier {
        render_complete_unix_ms: 100,
        review_start_unix_ms: 101,
        max_concurrent_reviewers: 5,
        reviewer_session_ids: (0..5).map(|i| format!("s{i}")).collect(),
        no_renderer_overlap: true,
    };
    assert!(prove_more_than_four_reviewers_fails(&barrier)
        .expect_err(">4 reviewers must fail")
        .contains("four"));
    assert!(prove_review_before_barrier_fails(200, 100)
        .expect_err("pre-barrier review must fail")
        .contains("barrier"));
}

#[test]
fn write_render_review_barrier_rejects_invalid_concurrency() {
    let render_complete = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    let sandbox = std::env::temp_dir().join(format!(
        "v3-gate-vision-barrier-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&sandbox).expect("temp barrier root");
    let err = write_render_review_barrier(
        sandbox.to_str().expect("utf8 temp path"),
        render_complete,
        5,
        &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
    )
    .expect_err("five concurrent reviewers must not write a barrier");
    assert!(err.contains("1..=4") || err.contains("four") || err.contains("concurrent"));
    // Valid write targets an isolated evidence root, never the mission commodity barrier.
    let ids = vec!["07321a83-692c-4877-b120-77ccfcbe0157".to_owned()];
    let path = write_render_review_barrier(
        sandbox.to_str().expect("utf8 temp path"),
        render_complete,
        4,
        &ids,
    )
    .expect("legal barrier write must succeed");
    assert!(Path::new(&path).is_file());
    let _ = std::fs::remove_dir_all(&sandbox);
}
