use std::process::Command;

use mockup_harness_v3::{
    audit::{audit_screen, sha256_hex},
    scenario::{apply_diff_hunks, diff_fixture_file_bytes, replay_case},
};

#[test]
fn ordered_hunk_coordinates_include_prior_cumulative_line_deltas() {
    let replay = replay_case("diff-apply-all", 1).expect("apply-all replay must exist");
    replay.validate().expect("apply-all replay must validate");
    let diff = replay.diff.as_ref().expect("diff evidence must exist");

    let guard = &diff.hunks[0];
    assert_eq!(
        (
            guard.old_start,
            guard.old_lines,
            guard.new_start,
            guard.new_lines,
        ),
        (42, 2, 42, 3)
    );
    let retry_test = &diff.hunks[1];
    assert_eq!(
        (
            retry_test.old_start,
            retry_test.old_lines,
            retry_test.new_start,
            retry_test.new_lines,
        ),
        (88, 2, 89, 2),
        "retry-test must include retry-guard's cumulative +1 line delta"
    );
}

#[test]
fn applying_both_hunks_places_retry_test_postimage_at_lines_89_and_90() {
    let before = diff_fixture_file_bytes();
    let after = apply_diff_hunks(
        &before,
        &["hunk.retry-guard".to_owned(), "hunk.retry-test".to_owned()],
    )
    .expect("both exact coordinate-bound hunks must apply");
    let lines = String::from_utf8(after.clone())
        .expect("fixture must remain UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();

    assert_eq!(lines[88], "assert_eq!(wait, 5)");
    assert_eq!(lines[89], "assert_eq!(attempts, 1)");
    let replay = replay_case("diff-apply-all", 1).expect("apply-all replay must exist");
    assert_eq!(
        replay.diff.as_ref().unwrap().working_tree_after,
        sha256_hex(&after)
    );
}

#[test]
fn staging_render_uses_the_validated_cumulative_coordinates() {
    let audit =
        audit_screen("diff-viewer-staging", "flow2-staging").expect("staging detail must render");
    let text = audit.render_txt();

    assert!(text.contains("hunk.retry-guard · old 42,2 -> new 42,3"));
    assert!(text.contains("hunk.retry-test · old 88,2 -> new 89,2"));
    assert!(text.contains("88  89  -assert_eq!(wait, 4)"));
    assert!(text.contains("    90  +assert_eq!(attempts, 1)"));
}

#[test]
fn selected_hunk_application_rejects_content_search_fallback() {
    let mut lines = String::from_utf8(diff_fixture_file_bytes())
        .expect("fixture must be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines[41] = "// stale retry guard coordinate".to_owned();
    lines[49] = "    return retry_after".to_owned();
    lines[50] = "    backoff = backoff + 1".to_owned();
    let stale_at_coordinate = format!("{}\n", lines.join("\n")).into_bytes();

    let rejection = apply_diff_hunks(&stale_at_coordinate, &["hunk.retry-guard".to_owned()])
        .expect_err("matching content elsewhere must not relocate a stale hunk");
    assert!(rejection.contains("coordinate") || rejection.contains("preimage"));
}

#[test]
fn public_negative_fixtures_reject_every_coordinate_and_payload_drift() {
    for fixture in [
        "diff-stale-old-coordinate",
        "diff-incorrect-new-start",
        "diff-wrong-counts",
        "diff-payload-coordinate-drift",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
            .args(["negative-fixture", fixture])
            .output()
            .expect("negative fixture CLI must run");
        assert_eq!(output.status.code(), Some(3), "{fixture} must be rejected");
        assert!(output.stdout.is_empty());
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains(&format!("fixture {fixture}")));
        assert!(diagnostic.contains("diff-viewer-staging"));
    }
}
