use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn multiline_composer_keeps_authored_lines_distinct_from_wrapping_with_a_visible_cursor() {
    let audit = audit_screen("composer-multiline", "flow2-multiline")
        .expect("the multiline composer must produce typed render evidence");
    audit
        .validate()
        .expect("multiline composer evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert!(text.contains("Draft · 3 authored lines"));
    assert!(text.contains("wraps visually"));
    assert!(text.contains("cursor at character 30"));
    assert!(text.contains("Check the retry path in @retry"));
    assert!(text.contains("Then explain the failure path that still needs coverage before the merge;"));
    assert!(text.contains("Keep the summary short before we merge."));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "composer-newline")
            .count(),
        3,
        "each authored line must have explicit evidence rather than being inferred from wrapping"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "cursor"
            && event.identity.subject_id == "draft-offset-30"
            && event.identity.render_refs[0].row == 33
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.composer-multiline.open-file-mention"
            && transition.to_screen_id == "file-mention"
            && transition.kind == "direct"
    }));
}

#[test]
fn file_mention_uses_the_visible_query_for_fuzzy_matches_and_one_selected_best_result() {
    let audit = audit_screen("file-mention", "flow2-file-mention")
        .expect("the file mention dock must produce typed render evidence");
    audit
        .validate()
        .expect("file mention evidence must stay bijective");

    let text = audit.render_txt();
    assert!(text.contains("@retry"));
    assert!(text.contains("3 matching project files"));
    assert!(text.contains("› src/net/retry_policy.rs"));
    assert!(text.contains("src/net/retry_queue.rs"));
    assert!(text.contains("tests/net/retry_policy_test.rs"));
    assert!(text.contains("project-relative paths"));
    assert!(text.contains("[tab] Insert file  [esc] Dismiss"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "file-match")
            .count(),
        3,
        "every rendered candidate must be a query match"
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1,
        "the file dock must have one selected result"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "filter"
            && event.identity.subject_id == "query-retry"
            && event.identity.detail_level == "focused"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.file-mention.insert-selected"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.file-mention.dismiss"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn mention_insert_and_dismiss_preserve_the_exact_multiline_origin() {
    let inserted = replay_case("file-mention-insert", 1)
        .expect("inserting the selected file must be replayable");
    let dismissed = replay_case("file-mention-dismiss", 1)
        .expect("dismissing the mention dock must be replayable");
    let no_results = replay_case("file-mention-no-results", 1)
        .expect("a nonmatching visible query must be replayable");

    for replay in [&inserted, &dismissed] {
        replay
            .validate()
            .expect("mention replay must expose one valid action and transition");
        assert_eq!(replay.before.screen_id, "file-mention");
        assert_eq!(replay.after.screen_id, "composer-multiline");
        assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
        assert_eq!(
            replay.before.scroll_anchor, replay.after.scroll_anchor,
            "temporary mention work must not move the conversation"
        );
        assert_eq!(
            replay.before.return_policy, replay.after.return_policy,
            "mention return policy must remain stable"
        );
        assert_eq!(replay.attempts[0].count, 1);
        assert!(replay.side_effects[0].simulated);
    }

    assert_eq!(
        dismissed.before.draft, dismissed.after.draft,
        "dismiss must restore the typed mention literally"
    );
    assert_eq!(
        dismissed.before.focus, dismissed.after.focus,
        "dismiss must restore focus to the original cursor"
    );
    assert_eq!(
        dismissed.before.selection, dismissed.after.selection,
        "dismiss must restore the original mention span"
    );
    for field in ["draft", "selection", "focus", "composer_height", "scroll_anchor"] {
        let hashes = dismissed
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "dismiss must preserve the original {field}"
        );
    }
    assert_ne!(
        inserted.before.draft, inserted.after.draft,
        "insert must replace the active mention span"
    );
    assert_eq!(
        inserted
            .before
            .draft
            .replace("@retry", "@src/net/retry_policy.rs"),
        inserted.after.draft,
        "insert may only replace the active mention fragment"
    );
    assert_eq!(
        inserted.before.draft.replace("@retry", ""),
        inserted.after.draft.replace("@src/net/retry_policy.rs", ""),
        "all surrounding multiline draft text and line breaks must remain exact"
    );
    let surrounding_hashes = inserted
        .preserved_field_hashes
        .get("draft_surrounding")
        .expect("insert must record a hash for unchanged draft surroundings");
    assert_eq!(
        surrounding_hashes.before, surrounding_hashes.after,
        "only the active mention span may change during insertion"
    );
    assert_eq!(
        inserted.before.transcript, inserted.after.transcript,
        "file insertion must not mutate the paused transcript"
    );
    assert_eq!(inserted.before.focus, "draft-cursor-30");
    assert_eq!(inserted.after.focus, "draft-cursor-48");
    assert_eq!(inserted.before.composer_height, "5-visual-rows");
    assert_eq!(
        inserted.before.composer_height, inserted.after.composer_height,
        "replacing the mention span must not change the preserved multiline composer height"
    );
    assert_eq!(
        inserted.before.queued_follow_up, inserted.after.queued_follow_up,
        "file insertion must not alter queued work"
    );
    assert_eq!(no_results.before.screen_id, "file-mention");
    assert_eq!(no_results.after.screen_id, "file-mention");
    assert_eq!(no_results.after.selection, "no-result");
    assert!(no_results.after.draft.contains("@zzq"));
    assert!(no_results.audit.render_txt().contains("@zzq"));
    assert!(no_results
        .audit
        .render_txt()
        .contains("No project files match @zzq."));
}

#[test]
fn public_single_screen_selectors_publish_the_two_flow2_canonical_pairs() {
    for screen in ["composer-multiline", "file-mention"] {
        let output = harness()
            .arg(screen)
            .output()
            .expect("the public canonical selector must run");
        assert!(
            output.status.success(),
            "{screen} must publish after renderer preflight:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for extension in ["txt", "png"] {
            assert!(
                Path::new(&format!(
                    "/projects/cortex-code/plans/assets/mockups-v3/input/{screen}.{extension}"
                ))
                .is_file(),
                "the canonical {screen}.{extension} artifact must publish"
            );
        }
    }
}
