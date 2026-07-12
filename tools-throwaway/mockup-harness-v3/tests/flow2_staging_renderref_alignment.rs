//! Focused regression: after multiline hunk expansion shifts staging chrome,
//! semantic action RenderRefs must sit on the same visible rows and cover the
//! same label text (include-all / edit / Finish / Cancel).

use mockup_harness_v3::{
    audit::{audit_screen, sha256_hex, AuditDocument},
    scenario::{diff_fixture_file_bytes, replay_case},
};

const STAGING_ACTIONS: [(&str, &str); 4] = [
    ("action.staging.include-all", "Include all"),
    ("action.staging.edit", "Edit"),
    ("action.staging.finish", "Finish"),
    ("action.staging.cancel", "Cancel"),
];

// Historical fixture identity for src/net/retry_policy.rs buried payloads.
const RETRY_GUARD_CONTENT_HASH: &str =
    "5ce43b2c003fa210c5b162b2189b98bde598b6251f0d60e2ba165bcdd9d637ed";
const RETRY_TEST_CONTENT_HASH: &str =
    "26b123444f81674df20c13aac678ae6ae5131fca2aae0b87e1f9d49612de1658";
const WORKING_TREE_BEFORE: &str =
    "76b7cd0894f4f75fa6a254393c49130771f81fc34eb5ac54994dee779ac06277";
const WORKING_TREE_AFTER_INCLUDED_GUARD: &str =
    "38d52616305288b0c9e74521f212dad096df1934a59ea1be71d60fc37c1e0125";
const WORKING_TREE_AFTER_BOTH: &str =
    "dd2f26ba071511814b7f4c22bfc5ddfb2ceb7c74fa1980eff69030b197f26fc4";
const GIT_INDEX_UNCHANGED: &str =
    "50a0c336ac7d7a54a241c0d4b842cd49efd519b42bbc11378b7e86116ea6ca42";

fn chrome_row_text(audit: &AuditDocument, row: u16) -> String {
    audit
        .styled_cells
        .iter()
        .filter(|cell| cell.row == row)
        .map(|cell| cell.glyph)
        .collect()
}

fn action_event<'a>(
    audit: &'a AuditDocument,
    action_id: &str,
) -> &'a mockup_harness_v3::audit::SemanticEvent {
    audit
        .events
        .iter()
        .find(|event| {
            event
                .action
                .as_ref()
                .is_some_and(|action| action.action_id == action_id)
        })
        .unwrap_or_else(|| panic!("missing semantic action {action_id}"))
}

fn cell_slice(audit: &AuditDocument, row: u16, start: u16, end: u16) -> String {
    audit
        .styled_cells
        .iter()
        .filter(|cell| cell.row == row && cell.column >= start && cell.column < end)
        .map(|cell| cell.glyph)
        .collect()
}

#[test]
fn staging_chrome_action_render_refs_match_visible_label_geometry() {
    let audit = audit_screen("diff-viewer-staging", "flow2-staging")
        .expect("staging detail must produce typed render evidence");
    audit
        .validate()
        .expect("staging audit bijection must stay valid");

    let text = audit.render_txt();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "staging canvas stays fixed at 40 rows");

    // Multiline hunks expand into the body; chrome lives below the payloads.
    let chrome_row = rows
        .iter()
        .position(|row| row.contains("[a] Include all") && row.contains("[enter] Finish"))
        .expect("staging chrome must remain visible after payload expansion")
        as u16;
    assert!(
        chrome_row >= 20,
        "chrome must sit under the expanded multiline hunk body, found row {chrome_row}"
    );
    assert!(
        rows[chrome_row as usize].contains("[e] Edit")
            && rows[chrome_row as usize].contains("[esc] Cancel"),
        "chrome must expose include-all/edit/Finish/Cancel on one row"
    );

    for (action_id, label) in STAGING_ACTIONS {
        let event = action_event(&audit, action_id);
        let action = event.action.as_ref().expect("action evidence");
        assert_eq!(action.label, label);
        assert_eq!(
            event.identity.render_refs.len(),
            1,
            "{action_id} must bind one visible render ref"
        );
        let reference = &event.identity.render_refs[0];
        assert_eq!(
            reference.row, chrome_row,
            "{action_id} audit row must track the visible chrome row after multiline expansion"
        );

        let visible = chrome_row_text(&audit, reference.row);
        // Render columns are char-cells; never use UTF-8 byte indexes for geometry.
        let label_start = visible
            .char_indices()
            .map(|(byte, _)| byte)
            .zip(0u16..)
            .find(|(byte, _)| visible[*byte..].starts_with(label))
            .map(|(_, col)| col)
            .unwrap_or_else(|| panic!("label {label:?} must appear on chrome row {chrome_row}"));
        let label_end = label_start + label.chars().count() as u16;
        assert!(
            reference.start <= label_start && reference.end >= label_end,
            "{action_id} ref [{}..{}) must cover label {label:?} at [{label_start}..{label_end}) on row {chrome_row}; cells={:?}",
            reference.start,
            reference.end,
            cell_slice(&audit, reference.row, reference.start, reference.end)
        );
        let fragment = cell_slice(&audit, reference.row, reference.start, reference.end);
        assert!(
            fragment.contains(label),
            "{action_id} cells must contain {label:?}, got {fragment:?}"
        );
    }
}

#[test]
fn staging_action_geometry_survives_include_all_and_apply_failure_variants() {
    for fixture in [
        "flow2-staging",
        "case.staging-include-all.tick.1",
        "case.staging-apply-partial-failure.tick.1",
    ] {
        let audit = audit_screen("diff-viewer-staging", fixture)
            .unwrap_or_else(|error| panic!("staging fixture {fixture} must render: {error}"));
        let chrome_row = audit
            .render_txt()
            .lines()
            .position(|row| row.contains("[a] Include all") && row.contains("[enter] Finish"))
            .unwrap_or_else(|| panic!("fixture {fixture} must keep staging chrome"))
            as u16;
        for (action_id, label) in STAGING_ACTIONS {
            let event = action_event(&audit, action_id);
            let reference = &event.identity.render_refs[0];
            assert_eq!(
                reference.row, chrome_row,
                "{fixture}/{action_id} must follow shifted chrome row {chrome_row}"
            );
            let fragment = cell_slice(&audit, reference.row, reference.start, reference.end);
            assert!(
                fragment.contains(label),
                "{fixture}/{action_id} geometry must cover {label:?}, got {fragment:?}"
            );
        }
    }
}

#[test]
fn staging_renderref_realignment_preserves_buried_hunk_payload_identity() {
    let before_bytes = diff_fixture_file_bytes();
    assert_eq!(sha256_hex(&before_bytes), WORKING_TREE_BEFORE);

    let finish = replay_case("staging-finish", 1).expect("staging finish must replay");
    let apply_all = replay_case("diff-apply-all", 1).expect("apply-all must replay");
    let finish_diff = finish.diff.as_ref().expect("finish diff evidence");
    let all_diff = apply_all.diff.as_ref().expect("apply-all diff evidence");

    for diff in [finish_diff, all_diff] {
        assert_eq!(diff.hunks.len(), 2);
        assert_eq!(diff.hunks[0].hunk_id, "hunk.retry-guard");
        assert_eq!(diff.hunks[1].hunk_id, "hunk.retry-test");
        assert_eq!(
            diff.hunks[0].old_payload,
            vec![
                "    return retry_after".to_owned(),
                "    backoff = backoff + 1".to_owned(),
            ]
        );
        assert_eq!(
            diff.hunks[0].new_payload,
            vec![
                "    return retry_after + jitter".to_owned(),
                "    backoff = backoff + jitter".to_owned(),
                "    log_retry_window(jitter)".to_owned(),
            ]
        );
        assert_eq!(
            diff.hunks[1].old_payload,
            vec![
                "assert_eq!(wait, 4)".to_owned(),
                "assert_eq!(attempts, 1)".to_owned(),
            ]
        );
        assert_eq!(
            diff.hunks[1].new_payload,
            vec![
                "assert_eq!(wait, 5)".to_owned(),
                "assert_eq!(attempts, 1)".to_owned(),
            ]
        );
        assert_eq!(diff.hunks[0].content_hash, RETRY_GUARD_CONTENT_HASH);
        assert_eq!(diff.hunks[1].content_hash, RETRY_TEST_CONTENT_HASH);
        assert_eq!(diff.working_tree_before, WORKING_TREE_BEFORE);
        assert_eq!(diff.git_index_before, GIT_INDEX_UNCHANGED);
        assert_eq!(diff.git_index_after, GIT_INDEX_UNCHANGED);
    }

    assert_eq!(
        finish_diff.working_tree_after,
        WORKING_TREE_AFTER_INCLUDED_GUARD
    );
    assert_eq!(all_diff.working_tree_after, WORKING_TREE_AFTER_BOTH);
}
