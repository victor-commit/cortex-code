//! Session shell, todo progress, and long-history navigation (VAL-V3-F1-029..034).

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

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
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "private or internal trace leaked: {banned}"
        );
    }
}

#[test]
fn shell_keeps_commands_and_output_inline_under_originating_turn() {
    let audit = audit_screen("shell", "session.shell.inline")
        .expect("shell must render");
    audit.validate().expect("shell audit");

    let text = audit.render_txt();
    assert!(
        (text.contains("Current conversation") || text.contains("Daily work · current conversation")),
        "shell stays in the conversation origin"
    );
    assert!(
        text.contains("Find where the server applies the rate limit")
            || text.contains("check the branch"),
        "originating request must remain visible"
    );

    let first = text
        .find("git status")
        .expect("first command must appear inline");
    let second = text
        .find("cargo test")
        .expect("second command must appear inline");
    assert!(first < second, "commands must stay in execution order");

    assert!(
        text.contains("exit 0")
            || text.contains("Complete")
            || text.contains("✓"),
        "completed command status must be readable without color"
    );
    assert!(
        text.contains("Running")
            || text.contains("●")
            || text.contains("in progress"),
        "live command status must be readable without color"
    );
    assert!(
        text.contains("master") || text.contains("ahead") || text.contains("Compiling"),
        "command output must stay inline under the turn"
    );
    assert!(
        !text.contains("Terminal dashboard") && !text.contains("open terminal"),
        "shell must not become a separate terminal destination"
    );
    assert_no_private_traces(&text);

    assert!(
        audit.events.iter().any(|event| event.identity.family == "activity"),
        "shell activity events must be present"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "shell"
                && (transition.action_id.contains("interrupt")
                    || transition.action_id.contains("complete"))
        }),
        "shell must declare safe exit transitions"
    );
}

#[test]
fn shell_interrupt_restores_composer_and_preserves_captured_output() {
    let replay = replay_case("shell-interrupt", 1).expect("shell interrupt");
    replay.validate().expect("shell interrupt validation");
    assert_eq!(replay.before.screen_id, "shell");
    assert_eq!(replay.after.screen_id, "shell");

    let text = replay.audit.render_txt();
    assert!(
        text.contains("Interrupted") || text.contains("stopped") || text.contains("✗"),
        "interrupted status must remain visible"
    );
    assert!(
        text.contains("git status") || text.contains("cargo test"),
        "captured command identity must remain"
    );
    assert!(
        text.contains("Compiling")
            || text.contains("Finished")
            || text.contains("master")
            || text.contains("ahead"),
        "captured output must not be deleted"
    );
    assert!(
        text.contains("›") && (text.contains("Reply") || text.contains("follow-up") || text.contains("Send")),
        "composer must return after interrupt"
    );

    for field in ["draft", "scroll_anchor"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved on interrupt"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive interrupt");
    }
    assert!(
        replay.preserved_field_hashes.contains_key("captured_output"),
        "captured output hash evidence is required"
    );
    let captured = replay
        .preserved_field_hashes
        .get("captured_output")
        .expect("captured_output");
    assert_eq!(
        captured.before, captured.after,
        "captured output must not change on interrupt"
    );
}

#[test]
fn shell_completion_restores_composer_with_exit_status() {
    let replay = replay_case("shell-complete", 1).expect("shell complete");
    replay.validate().expect("shell complete validation");
    assert_eq!(replay.before.screen_id, "shell");
    assert_eq!(replay.after.screen_id, "shell");

    let text = replay.audit.render_txt();
    assert!(
        text.contains("exit 0") || text.contains("Complete") || text.contains("✓"),
        "completion must leave a readable exit or success status"
    );
    assert!(
        text.contains("cargo test") || text.contains("git status"),
        "completed command remains inline"
    );
    assert!(
        text.contains("›")
            && (text.contains("Reply")
                || text.contains("follow-up")
                || text.contains("Send")
                || text.contains("continue")),
        "composer must return after completion"
    );
    for field in ["draft", "captured_output"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved on complete"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive completion");
    }
}

#[test]
fn todo_list_progress_reconciles_item_states_with_summary_counts() {
    let audit = audit_screen("todo-list", "session.todo.baseline")
        .expect("todo-list must render");
    audit.validate().expect("todo-list audit");

    let text = audit.render_txt();
    assert!(
        (text.contains("Current conversation") || text.contains("Daily work · current conversation")),
        "todo stays in conversation context"
    );
    assert!(
        text.contains("rate limit") || text.contains("login") || text.contains("implement"),
        "originating work remains visible"
    );

    // Ordered plan items with plain-English states.
    let done_markers = text.matches('✓').count()
        + text.matches("done").count()
        + text.matches("Complete").count()
        + text.matches("Done").count();
    let running_markers = text.matches('●').count()
        + text.matches("in progress").count()
        + text.matches("In progress").count()
        + text.matches("Running").count();
    let pending_markers = text.matches('○').count()
        + text.matches("pending").count()
        + text.matches("Pending").count();
    assert!(done_markers >= 3, "completed items must be readable");
    assert!(running_markers >= 1, "exactly one current item must be readable");
    assert!(pending_markers >= 1, "pending work must be readable");

    assert!(
        text.contains("3 of 5")
            || text.contains("3/5")
            || (text.contains("3") && text.contains("done") && text.contains("5")),
        "overall progress summary must be visible"
    );
    assert!(
        text.contains("1 in progress")
            || text.contains("1 current")
            || (text.contains("in progress") && text.contains("pending")),
        "summary must reconcile current and pending counts"
    );
    assert!(
        text.contains("›") || text.contains("follow-up") || text.contains("Reply"),
        "composer must remain available"
    );
    assert!(
        !text.contains("Project board") && !text.contains("kanban"),
        "todo must not open a detached project-management view"
    );
    assert_no_private_traces(&text);

    // Audit facts reconcile item states with summary counts.
    let completed = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item"
                && (event.identity.subject_id.contains("done")
                    || event.identity.subject_id.contains("complete"))
        })
        .count();
    let current = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item"
                && (event.identity.subject_id.contains("current")
                    || event.identity.subject_id.contains("progress"))
        })
        .count();
    let pending = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item" && event.identity.subject_id.contains("pending")
        })
        .count();
    assert_eq!(completed, 3, "audit must mark exactly 3 completed items");
    assert_eq!(current, 1, "audit must mark exactly 1 current item");
    assert_eq!(pending, 1, "audit must mark exactly 1 pending item");
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "todo-summary"
                && event.identity.subject_id.contains("progress")
        }),
        "progress summary event required"
    );
}

#[test]
fn todo_progress_update_preserves_origin_request_and_composer() {
    let replay = replay_case("todo-progress", 1).expect("todo progress");
    replay.validate().expect("todo progress validation");
    assert_eq!(replay.before.screen_id, "todo-list");
    assert_eq!(replay.after.screen_id, "todo-list");

    for field in ["draft", "scroll_anchor", "user_task"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved on todo update"));
        assert_eq!(
            hashes.before, hashes.after,
            "{field} must stay stable while only todo state advances"
        );
    }

    let text = replay.audit.render_txt();
    assert!(
        text.contains("4 of 5")
            || text.contains("4/5")
            || (text.contains("4") && text.contains("done")),
        "after advance, summary counts must update"
    );
    assert!(
        text.contains("›") || text.contains("Reply") || text.contains("follow-up"),
        "composer remains available after progress"
    );
    assert!(
        text.contains("Daily work") || text.contains("rate limit") || text.contains("login"),
        "originating request context remains in place"
    );
}

#[test]
fn chat_long_scroll_signals_hidden_earlier_turns_and_navigation() {
    let audit = audit_screen("chat-long-scroll", "session.history.earlier")
        .expect("chat-long-scroll must render");
    audit.validate().expect("chat-long-scroll audit");

    let text = audit.render_txt();
    assert!(
        text.contains("earlier") || text.contains("Earlier"),
        "must indicate earlier turns exist outside the viewport"
    );
    assert!(
        text.contains("12") || text.contains("↑") || text.contains("turns"),
        "earlier-turn count or indicator required"
    );
    assert!(
        text.contains("scroll")
            || text.contains("Jump")
            || text.contains("earlier turns")
            || text.contains("ctrl"),
        "action to reach earlier turns must be present"
    );
    assert!(
        text.contains("You ·") || text.contains("Code ·") || text.contains("You ·"),
        "visible transcript remains readable"
    );
    assert!(
        !text.contains("████") || text.contains("You"),
        "history indicator must not obliterate transcript content"
    );
    assert_no_private_traces(&text);

    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "scroll"
                || event.identity.subject_id.contains("earlier")
                || event.identity.subject_id.contains("history")
        }),
        "scroll or history navigation events required"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "chat-long-scroll"
                && (transition.action_id.contains("earlier")
                    || transition.action_id.contains("latest")
                    || transition.action_id.contains("history")
                    || transition.action_id.contains("compact"))
        }),
        "history navigation transitions required"
    );
}

#[test]
fn chat_long_scroll_returns_to_latest_turn_preserving_draft() {
    let replay = replay_case("chat-long-scroll-return-latest", 1)
        .expect("return to latest");
    replay.validate().expect("return-latest validation");
    assert_eq!(replay.before.screen_id, "chat-long-scroll");
    assert_eq!(replay.after.screen_id, "chat-long-scroll");

    let draft = replay
        .preserved_field_hashes
        .get("draft")
        .expect("draft preserved on return to latest");
    assert_eq!(draft.before, draft.after, "draft must survive return to latest");

    let text = replay.audit.render_txt();
    assert!(
        text.contains("latest")
            || text.contains("newest")
            || text.contains("Jump to latest")
            || text.contains("near the latest")
            || text.contains("latest activity"),
        "landing near latest must be explicit or labeled"
    );
    assert!(
        text.contains("›")
            && (text.contains("Reply")
                || text.contains("follow-up")
                || text.contains("Send")
                || text.contains(replay.after.draft.as_str())
                || text.contains("Add a short note")),
        "composer draft must remain available at latest"
    );
    assert_eq!(
        replay.after.scroll_anchor, "latest-turn",
        "return must land at the deterministic latest anchor"
    );
}

#[test]
fn chat_long_scroll_jump_to_earlier_preserves_draft_and_anchor() {
    let replay = replay_case("chat-long-scroll-jump-earlier", 1)
        .expect("jump earlier");
    replay.validate().expect("jump earlier validation");
    assert_eq!(replay.before.screen_id, "chat-long-scroll");
    assert_eq!(replay.after.screen_id, "chat-long-scroll");

    let draft = replay
        .preserved_field_hashes
        .get("draft")
        .expect("draft preserved on jump earlier");
    assert_eq!(draft.before, draft.after);

    assert_eq!(
        replay.after.scroll_anchor, "earlier-turn-band",
        "earlier navigation must land at a deterministic earlier anchor"
    );
    let text = replay.audit.render_txt();
    assert!(
        text.contains("earlier") || text.contains("↑") || text.contains("Turn 1"),
        "earlier position must be visible after jump"
    );
    assert!(
        text.contains("latest") || text.contains("Jump to latest") || text.contains("newest"),
        "return-to-latest affordance must remain available from earlier position"
    );
}
