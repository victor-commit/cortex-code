//! Flow 2 queue inspector and temporary help (VAL-V3-F2-045..048).

use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn assert_no_private_traces(text: &str) {
    for banned in [
        "chain-of-thought",
        "hidden prompt",
        "provider payload",
        "system prompt",
        "tool_envelope",
        "wave_id",
        "score=",
        "token_count=",
        "ctx %",
        "snake_case_status",
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "private or internal trace leaked: {banned}"
        );
    }
}

fn assert_statusline_minimal(text: &str) {
    let status = text.lines().last().expect("statusline");
    assert!(
        status.contains("~/project") && status.contains("Claude Opus 4.8"),
        "minimal statusline required: {status}"
    );
    assert!(
        !status.contains('%') && !status.contains('$') && !status.contains("idle"),
        "statusline must omit cost/context/idle"
    );
}

#[test]
fn sidebar_queue_preserves_transcript_composer_and_active_queue_context() {
    let audit = audit_screen("sidebar-queue", "views.sidebar.queue")
        .expect("sidebar-queue must produce typed render evidence");
    audit
        .validate()
        .expect("sidebar-queue evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);

    // VAL-V3-F2-045: narrowed transcript + one composer + active Queue context.
    assert!(
        text.contains("Input and approval") || text.contains("paused conversation"),
        "origin must stay recognizable beside the queue inspector"
    );
    assert!(
        text.contains("retry path") || text.contains("Check the retry"),
        "paused conversation transcript must remain readable"
    );
    assert!(
        text.contains("›")
            && (text.contains("Check the retry path in")
                || text.contains("Keep the summary short")),
        "composer must remain recognizable with the preserved draft"
    );
    assert!(
        text.contains("Queue") || text.contains("Queued"),
        "Queue must identify as the active context"
    );
    assert!(
        text.contains("Also list unit tests") && text.contains("Then summarize risks"),
        "ordered queued prompt summaries must appear"
    );
    assert!(
        text.contains("1") && text.contains("2"),
        "queue order must be scannable as plain text"
    );
    assert!(
        text.contains("[next]") || text.contains("runs next") || text.contains("next"),
        "next-state text must identify which prompt runs next"
    );
    assert!(
        text.contains("queue-item-1") && text.contains("queue-item-2"),
        "shared queue identities must match queue-manager"
    );
    assert!(
        text.contains("rev=1") || text.contains("rev 1"),
        "queue revisions must be readable"
    );
    assert!(
        !text.contains("enter send") && !text.contains("? help") && !text.contains("/ commands"),
        "sidebar-queue must not grow a global hint strip"
    );

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "active-context"
            && event.identity.subject_id.to_lowercase().contains("queue")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "queue-item" && event.identity.subject_id.contains("queue-item-1")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "queue-item" && event.identity.subject_id.contains("queue-item-2")
    }));
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "queue-next"
                || event.identity.subject_id.contains("runs-next")
                || event.identity.subject_id.contains("next")
        }),
        "queue-next event required"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "sidebar-queue"
                && transition.to_screen_id == "queue-manager"
                && transition.action_id.contains("manage")
        }),
        "open manage/edit path to queue-manager required"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "sidebar-queue"
                && (transition.action_id.contains("close")
                    || transition.action_id.contains("return")
                    || transition.action_id.contains("cancel")
                    || transition.action_id.contains("back"))
                && (transition.to_screen_id == "chat-turn"
                    || transition.to_screen_id == "composer-multiline")
        }),
        "close/back/cancel must restore the conversation origin"
    );

    let conversation = audit
        .conversation
        .as_ref()
        .expect("sidebar-queue must audit conversation identity");
    assert_eq!(conversation.conversation_id, "conversation.retry-review");
    assert!(conversation.user_task.contains("retry path"));
}

#[test]
fn sidebar_queue_and_manager_share_identities_and_reconcile_round_trips() {
    // Consistency of static identities / order / next marker across surfaces.
    let sidebar = audit_screen("sidebar-queue", "views.sidebar.queue").expect("sidebar");
    let manager = audit_screen("queue-manager", "flow2-queue-manager").expect("manager");
    let side_text = sidebar.render_txt();
    let man_text = manager.render_txt();
    for fragment in [
        "queue-item-1",
        "queue-item-2",
        "Also list unit tests",
        "Then summarize risks",
        "[attachment: queue-notes-a.md]",
        "[attachment: queue-notes-b.md]",
    ] {
        assert!(
            side_text.contains(fragment) && man_text.contains(fragment),
            "queue surfaces must share {fragment}"
        );
    }

    let apply = replay_case("sidebar-queue-manage-apply", 1).expect("manage apply");
    apply
        .validate()
        .expect("sidebar→manager→sidebar apply must validate");
    assert_eq!(apply.before.screen_id, "sidebar-queue");
    assert_eq!(apply.after.screen_id, "sidebar-queue");
    assert!(
        apply.before.queue.contains("queue-item-1")
            && apply.before.queue.contains("queue-item-2"),
        "opened queue identities required"
    );
    assert_ne!(
        apply.before.queue, apply.after.queue,
        "apply must persist the shown reorder against shared identities/revisions"
    );
    assert!(
        apply.after.queue.starts_with("1:queue-item-2")
            || apply.after.queue.contains("1:queue-item-2:rev=2"),
        "applied order must prefer item-2 first / next"
    );
    assert_eq!(
        apply.before.draft, apply.after.draft,
        "apply must not submit or clear the draft"
    );
    assert!(
        apply
            .preserved_field_hashes
            .get("draft")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false),
        "draft hash must stay stable across apply"
    );
    assert!(
        apply
            .preserved_field_hashes
            .get("scroll_anchor")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false),
        "transcript scroll must stay stable across apply"
    );
    assert_eq!(apply.actions[0].action_id, "action.queue-manager.apply");

    // VAL-V3-F2-046: public --case after-state paint must mirror after.queue next marker.
    // Audit already reorders item-2 first; stale default sidebar bytes are a real fail.
    let apply_paint = apply.audit.render_txt();
    assert!(
        apply_paint.contains("queue-item-2") && apply_paint.contains("[next]"),
        "sidebar-queue-manage-apply paint must show item-2 as [next] after apply"
    );
    let item2_next = apply_paint
        .lines()
        .any(|line| line.contains("queue-item-2") && line.contains("[next]"));
    let item1_still_next = apply_paint
        .lines()
        .any(|line| line.contains("queue-item-1") && line.contains("[next]"));
    assert!(
        item2_next && !item1_still_next,
        "apply after-state must mark only item-2 [next], not retain default item-1 [next]\n{apply_paint}"
    );
    assert!(
        apply.audit.fixture_id.contains("applied")
            || apply.audit.fixture_id.contains("after-apply")
            || apply.audit.fixture_id.contains("manage-apply"),
        "apply after-state fixture must own the applied queue order: {}",
        apply.audit.fixture_id
    );

    let cancel = replay_case("sidebar-queue-manage-cancel", 1).expect("manage cancel");
    cancel
        .validate()
        .expect("sidebar→manager→sidebar cancel must validate");
    assert_eq!(cancel.before.screen_id, "sidebar-queue");
    assert_eq!(cancel.after.screen_id, "sidebar-queue");
    assert_eq!(
        cancel.before.queue, cancel.after.queue,
        "cancel restores the exact opened queue order"
    );
    assert_eq!(cancel.before.draft, cancel.after.draft);
    assert_eq!(cancel.before.selection, cancel.after.selection);
    assert_eq!(cancel.before.scroll_anchor, cancel.after.scroll_anchor);
    assert!(
        cancel.after.selection.to_lowercase().contains("queue")
            || cancel.after.focus.to_lowercase().contains("queue")
            || cancel.after.selection.contains("queue-item"),
        "cancel must restore active Queue context selection"
    );
    assert_eq!(cancel.actions[0].action_id, "action.queue-manager.cancel");

    // After apply, the sidebar applied fixture must show the new next marker.
    let applied = audit_screen("sidebar-queue", "views.sidebar.queue.applied")
        .expect("applied sidebar fixture");
    let applied_text = applied.render_txt();
    assert!(
        applied_text.contains("queue-item-2") && applied_text.contains("[next]"),
        "applied sidebar must show item-2 as next"
    );
    assert!(
        applied_text.contains("Then summarize risks"),
        "applied sidebar still lists reordered prompts"
    );
}

#[test]
fn help_is_temporary_detail_with_task_grouped_shortcuts_and_return() {
    let audit =
        audit_screen("help", "views.help.open").expect("help must produce typed render evidence");
    audit.validate().expect("help evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);

    // VAL-V3-F2-047
    assert!(
        text.contains("Help") || text.contains("Shortcut reference"),
        "temporary help reference title required"
    );
    assert!(
        text.contains("From")
            || text.contains("origin")
            || text.contains("paused conversation")
            || text.contains("composer"),
        "originating context must be named"
    );
    // Task-grouped curated actions.
    for group in ["Write", "Navigate", "Manage", "Inspect"] {
        assert!(
            text.contains(group)
                || text.to_lowercase().contains(&group.to_lowercase())
                || text.contains("Compose")
                || text.contains("Find")
                || text.contains("Queue")
                || text.contains("Review"),
            "help must group human-readable actions by task (missing group signal near {group})"
        );
    }
    // Includes palette trigger and curated shortcuts (VAL-V3-DS-008: lowercase chords).
    assert!(
        text.contains("ctrl+k"),
        "palette trigger must appear as lowercase chord ctrl+k (VAL-V3-DS-008)"
    );
    assert!(
        text.contains("ctrl+r") && text.contains("ctrl+f"),
        "help review/search chords must be lowercase ctrl+r and ctrl+f"
    );
    assert!(
        !text.contains("Ctrl+K")
            && !text.contains("Ctrl+R")
            && !text.contains("Ctrl+F")
            && !text.contains("Ctrl-K")
            && !text.contains("Ctrl-R")
            && !text.contains("Ctrl-F"),
        "uppercase Ctrl+* chords are forbidden; chords must be lowercase"
    );
    assert!(
        text.contains("/help") || text.contains("Help"),
        "help entry itself should remain discoverable"
    );
    assert!(
        text.contains("[esc]") || text.contains("Return"),
        "one obvious Return action required"
    );
    assert!(
        text.contains("Filter") || text.contains("filter") || text.contains("Type to filter"),
        "local filter field required"
    );
    assert!(
        text.contains("only place for grouped shortcuts")
            || text.contains("temporary reference is the only place"),
        "help must remain the sole multi-shortcut reference surface"
    );

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "detail-origin"
            || event.identity.family == "origin"
            || event.identity.subject_id.contains("origin")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "help-group"
            || event.identity.family == "action-group"
            || event.identity.subject_id.contains("group")
    }));
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "help"
                && (transition.action_id.contains("return")
                    || transition.action_id.contains("cancel")
                    || transition.action_id.contains("back")
                    || transition.action_id.contains("close"))
                && (transition.to_screen_id == "composer-multiline"
                    || transition.to_screen_id == "chat-turn"
                    || transition.to_screen_id == "sidebar-queue")
        }),
        "Return must leave the temporary help view"
    );

    let conversation = audit
        .conversation
        .as_ref()
        .expect("help must audit conversation identity");
    assert!(
        conversation.later_draft.is_some()
            || conversation.submitted_prompt.is_some()
            || !conversation.user_task.is_empty(),
        "help must preserve work context identity"
    );
}

#[test]
fn help_filter_no_results_and_return_preserve_prior_state() {
    let filtered = audit_screen("help", "views.help.filtered").expect("filtered help");
    filtered.validate().expect("filtered help audit");
    let filtered_text = filtered.render_txt();
    assert!(
        filtered_text.contains("queue") || filtered_text.contains("Queue"),
        "filtered help must retain the visible query"
    );
    assert!(
        filtered_text.contains("Queue") || filtered_text.contains("/queue"),
        "matching help actions must remain"
    );

    let empty = audit_screen("help", "views.help.no-results").expect("no-results help");
    empty.validate().expect("no-results help audit");
    let empty_text = empty.render_txt();
    assert!(
        empty_text.contains("No matching")
            || empty_text.contains("no results")
            || empty_text.contains("No results")
            || empty_text.contains("nothing matched"),
        "explicit recoverable no-results state required"
    );
    assert!(
        empty_text.contains("zzz") || empty_text.contains("filter"),
        "query must be retained in the no-results state"
    );

    let returned = replay_case("help-return", 1).expect("help return");
    returned.validate().expect("help return valid");
    assert_eq!(returned.before.screen_id, "help");
    assert_eq!(returned.after.screen_id, "composer-multiline");
    assert_eq!(returned.before.draft, returned.after.draft);
    assert!(!returned.after.draft.is_empty());
    assert_eq!(
        returned.before.scroll_anchor, returned.after.scroll_anchor,
        "transcript scroll must restore"
    );
    assert!(
        returned.after.focus.contains("draft")
            || returned.after.focus.contains("cursor")
            || returned.after.focus.contains("composer"),
        "focus restoration should land on the prior composer control/cursor family: {}",
        returned.after.focus
    );
    assert!(
        returned
            .preserved_field_hashes
            .get("draft")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false),
        "draft hash must stay byte-stable on Return"
    );
    assert!(
        returned
            .preserved_field_hashes
            .get("scroll_anchor")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false)
    );

    let cancelled = replay_case("help-cancel", 1).expect("help cancel");
    cancelled.validate().expect("help cancel valid");
    assert_eq!(cancelled.before.screen_id, "help");
    assert_eq!(cancelled.after.screen_id, "composer-multiline");
    assert_eq!(cancelled.before.draft, cancelled.after.draft);

    let from_inspector = replay_case("help-return-to-sidebar-queue", 1)
        .expect("return to inspector origin");
    from_inspector
        .validate()
        .expect("inspector-origin help return must validate");
    assert_eq!(from_inspector.before.screen_id, "help");
    assert_eq!(from_inspector.after.screen_id, "sidebar-queue");
    assert_eq!(from_inspector.before.draft, from_inspector.after.draft);
    assert!(
        from_inspector
            .after
            .selection
            .to_lowercase()
            .contains("queue")
            || from_inspector.after.focus.to_lowercase().contains("queue"),
        "return must restore the open Queue inspector context"
    );

    let filter_case = replay_case("help-filter-no-results", 1).expect("filter scenario");
    filter_case.validate().expect("filter scenario valid");
    assert_eq!(filter_case.before.screen_id, "help");
    assert_eq!(filter_case.after.screen_id, "help");
    assert!(
        filter_case.after.selection.contains("no-results")
            || filter_case.after.focus.contains("filter")
            || filter_case.after.selection.contains("filter"),
        "filter no-results must keep help open with recoverable filter state"
    );
}

#[test]
fn public_selector_publishes_sidebar_queue_and_help_pairs() {
    for (screen, category) in [("sidebar-queue", "views"), ("help", "views")] {
        let output = harness()
            .args([screen])
            .output()
            .expect("public selector must run");
        assert!(
            output.status.success(),
            "{screen} must publish after renderer preflight:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for extension in ["txt", "png"] {
            assert!(
                Path::new(&format!(
                    "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.{extension}"
                ))
                .is_file(),
                "canonical {screen}.{extension} must publish"
            );
        }
    }
}
