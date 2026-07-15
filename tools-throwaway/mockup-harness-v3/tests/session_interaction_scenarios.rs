//! Advanced conversation interaction scenarios (VAL-V3-F1-060..067).

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};
use std::collections::BTreeMap;

fn assert_no_private_traces(text: &str) {
    for banned in [
        "hidden prompt",
        "chain-of-thought",
        "provider payload",
        "system prompt",
        "internal event",
        "temperature=",
        "raw_logits",
        "tool_envelope",
        "wave_id",
        "score=",
        "snake_case_status",
        "api_key",
        "Bearer ",
    ] {
        assert!(
            !text.to_lowercase().contains(&banned.to_lowercase()),
            "private or internal trace leaked: {banned}"
        );
    }
}

fn assert_preserved(
    hashes: &BTreeMap<String, mockup_harness_v3::scenario::PreservedFieldHash>,
    fields: &[&str],
) {
    for field in fields {
        let entry = hashes
            .get(*field)
            .unwrap_or_else(|| panic!("{field} must be present in preservation hashes"));
        assert_eq!(
            entry.before, entry.after,
            "{field} must stay identical across the transition"
        );
    }
}

fn assert_hash_changed(
    hashes: &BTreeMap<String, mockup_harness_v3::scenario::PreservedFieldHash>,
    field: &str,
) {
    let entry = hashes
        .get(field)
        .unwrap_or_else(|| panic!("{field} must be present"));
    assert_ne!(
        entry.before, entry.after,
        "{field} is expected to change for this transition"
    );
}

/// VAL-V3-F1-060: interrupt keeps two distinct queued prompts and attachments.
#[test]
fn interrupt_preserves_queued_followups_identity_revision_hash_and_order() {
    for case_id in [
        "queued-interrupt-streaming",
        "queued-interrupt-tool",
        "queued-interrupt-shell",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|err| panic!("{case_id}: {err}"));
        replay
            .validate()
            .unwrap_or_else(|err| panic!("{case_id} invalid: {err}"));

        assert_eq!(replay.attempts[0].count, 1, "{case_id} resumes once");
        assert_eq!(
            replay.attempts[0].outcome, "interrupted",
            "{case_id} marks interruption once"
        );

        // Two ordered queue records with stable ids, revisions, hashes.
        let queue_before = &replay.before.queue;
        let queue_after = &replay.after.queue;
        assert_eq!(queue_before, queue_after, "{case_id} queue order+content");
        assert!(
            queue_before.contains("queue-item-1")
                && queue_before.contains("queue-item-2")
                && queue_before.contains("rev=1")
                && queue_before.contains("rev=2"),
            "{case_id} queue must encode identity and revision: {queue_before}"
        );
        let pos1 = queue_before
            .find("queue-item-1")
            .expect("queue-item-1 present");
        let pos2 = queue_before
            .find("queue-item-2")
            .expect("queue-item-2 present");
        assert!(pos1 < pos2, "{case_id} keeps original queued order");

        assert!(
            queue_before.contains("[attachment: queue-notes-a.md]")
                && queue_before.contains("[attachment: queue-notes-b.md]"),
            "{case_id} keeps both attachments with queued prompts"
        );
        assert_preserved(
            &replay.preserved_field_hashes,
            &[
                "queue",
                "queued_follow_up",
                "draft",
                "scroll_anchor",
                "partial_response",
                "session_id",
                "turn_id",
            ],
        );

        let text = replay.audit.render_txt();
        assert!(
            text.contains("Queued") || text.contains("queue-notes") || text.contains("follow-up"),
            "{case_id} after-state shows preserved queue/attachments"
        );
        assert_no_private_traces(&text);
    }
}

/// VAL-V3-F1-061: agent question opens an inline dock, submit resumes once, cancel keeps pause.
#[test]
fn agent_question_uses_anchored_decision_dock_with_single_resume() {
    let open = replay_case("agent-question-open", 1).expect("agent-question-open");
    open.validate().expect("agent-question-open validation");
    assert_eq!(open.before.screen_id, "streaming");
    assert_eq!(open.after.screen_id, "streaming");
    let open_text = open.audit.render_txt();
    assert!(
        open_text.contains("Which recovery path") || open_text.contains("Which option"),
        "inline question visible with the paused turn"
    );
    assert!(
        (open_text.contains("Submit answer") || open_text.contains("Enter to submit answer"))
            && (open_text.contains("Cancel") || open_text.contains("Esc to cancel")),
        "anchored dock must expose Submit answer and Cancel (prose byline or residual labels)"
    );
    assert!(
        open_text.contains("›") || (open_text.contains("Enter to") || open_text.contains("[enter]")),
        "focused response / primary dock affordance required"
    );
    assert_no_private_traces(&open_text);

    let submit = replay_case("agent-question-submit", 1).expect("agent-question-submit");
    submit.validate().expect("submit validation");
    assert_eq!(submit.attempts[0].count, 1, "submit resumes exactly once");
    assert_eq!(submit.attempts[0].outcome, "resumed");
    assert_preserved(
        &submit.preserved_field_hashes,
        &["draft", "session_id", "turn_id", "scroll_anchor", "queue"],
    );
    let submit_text = submit.audit.render_txt();
    assert!(
        (submit_text.contains("›") || submit_text.contains("Enter to send"))
            && (submit_text.contains("Reply")
                || submit_text.contains("follow-up")
                || submit_text.contains("Send")
                || submit_text.contains("send")
                || submit_text.contains("restored")
                || submit_text.contains("activity resumed")
                || submit_text.contains("Activity resumed")),
        "after submit the prior composer draft returns"
    );

    let cancel = replay_case("agent-question-cancel", 1).expect("agent-question-cancel");
    cancel.validate().expect("cancel validation");
    assert_eq!(cancel.after.pending_action.as_deref(), Some("activity.agent-question"));
    assert_eq!(cancel.attempts[0].outcome, "still-paused");
    assert_preserved(
        &cancel.preserved_field_hashes,
        &["draft", "session_id", "turn_id", "partial_response", "queue"],
    );
    let cancel_text = cancel.audit.render_txt();
    assert!(
        cancel_text.contains("Submit answer")
            || cancel_text.contains("Enter to submit answer")
            || cancel_text.contains("paused")
            || cancel_text.contains("Answer needed"),
        "cancel keeps the activity paused with the dock available"
    );
}

/// VAL-V3-F1-063: live updates never force scroll; New output jumps to latest.
#[test]
fn live_updates_preserve_reading_anchor_until_new_output_action() {
    let arrive = replay_case("reading-anchor-output-arrive", 1).expect("output-arrive");
    arrive.validate().expect("output-arrive validation");
    assert_preserved(
        &arrive.preserved_field_hashes,
        &["scroll_anchor", "draft", "session_id", "turn_id"],
    );
    let arrive_text = arrive.audit.render_txt();
    assert!(
        arrive_text.contains("New output"),
        "New output affordance appears when reading above latest content"
    );
    assert!(
        arrive_text.contains("rate-limit guard") || arrive_text.contains("earlier"),
        "reading anchor content remains visible"
    );
    assert_no_private_traces(&arrive_text);

    let jump = replay_case("reading-anchor-jump-latest", 1).expect("jump-latest");
    jump.validate().expect("jump validation");
    assert_hash_changed(&jump.preserved_field_hashes, "scroll_anchor");
    assert_preserved(&jump.preserved_field_hashes, &["draft", "session_id", "turn_id"]);
    let jump_text = jump.audit.render_txt();
    assert!(
        jump_text.contains("latest") || jump_text.contains("newest") || jump_text.contains("Below"),
        "jump lands at newest output"
    );
    assert!(
        !jump_text.contains("[enter] New output") && !jump_text.contains("› New output"),
        "after jump the affordance is no longer the reading guard"
    );
}

/// VAL-V3-F1-064: transcript search/copy stay local and never leak hidden rationale.
#[test]
fn transcript_search_and_copy_are_local_and_safe() {
    let search = replay_case("transcript-search-open", 1).expect("search-open");
    search.validate().expect("search validation");
    assert_preserved(
        &search.preserved_field_hashes,
        &["draft", "scroll_anchor", "session_id", "turn_id"],
    );
    let search_text = search.audit.render_txt();
    assert!(
        search_text.contains("Search") && search_text.contains("3 matches"),
        "deterministic match count is visible: {search_text}"
    );
    assert_no_private_traces(&search_text);

    let next = replay_case("transcript-search-next", 1).expect("search-next");
    next.validate().expect("next validation");
    assert_eq!(next.after.selection, "match-2");
    assert_preserved(&next.preserved_field_hashes, &["draft", "session_id"]);

    let none = replay_case("transcript-search-no-results", 1).expect("search-no-results");
    none.validate().expect("no-results validation");
    let none_text = none.audit.render_txt();
    assert!(
        none_text.contains("No matches") || none_text.contains("0 matches"),
        "recoverable no-results state"
    );
    assert_preserved(&none.preserved_field_hashes, &["draft", "scroll_anchor"]);

    let close = replay_case("transcript-search-close", 1).expect("search-close");
    close.validate().expect("close validation");
    assert_eq!(close.after.scroll_anchor, close.before.scroll_anchor);
    assert_preserved(
        &close.preserved_field_hashes,
        &["draft", "scroll_anchor", "turn_id"],
    );

    let copy = replay_case("transcript-copy-message", 1).expect("copy");
    copy.validate().expect("copy validation");
    assert_preserved(
        &copy.preserved_field_hashes,
        &["draft", "scroll_anchor", "turn_count", "transcript"],
    );
    let copy_text = copy.audit.render_txt();
    assert!(
        copy_text.contains("Copied") || copy_text.contains("copy confirmed"),
        "local confirmation near the message"
    );
    assert!(
        !copy_text.contains("Why this answer") || !copy_text.to_lowercase().contains("private"),
        "copy confirmation must not dump hidden rationale"
    );
    assert_no_private_traces(&copy_text);
    // Turn count evidence and copied-text sentinel stay out of private packages.
    let turn = copy
        .preserved_field_hashes
        .get("turn_count")
        .expect("turn_count");
    assert_eq!(turn.before, turn.after, "copy never adds a transcript turn");
    let copied = copy
        .preserved_field_hashes
        .get("copied_text")
        .expect("copied_text sentinel required");
    assert_eq!(
        copied.before, copied.after,
        "copied payload is the selected public message only"
    );
}

/// VAL-V3-F1-065: foreground interrupt vs scoped background stop.
#[test]
fn foreground_and_background_commands_have_separate_lifecycles() {
    let mixed = audit_screen("shell", "session.shell.mixed")
        .expect("mixed shell fixture must render");
    mixed.validate().expect("mixed shell audit");
    let mixed_text = mixed.render_txt();
    assert!(
        mixed_text.contains("Foreground") && mixed_text.contains("Background"),
        "shell must label foreground vs background lifecycle"
    );
    assert!(
        mixed_text.contains("Stop command")
            || mixed_text.contains("Stop foreground")
            || mixed_text.contains("Esc to stop command")
            || mixed_text.contains("Esc to stop"),
        "foreground has interrupt (prose Esc-to-stop or residual Stop command label)"
    );
    assert!(
        mixed_text.contains("Stop background") || mixed_text.contains("stop background"),
        "background has scoped stop"
    );
    assert_no_private_traces(&mixed_text);

    let fg = replay_case("shell-foreground-interrupt", 1).expect("fg interrupt");
    fg.validate().expect("fg interrupt validation");
    assert_eq!(fg.attempts[0].owner_id, "activity.shell.foreground-cargo");
    assert_eq!(fg.attempts[0].count, 1);
    assert_eq!(fg.attempts[0].outcome, "interrupted");
    assert_preserved(
        &fg.preserved_field_hashes,
        &["captured_output", "background_process", "draft"],
    );
    let fg_text = fg.audit.render_txt();
    assert!(
        fg_text.contains("Interrupted") || fg_text.contains("stopped"),
        "foreground shows interrupted"
    );
    assert!(
        fg_text.contains("Background") && (fg_text.contains("Running") || fg_text.contains("●")),
        "background continues after foreground interrupt"
    );

    let bg = replay_case("shell-background-stop", 1).expect("bg stop");
    bg.validate().expect("bg stop validation");
    assert_eq!(bg.attempts[0].owner_id, "activity.shell.background-watch");
    assert_eq!(bg.attempts[0].count, 1);
    assert_eq!(bg.attempts[0].outcome, "stopped");
    assert_preserved(
        &bg.preserved_field_hashes,
        &["captured_output", "foreground_process", "draft"],
    );
    let bg_text = bg.audit.render_txt();
    assert!(
        bg_text.contains("Background")
            && (bg_text.contains("Stopped") || bg_text.contains("stopped")),
        "background stop is explicit and scoped"
    );
}

/// VAL-V3-F1-066: delegated controls only mutate the selected sibling task.
#[test]
fn delegated_work_controls_are_sibling_scoped() {
    for case_id in [
        "tool-task-stop-selected",
        "tool-task-retry-selected",
        "tool-task-cancel-selected",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|err| panic!("{case_id}: {err}"));
        replay
            .validate()
            .unwrap_or_else(|err| panic!("{case_id} invalid: {err}"));
        assert_eq!(replay.attempts[0].count, 1, "{case_id} one invocation");
        assert!(
            replay.actions[0].label.contains("Stop")
                || replay.actions[0].label.contains("Retry")
                || replay.actions[0].label.contains("Cancel")
                || replay.actions[0].label.contains("Recover"),
            "{case_id} control labels the consequence: {}",
            replay.actions[0].label
        );
        assert_preserved(
            &replay.preserved_field_hashes,
            &[
                "sibling_task_hashes",
                "session_id",
                "turn_id",
                "draft",
                "scroll_anchor",
            ],
        );
        assert_hash_changed(&replay.preserved_field_hashes, "selected_task");
        let text = replay.audit.render_txt();
        assert!(
            text.contains("Audit the auth module")
                && text.contains("Build the test suite")
                && (text.contains("Review refresh") || text.contains("refresh")),
            "{case_id} keeps sibling tasks visible"
        );
        assert_no_private_traces(&text);
    }

    // Compact default and expanded both expose sibling state.
    for (screen, fixture) in [
        ("tool-task", "session.task.controls"),
        ("tool-task-expanded", "session.task.controls-expanded"),
    ] {
        let audit = audit_screen(screen, fixture).unwrap_or_else(|err| panic!("{screen}: {err}"));
        audit
            .validate()
            .unwrap_or_else(|err| panic!("{screen} audit: {err}"));
        let text = audit.render_txt();
        assert!(
            text.contains("Stop") || text.contains("Retry") || text.contains("Cancel"),
            "{screen} shows scoped controls with consequences"
        );
        assert!(
            text.contains("Audit") && text.contains("queued") || text.contains("Build"),
            "{screen} keeps ordered siblings"
        );
    }
}

/// VAL-V3-F1-067: large output stays bounded, opens full searchable detail, returns.
#[test]
fn large_output_is_bounded_searchable_copyable_and_reversible() {
    let compact = audit_screen("shell", "session.shell.large-output")
        .expect("large-output shell must render");
    compact.validate().expect("large-output compact audit");
    let compact_text = compact.render_txt();
    assert!(
        compact_text.contains("omitted")
            || compact_text.contains("of 240")
            || compact_text.contains("12 of"),
        "bounded summary states omitted count"
    );
    assert!(
        compact_text.contains("Full detail")
            || compact_text.contains("Show all")
            || compact_text.contains("View all"),
        "explicit full-detail action is present"
    );
    assert_no_private_traces(&compact_text);

    let open = replay_case("large-output-open", 1).expect("large-output-open");
    open.validate().expect("open validation");
    assert_eq!(open.before.screen_id, "shell");
    assert_eq!(open.after.screen_id, "shell");
    let open_text = open.audit.render_txt();
    assert!(
        open_text.contains("Search output") || open_text.contains("Search"),
        "full detail is searchable"
    );
    assert!(
        open_text.contains("Copy")
            || open_text.contains("[c]")
            || open_text.contains("c to copy")
            || open_text.contains("copy"),
        "full detail is copyable"
    );
    assert!(
        open_text.contains("Return")
            || open_text.contains("Collapse")
            || open_text.contains("Back")
            || open_text.contains("Esc to return")
            || open_text.contains("return"),
        "exact return to compact activity"
    );
    assert_preserved(
        &open.preserved_field_hashes,
        &["complete_output", "session_id", "turn_id", "draft"],
    );

    let search = replay_case("large-output-search", 1).expect("large-output-search");
    search.validate().expect("search validation");
    assert_eq!(search.after.selection, "output-match-1");
    assert_preserved(&search.preserved_field_hashes, &["complete_output", "draft"]);

    let close = replay_case("large-output-close", 1).expect("large-output-close");
    close.validate().expect("close validation");
    assert_eq!(close.after.focus, "large-output-open");
    assert_eq!(close.after.scroll_anchor, "shell-large-compact");
    assert_preserved(
        &close.preserved_field_hashes,
        &["complete_output", "draft", "session_id", "turn_id"],
    );
    let close_text = close.audit.render_txt();
    assert!(
        close_text.contains("omitted") || close_text.contains("of 240"),
        "return restores the compact bounded summary"
    );
}
