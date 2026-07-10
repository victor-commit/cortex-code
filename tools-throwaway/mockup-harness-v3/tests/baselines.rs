use std::path::Path;

#[test]
fn frozen_v1_v2_references_match_the_recorded_manifest() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    mockup_harness_v3::verify_frozen_references(&repository)
        .expect("frozen v1/v2 references must match the immutable baseline");
}

#[test]
fn forbidden_paths_match_the_recorded_status_baseline() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    mockup_harness_v3::verify_forbidden_paths(&repository)
        .expect("off-limits paths must match the status baseline");
}
