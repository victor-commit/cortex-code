//! Flow 1 contextual inspectors (VAL-V3-F1-035..042).

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
        "token_count=",
        "ctx %",
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "private or internal trace leaked: {banned}"
        );
    }
}

fn assert_transcript_and_composer(text: &str) {
    assert!(
        text.contains("Daily work · current conversation"),
        "origin must remain recognizable beside the inspector"
    );
    assert!(
        text.contains("Find where the server applies the rate limit")
            || text.contains("rate limit"),
        "active turn request must stay visible"
    );
    assert!(
        text.contains("shared request guard") || text.contains("request guard"),
        "conversation transcript content must remain readable"
    );
    assert!(
        text.contains("›")
            && (text.contains("Also list the tests")
                || text.contains("follow-up")
                || text.contains("Type a follow-up")),
        "composer must remain recognizable"
    );
    assert!(
        !text.contains("ctx 6%")
            && !text.contains("context %")
            && !text.contains("$0.02"),
        "statusline must not carry context percentage or cost"
    );
}

fn audit_all_five() -> [mockup_harness_v3::audit::AuditDocument; 5] {
    [
        audit_screen("sidebar-context", "views.sidebar.context").expect("context"),
        audit_screen("sidebar-files", "views.sidebar.files").expect("files"),
        audit_screen("sidebar-lsp", "views.sidebar.lsp").expect("lsp"),
        audit_screen("sidebar-todo", "views.sidebar.todo").expect("todo"),
        audit_screen("sidebar-subagents", "views.sidebar.subagents").expect("subagents"),
    ]
}

#[test]
fn every_inspector_keeps_transcript_composer_and_shared_origin() {
    for audit in audit_all_five() {
        audit.validate().expect("inspector audit must validate");
        let text = audit.render_txt();
        assert_transcript_and_composer(&text);
        assert_no_private_traces(&text);
        assert!(
            text.contains("Context")
                && text.contains("Files")
                && text.contains("LSP")
                && text.contains("Todo")
                && text.contains("Subs"),
            "short contextual tab labels must be present: {}",
            audit.screen_id
        );
        assert!(
            text.contains("[esc]") || text.contains("Return"),
            "inspector must expose an explicit return to the conversation"
        );
        let conversation = audit
            .conversation
            .as_ref()
            .expect("inspectors must audit conversation identity");
        assert_eq!(conversation.conversation_id, "conversation.daily-work");
        assert_eq!(conversation.transcript_id, "transcript.daily-work");
        assert_eq!(conversation.session_id, "session.daily-work.1");
        assert_eq!(conversation.turn_id, "turn.daily-work.inspector.18");
        assert!(conversation.user_task.contains("rate limit"));
        assert!(conversation.visible_origin.contains("Daily work"));
        assert!(
            audit.transitions.iter().any(|transition| {
                transition.from_screen_id == audit.screen_id
                    && (transition.action_id.contains("close")
                        || transition.action_id.contains("return")
                        || transition.action_id.contains("cancel")
                        || transition.action_id.contains("back"))
                    && transition.to_screen_id == "chat-turn"
            }),
            "close/back/cancel must restore the originating conversation shell"
        );
    }
}

#[test]
fn context_inspector_shows_useful_capacity_without_statusline_tokens() {
    let audit = audit_screen("sidebar-context", "views.sidebar.context").expect("context");
    audit.validate().expect("context audit");
    let text = audit.render_txt();
    assert!(
        text.contains("middleware.rs")
            || text.contains("src/auth")
            || text.contains("retry_policy"),
        "relevant context files must appear"
    );
    assert!(
        text.contains("used")
            || text.contains("capacity")
            || text.contains("space left")
            || text.contains("of "),
        "concise capacity summary required"
    );
    assert!(
        !text.contains("prompt_tokens")
            && !text.contains("token_budget")
            && !text.contains("tokens/"),
        "raw token internals must not dominate"
    );
    let status = text
        .lines()
        .last()
        .expect("canvas ends with statusline");
    assert!(
        !status.contains('%') && !status.to_lowercase().contains("token"),
        "statusline must not show context percentage"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "inspector-context"),
        "context inspector events required"
    );
}

#[test]
fn files_inspector_shows_hierarchy_and_change_state_without_color_only_cues() {
    let audit = audit_screen("sidebar-files", "views.sidebar.files").expect("files");
    audit.validate().expect("files audit");
    let text = audit.render_txt();
    assert!(
        text.contains("src") && (text.contains("auth") || text.contains("middleware")),
        "scannable hierarchy required"
    );
    assert!(
        text.contains("modified") || text.contains("changed"),
        "modified state must be readable as text"
    );
    assert!(
        text.contains("unchanged") || text.contains("same"),
        "unchanged state must be readable as text"
    );
    assert!(
        text.contains("modified") && (text.contains('·') || text.contains('+') || text.contains('-')),
        "change facts must accompany modified files"
    );
    let modified_events = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "inspector-file"
                && event.identity.subject_id.contains("modified")
        })
        .count();
    let unchanged_events = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "inspector-file"
                && event.identity.subject_id.contains("unchanged")
        })
        .count();
    assert!(modified_events >= 2, "audit must count modified files");
    assert!(unchanged_events >= 1, "audit must count unchanged files");
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "inspector-file-summary"),
        "file summary event required"
    );
}

#[test]
fn lsp_inspector_lists_diagnostics_errors_first_with_message() {
    let audit = audit_screen("sidebar-lsp", "views.sidebar.lsp").expect("lsp");
    audit.validate().expect("lsp audit");
    let text = audit.render_txt();
    let error_pos = text
        .find("error")
        .or_else(|| text.find("Error"))
        .expect("severity severity required");
    let warn_pos = text
        .find("warning")
        .or_else(|| text.find("Warning"))
        .expect("warning severity required");
    assert!(error_pos < warn_pos, "errors must sort before warnings");
    assert!(
        text.contains(':') || text.contains("line ") || text.chars().any(|c| c.is_ascii_digit()),
        "diagnostic locations required"
    );
    assert!(
        text.contains("capacity")
            || text.contains("unused")
            || text.contains("must")
            || text.contains("import")
            || text.contains("clone"),
        "plain-English diagnostic message required"
    );
    let errors = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "inspector-diagnostic"
                && event.identity.subject_id.contains("error")
        })
        .count();
    let warnings = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "inspector-diagnostic"
                && event.identity.subject_id.contains("warning")
        })
        .count();
    assert_eq!(errors, 1, "exactly one error diagnostic");
    assert_eq!(warnings, 2, "exactly two warning diagnostics");
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "inspector-diagnostic-summary"),
        "diagnostic summary event required"
    );
}

#[test]
fn todo_inspector_mirrors_active_todo_list_truth() {
    let sidebar = audit_screen("sidebar-todo", "views.sidebar.todo").expect("sidebar todo");
    let full = audit_screen("todo-list", "session.todo.baseline").expect("todo-list");
    sidebar.validate().expect("sidebar todo audit");
    full.validate().expect("todo-list audit");

    let sidebar_text = sidebar.render_txt();
    let full_text = full.render_txt();
    assert!(
        sidebar_text.contains("3 of 5") && full_text.contains("3 of 5"),
        "both states must share the 3 of 5 summary"
    );
    assert!(
        sidebar_text.contains("1 in progress") && full_text.contains("1 in progress"),
        "in-progress counts must match"
    );
    assert!(
        sidebar_text.contains("1 pending") && full_text.contains("1 pending"),
        "pending counts must match"
    );
    assert!(
        sidebar_text.contains("Add tests") || sidebar_text.contains("login_ratelimit"),
        "current item must match the active work"
    );
    assert!(
        sidebar_text.contains("Run cargo test") || sidebar_text.contains("verify"),
        "pending item must remain visible"
    );

    let done = sidebar
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item"
                && (event.identity.subject_id.contains("done")
                    || event.identity.subject_id.contains("complete"))
        })
        .count();
    let current = sidebar
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item"
                && (event.identity.subject_id.contains("current")
                    || event.identity.subject_id.contains("progress"))
        })
        .count();
    let pending = sidebar
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "todo-item" && event.identity.subject_id.contains("pending")
        })
        .count();
    assert_eq!(done, 3);
    assert_eq!(current, 1);
    assert_eq!(pending, 1);
}

#[test]
fn subagents_inspector_summarizes_delegation_without_worker_internals() {
    let audit = audit_screen("sidebar-subagents", "views.sidebar.subagents").expect("subagents");
    audit.validate().expect("subagents audit");
    let text = audit.render_txt();
    assert!(
        text.contains("running") || text.contains("Running"),
        "running state required"
    );
    assert!(
        text.contains("done") || text.contains("completed") || text.contains("Complete"),
        "completed state required"
    );
    assert!(
        text.contains("queued") || text.contains("Queued") || text.contains("waiting"),
        "queued state required"
    );
    assert!(
        text.contains("auth") || text.contains("middleware") || text.contains("find"),
        "delegated purpose must be human-readable"
    );
    assert!(
        !text.contains("worker_id")
            && !text.contains("wave")
            && !text.contains("spawn_tree")
            && !text.contains("tool_envelope"),
        "worker internals must stay hidden"
    );
    let rows = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "inspector-subagent")
        .count();
    assert_eq!(rows, 3, "three delegated purpose rows");
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "inspector-subagent-summary"),
        "subagent summary event required"
    );
}

#[test]
fn inspector_tab_switch_preserves_transcript_draft_and_scroll() {
    for case_id in [
        "inspector-tab-files",
        "inspector-tab-lsp",
        "inspector-tab-todo",
        "inspector-tab-subagents",
    ] {
        let replay = replay_case(case_id, 1).expect(case_id);
        replay.validate().expect("tab switch validation");
        assert!(
            replay.before.screen_id.starts_with("sidebar-"),
            "tab switch starts from an inspector"
        );
        assert!(
            replay.after.screen_id.starts_with("sidebar-"),
            "tab switch stays on an inspector"
        );
        assert_ne!(
            replay.before.screen_id, replay.after.screen_id,
            "tab change updates only the selected inspector panel"
        );
        for field in [
            "draft",
            "scroll_anchor",
            "user_task",
            "visible_origin",
            "conversation_id",
            "transcript_id",
            "session_id",
            "turn_id",
        ] {
            let hashes = replay
                .preserved_field_hashes
                .get(field)
                .unwrap_or_else(|| panic!("{case_id} must preserve {field}"));
            assert_eq!(
                hashes.before, hashes.after,
                "{case_id} must keep {field} stable across tabs"
            );
        }
        let text = replay.audit.render_txt();
        assert_transcript_and_composer(&text);
    }
}

#[test]
fn inspector_close_restores_origin_conversation_and_draft() {
    for case_id in [
        "inspector-close-context",
        "inspector-close-files",
        "inspector-close-lsp",
        "inspector-close-todo",
        "inspector-close-subagents",
    ] {
        let replay = replay_case(case_id, 1).expect(case_id);
        replay.validate().expect("close validation");
        assert!(
            replay.before.screen_id.starts_with("sidebar-"),
            "close starts from an inspector"
        );
        assert_eq!(
            replay.after.screen_id, "chat-turn",
            "close must restore the originating conversation shell, not home"
        );
        assert_ne!(
            replay.after.screen_id, "home",
            "close must never send the user to home"
        );
        for field in ["draft", "scroll_anchor", "user_task", "visible_origin", "turn_id"] {
            let hashes = replay
                .preserved_field_hashes
                .get(field)
                .unwrap_or_else(|| panic!("{case_id} must preserve {field}"));
            assert_eq!(
                hashes.before, hashes.after,
                "{case_id} must keep {field} after close"
            );
        }
        let text = replay.audit.render_txt();
        assert!(
            text.contains("Daily work · current conversation"),
            "returned conversation must show the same origin"
        );
        assert!(
            text.contains("rate limit") || text.contains("Find where the server"),
            "returned conversation must keep the same turn"
        );
        assert!(
            text.contains("Also list the tests")
                || text.contains(replay.after.draft.as_str())
                || text.contains("›"),
            "composer draft must remain after close"
        );
        assert!(
            !text.contains("Context · Files · LSP")
                && !text.contains("sidebar"),
            "inspector chrome must not remain after close"
        );
    }
}
