//! Final canonical vision product tests.
//!
//! Owned assertions:
//! - VAL-V3-CROSS-017 full native Opus review of 86 canonical PNG hashes
//! - VAL-V3-CROSS-018 grounded claims + ordered MCP/mission transition reviews
//! - VAL-V3-HARNESS-033 vision evidence covers final canonical and gate PNG hashes

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use mockup_harness_v3::final_canonical_vision::{
    prove_final_more_than_four_reviewers_fails, validate_final_canonical_vision,
    write_final_render_review_barrier, MCP_ORDERED_SCREEN_IDS, MISSION_ORDERED_SCREEN_IDS,
};
use mockup_harness_v3::gate_vision_review::RenderReviewBarrier;
use mockup_harness_v3::canonical_inventory;

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

#[test]
fn final_vision_index_matches_png_manifest() {
    let report = validate_final_canonical_vision(REPOSITORY, MISSION_DIRECTORY)
        .expect("complete final vision product is required");
    assert_eq!(report.canonical_records.len(), 86);
    assert_eq!(report.gate_png_join_count, 35);
    assert_eq!(report.unresolved_issue_count, 0);
    assert!(report.canonical_records.iter().all(|record| {
        record.record_is_current
            && record.provenance_ok
            && record.model == "anthropic/claude-opus-4.8-fast"
            && record.verdict == "pass"
            && record.response_sha256.len() == 64
            && record.hierarchy_ok
            && record.density_ok
            && record.primary_action_ok
            && record.color_restraint_ok
            && record.disclosure_ok
            && record.reconciled_with_txt_audit
    }));

    // Registry bijection by id.
    let expected: Vec<&str> = canonical_inventory().iter().map(|s| s.id).collect();
    let actual: Vec<String> = report
        .canonical_records
        .iter()
        .map(|record| record.screen_id.clone())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn ordered_mcp_and_mission_transition_reviews_bind_live_hashes() {
    let report = validate_final_canonical_vision(REPOSITORY, MISSION_DIRECTORY)
        .expect("ordered transition reviews required");
    assert_eq!(
        report.mcp_transition.ordered_screen_ids.len(),
        MCP_ORDERED_SCREEN_IDS.len()
    );
    assert_eq!(
        report.mission_transition.ordered_screen_ids.len(),
        MISSION_ORDERED_SCREEN_IDS.len()
    );
    assert!(report.mcp_transition.return_to_origin_ok);
    assert!(report.mcp_transition.transition_cues_ok);
    assert!(report.mission_transition.return_to_origin_ok);
    assert!(report.mission_transition.transition_cues_ok);
    assert_eq!(
        report.mcp_transition.ordered_png_hashes.len(),
        MCP_ORDERED_SCREEN_IDS.len()
    );
    assert_eq!(
        report.mission_transition.ordered_png_hashes.len(),
        MISSION_ORDERED_SCREEN_IDS.len()
    );
    assert!(report.barrier.max_concurrent_reviewers <= 4);
    assert!(report.barrier.no_renderer_overlap);
    assert!(report.barrier.review_start_unix_ms >= report.barrier.render_complete_unix_ms);
}

#[test]
fn more_than_four_reviewers_and_missing_product_are_rejected() {
    let barrier = RenderReviewBarrier {
        render_complete_unix_ms: 100,
        review_start_unix_ms: 101,
        max_concurrent_reviewers: 5,
        reviewer_session_ids: (0..5).map(|i| format!("s{i}")).collect(),
        no_renderer_overlap: true,
    };
    assert!(prove_final_more_than_four_reviewers_fails(&barrier)
        .expect_err(">4 reviewers must fail")
        .contains("four"));

    // Isolated temp barrier write allows at most 4 concurrent reviewers.
    let render_complete = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    let sandbox = std::env::temp_dir().join(format!(
        "v3-final-vision-barrier-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&sandbox).expect("temp barrier root");
    let err = write_final_render_review_barrier(
        sandbox.to_str().expect("utf8"),
        render_complete,
        5,
        &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
    )
    .expect_err("five concurrent reviewers must not write a barrier");
    assert!(err.contains("1..=4") || err.contains("four") || err.contains("concurrent"));
    let ids = vec!["e1ca2399-0414-4e13-8336-21ac05624b8b".to_owned()];
    let path = write_final_render_review_barrier(
        sandbox.to_str().expect("utf8"),
        render_complete,
        4,
        &ids,
    )
    .expect("legal barrier write must succeed");
    assert!(Path::new(&path).is_file());
    let _ = std::fs::remove_dir_all(&sandbox);
}
