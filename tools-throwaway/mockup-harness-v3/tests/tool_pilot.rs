use mockup_harness_v3::{
    audit::audit_screen,
    scenario::replay_case,
};

#[test]
fn running_tool_is_one_compact_progress_row_with_local_stop_and_one_live_owner() {
    let audit = audit_screen("tool-running", "tool-pilot-running")
        .expect("the running tool fixture must render typed evidence");
    audit
        .validate()
        .expect("the running tool audit must remain semantically bijective");

    let text = audit.render_txt();
    let progress_row = text
        .lines()
        .find(|line| line.contains("Searching project code") || line.contains("Search project code"))
        .expect("the active search must have one compact visible row");
    assert!(
        progress_row.contains("Searching") || progress_row.contains("Running"),
        "live tool header must be present or progressive: {progress_row}"
    );
    assert!(progress_row.contains("128 of 400 files"));
    assert!(progress_row.contains("3 matches so far"));
    assert!(progress_row.contains("4s"));
    assert!(
        progress_row.contains("Esc to stop search"),
        "interrupt must be a local prose byline, not a bracket key atom"
    );
    assert!(
        !progress_row.contains("[s]"),
        "running tool row must not emit bracket stop chords"
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.contains("Searching project code") || line.contains("Search project code"))
            .count(),
        1,
        "the active tool must not split into duplicate running rows"
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "activity")
            .count(),
        1,
        "one active operation must have one activity owner"
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.tool-running.interrupt"
            && transition.to_screen_id == "chat-turn"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.tool-running.complete"
            && transition.to_screen_id == "tool-success"
    }));
}

#[test]
fn tool_interrupt_is_scoped_and_completion_replaces_running_evidence_once() {
    let interrupted = replay_case("tool-running-interrupt", 1)
        .expect("the local tool interruption must replay");
    interrupted
        .validate()
        .expect("the interruption trace must remain complete");
    assert_eq!(interrupted.before.screen_id, "tool-running");
    assert_eq!(interrupted.after.screen_id, "chat-turn");
    assert_eq!(
        interrupted.actions[0].action_id,
        "action.tool-running.interrupt"
    );
    assert_eq!(interrupted.attempts[0].owner_id, "activity.tool.search");
    assert!(
        interrupted.after.transcript.contains("Checked session rules"),
        "the completed sibling activity must remain in the same turn"
    );
    for field in ["draft", "transcript", "queued_follow_up"] {
        let hashes = interrupted
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} preservation must be recorded"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive interruption");
    }
    assert!(interrupted.audit.render_txt().contains("Reply to Code"));

    let completed = replay_case("tool-running-complete", 1)
        .expect("the automatic tool completion must replay");
    completed
        .validate()
        .expect("the completion trace must remain complete");
    assert_eq!(completed.before.screen_id, "tool-running");
    assert_eq!(completed.after.screen_id, "tool-success");
    assert!(completed.actions[0].automatic);
    assert_eq!(completed.actions[0].action_id, "action.tool-running.complete");
    assert_eq!(completed.attempts[0].count, 1);

    let success = completed.audit.render_txt();
    let result_row = success
        .lines()
        .find(|line| line.contains("Searched project code") || line.contains("Search project code"))
        .expect("the terminal result must remain inline");
    assert!(
        result_row.contains("Searched") || result_row.contains("Complete") || result_row.contains('✓'),
        "terminal tool row needs past verb or success mark: {result_row}"
    );
    assert!(result_row.contains("3 matches in 400 files"));
    assert!(result_row.contains("0.4s"));
    assert!(
        !success.lines().any(|line| line.contains("Searching project code")),
        "completion must remove stale live Searching evidence"
    );
    assert_eq!(
        success
            .lines()
            .filter(|line| line.contains("Searched project code") || line.contains("Search project code"))
            .count(),
        1,
        "completion must leave exactly one terminal replacement"
    );
}
