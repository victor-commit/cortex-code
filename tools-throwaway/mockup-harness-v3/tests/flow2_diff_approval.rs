use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

#[test]
fn pending_diff_is_read_only_and_has_one_review_primary() {
    let audit = audit_screen("diff-edit", "flow2-diff-pending")
        .expect("pending diff must produce typed render evidence");
    audit.validate().expect("pending diff audit must validate");
    let text = audit.render_txt();

    assert!(text.contains("Pending edit"));
    assert!(text.contains("src/net/retry_policy.rs"));
    assert!(text.contains("Patch summary +4 -3 · 2 hunks"));
    assert!(text.contains("Hunk @@ -42,2 +42,3"));
    assert!(text.contains("-    return retry_after"));
    assert!(text.contains("+    return retry_after + jitter"));
    assert!(text.contains("Why this change"));
    assert!(!text.contains("Applied"));
    assert!(!text.contains("Success"));
    assert!(!text.contains("Staged"));

    let actions = audit
        .events
        .iter()
        .filter_map(|event| event.action.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        actions
            .iter()
            .filter(|action| action.label == "Review changes")
            .count(),
        1
    );
    assert_eq!(
        actions
            .iter()
            .filter(|action| action.label == "Apply all" || action.label == "Cancel")
            .count(),
        2
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn review_enters_staging_without_writing_and_binds_hunks_to_file() {
    let replay =
        replay_case("diff-review", 1).expect("review changes must open staging without a write");
    replay.validate().expect("review replay must validate");
    assert_eq!(replay.before.screen_id, "diff-edit");
    assert_eq!(replay.after.screen_id, "diff-viewer-staging");
    assert_eq!(replay.attempts[0].count, 0);
    assert_eq!(replay.side_effects[0].status, "no-write");
    let diff = replay.diff.as_ref().expect("review must include diff evidence");
    assert_eq!(diff.patch_revision, "patch-retry-18");
    assert_eq!(diff.file_path, "src/net/retry_policy.rs");
    assert_eq!(diff.hunks.len(), 2);
    assert_eq!(diff.hunks[0].hunk_id, "hunk.retry-guard");
    assert_eq!(diff.hunks[1].hunk_id, "hunk.retry-test");
    assert_eq!(diff.included_count, 1);
    assert_eq!(diff.git_index_before, diff.git_index_after);
}

#[test]
fn staging_is_understandable_without_color_and_include_exclude_are_local() {
    let audit = audit_screen("diff-viewer-staging", "flow2-staging")
        .expect("staging detail must produce typed render evidence");
    audit.validate().expect("staging audit must validate");
    let text = audit.render_txt();
    assert!(text.contains("Included"));
    assert!(text.contains("Pending"));
    assert!(text.contains("Included 1 of 2 hunks"));
    assert!(text.contains("Git index unchanged"));
    assert!(text.contains("+"));
    assert!(text.contains("-"));
    assert!(text.contains("old 42"));
    assert!(text.contains("new 42"));
    assert!(text.contains("Edit"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "hunk")
            .count(),
        2
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "focused-hunk")
            .count(),
        1
    );

    let include = replay_case("staging-include-hunk", 1).expect("include must replay");
    let exclude = replay_case("staging-exclude-hunk", 1).expect("exclude must replay");
    let include_all = replay_case("staging-include-all", 1).expect("include all must replay");
    for replay in [&include, &exclude] {
        replay.validate().expect("focused hunk action must validate");
        assert_eq!(replay.after.screen_id, "diff-viewer-staging");
        assert_eq!(replay.attempts[0].count, 0);
        assert_eq!(replay.side_effects[0].status, "no-write");
        assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
    }
    assert_eq!(
        include.diff.as_ref().unwrap().included_count,
        2,
        "Include changes only the focused hunk and updates the count"
    );
    assert_eq!(
        exclude.diff.as_ref().unwrap().included_count,
        0,
        "Exclude changes only the focused hunk and updates the count"
    );
    include_all
        .validate()
        .expect("Include all replay must validate");
    assert_eq!(include_all.diff.as_ref().unwrap().included_count, 2);
    assert_eq!(include_all.side_effects[0].status, "no-write");
}

#[test]
fn finish_applies_only_included_hunks_and_never_changes_git_index() {
    let replay =
        replay_case("staging-finish", 1).expect("staging finish must apply included hunks");
    replay.validate().expect("staging finish replay must validate");
    assert_eq!(replay.before.screen_id, "diff-viewer-staging");
    assert_eq!(replay.after.screen_id, "chat-turn");
    let diff = replay.diff.as_ref().unwrap();
    assert_eq!(diff.included_count, 1);
    assert_eq!(diff.applied_hunks, vec!["hunk.retry-guard"]);
    assert_eq!(diff.excluded_hunks, vec!["hunk.retry-test"]);
    assert_eq!(diff.write_count, 1);
    assert_eq!(diff.git_index_before, diff.git_index_after);
    assert_eq!(replay.side_effects[0].status, "applied-included-hunks");
    assert_eq!(
        replay
            .preserved_field_hashes
            .get("patch_revision")
            .unwrap()
            .before,
        replay
            .preserved_field_hashes
            .get("patch_revision")
            .unwrap()
            .after
    );
    assert!(replay.audit.render_txt().contains("Applied 1 hunk"));
    assert!(replay.audit.render_txt().contains("hunk.retry-test remains pending"));
}

#[test]
fn cancel_and_back_write_nothing_and_restore_the_immediate_origin() {
    let pending_cancel = replay_case("diff-cancel", 1).expect("pending cancel must replay");
    let staging_cancel = replay_case("staging-cancel", 1).expect("staging cancel must replay");

    pending_cancel
        .validate()
        .expect("pending cancel replay must validate");
    staging_cancel
        .validate()
        .expect("staging cancel replay must validate");
    assert_eq!(pending_cancel.after.screen_id, "chat-turn");
    assert_eq!(staging_cancel.after.screen_id, "diff-edit");
    assert_eq!(pending_cancel.side_effects[0].status, "no-write");
    assert_eq!(staging_cancel.side_effects[0].status, "no-write");
    for replay in [&pending_cancel, &staging_cancel] {
        assert_eq!(replay.attempts[0].count, 0);
        assert_eq!(replay.before.draft, replay.after.draft);
        assert_eq!(replay.before.transcript, replay.after.transcript);
        assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
        assert_eq!(
            replay.diff.as_ref().unwrap().patch_revision,
            "patch-retry-18"
        );
    }
}

#[test]
fn staging_replays_render_their_derived_focus_and_hunk_state() {
    let include = replay_case("staging-include-hunk", 1).expect("include must replay");
    let exclude = replay_case("staging-exclude-hunk", 1).expect("exclude must replay");
    let include_all = replay_case("staging-include-all", 1).expect("include all must replay");

    assert_eq!(include.after.focus, "staging-detail");
    assert_eq!(include.after.selection, "include");
    assert_eq!(include.diff.as_ref().unwrap().hunks[0].state, "Included");
    assert_eq!(include.diff.as_ref().unwrap().hunks[1].state, "Included");
    assert_eq!(exclude.diff.as_ref().unwrap().hunks[0].state, "Excluded");
    assert_eq!(exclude.diff.as_ref().unwrap().hunks[1].state, "Pending");
    assert_eq!(
        include_all.diff.as_ref().unwrap().included_count,
        include_all
            .diff
            .as_ref()
            .unwrap()
            .hunks
            .iter()
            .filter(|hunk| hunk.state == "Included")
            .count()
    );

    let rendered = audit_screen("diff-viewer-staging", "case.staging-exclude-hunk.tick.1")
        .expect("excluded staging result must render");
    let text = rendered.render_txt();
    assert!(text.contains("Included 0 of 2 hunks"));
    assert!(text.contains("✗ Excluded · hunk.retry-guard"));
    assert!(text.contains("○ Pending · hunk.retry-test"));
    let included = audit_screen("diff-viewer-staging", "case.staging-include-hunk.tick.1")
        .expect("included staging result must render");
    assert!(included.render_txt().contains("Included 2 of 2 hunks"));
    let all = audit_screen("diff-viewer-staging", "case.staging-include-all.tick.1")
        .expect("include-all staging result must render");
    assert!(all.render_txt().contains("Included 2 of 2 hunks"));
}

#[test]
fn apply_evidence_uses_exact_file_hashes_and_atomic_failure_preserves_bytes() {
    let finish = replay_case("staging-finish", 1).expect("finish must replay");
    let diff = finish.diff.as_ref().unwrap();
    assert_eq!(diff.working_tree_before.len(), 64);
    assert_eq!(diff.working_tree_after.len(), 64);
    assert_ne!(diff.working_tree_before, diff.working_tree_after);
    assert_eq!(diff.git_index_before, diff.git_index_after);
    assert_eq!(diff.excluded_hunks, vec!["hunk.retry-test"]);
    assert!(finish.audit.render_txt().contains("Before SHA-256:"));
    assert!(finish.audit.render_txt().contains("After SHA-256:"));

    let failed = replay_case("staging-apply-partial-failure", 1).expect("failed apply must replay");
    let failed_diff = failed.diff.as_ref().unwrap();
    assert_eq!(failed.after.screen_id, "diff-viewer-staging");
    assert_eq!(failed_diff.write_count, 0);
    assert_eq!(
        failed_diff.working_tree_before,
        failed_diff.working_tree_after
    );
    assert_eq!(failed_diff.git_index_before, failed_diff.git_index_after);
    assert_eq!(failed.side_effects[0].status, "apply-failed-no-write");
}
