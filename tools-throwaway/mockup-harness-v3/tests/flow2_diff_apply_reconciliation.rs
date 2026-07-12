use mockup_harness_v3::{
    audit::sha256_hex,
    scenario::{apply_diff_hunks, diff_fixture_file_bytes, diff_index_fixture_bytes, replay_case},
};

#[test]
fn include_replay_focus_is_visible_on_the_test_hunk() {
    let replay = replay_case("staging-include-hunk", 1).expect("include replay must exist");
    replay.validate().expect("include replay must validate");

    assert_eq!(
        replay.diff.as_ref().unwrap().focused_hunk,
        "hunk.retry-test"
    );
    let text = replay.audit.render_txt();
    assert!(text.contains("› ✓ Included · hunk.retry-test"));
    assert!(!text.contains("› ✓ Included · hunk.retry-guard"));
}

#[test]
fn apply_all_renders_both_applied_hunks_without_pending_work() {
    let replay = replay_case("diff-apply-all", 1).expect("apply-all replay must exist");
    replay.validate().expect("apply-all replay must validate");

    let diff = replay.diff.as_ref().unwrap();
    assert_eq!(
        diff.applied_hunks,
        vec!["hunk.retry-guard".to_owned(), "hunk.retry-test".to_owned()]
    );
    assert!(diff.excluded_hunks.is_empty());
    assert_eq!(diff.write_count, 1);

    let text = replay.audit.render_txt();
    assert!(text.contains("Applied 2 hunks to src/net/retry_policy.rs."));
    assert!(text.contains("No hunks remain pending."));
    assert!(!text.contains("remains pending"));
}

#[test]
fn diff_hashes_bind_to_fixture_bytes_and_disclosed_hunks() {
    let finish = replay_case("staging-finish", 1).expect("finish replay must exist");
    let finish_diff = finish.diff.as_ref().unwrap();

    let before = diff_fixture_file_bytes();
    let after_guard = apply_diff_hunks(&before, &["hunk.retry-guard".to_owned()])
        .expect("guard hunk must apply to fixture bytes");
    let index = diff_index_fixture_bytes();
    assert_eq!(finish_diff.working_tree_before, sha256_hex(&before));
    assert_eq!(
        finish_diff.working_tree_after,
        sha256_hex(&after_guard),
        "Finish must hash the bytes after applying the disclosed included hunk"
    );
    assert_eq!(
        finish_diff.git_index_before,
        sha256_hex(index),
        "Git index evidence must hash the index fixture bytes"
    );
    assert_eq!(finish_diff.git_index_before, finish_diff.git_index_after);

    let all = replay_case("diff-apply-all", 1).expect("apply-all replay must exist");
    let all_diff = all.diff.as_ref().unwrap();
    let after_all = apply_diff_hunks(
        &before,
        &["hunk.retry-guard".to_owned(), "hunk.retry-test".to_owned()],
    )
    .expect("both hunks must apply to fixture bytes");
    assert_eq!(all_diff.working_tree_before, sha256_hex(&before));
    assert_eq!(all_diff.working_tree_after, sha256_hex(&after_all));

    let mut invalid = finish.clone();
    invalid.diff.as_mut().unwrap().working_tree_before = "0".repeat(64);
    assert!(
        invalid.validate().is_err(),
        "a well-shaped but unrelated hash must be rejected"
    );

    let mut invalid_hunk = finish;
    invalid_hunk.diff.as_mut().unwrap().hunks[0].content_hash = "f".repeat(64);
    assert!(
        invalid_hunk.validate().is_err(),
        "a well-shaped but unrelated hunk hash must be rejected"
    );
}
