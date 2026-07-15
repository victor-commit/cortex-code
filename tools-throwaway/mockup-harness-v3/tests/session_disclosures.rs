//! Session progressive-disclosure tests for reasoning, tool results, tasks, and compact history.

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
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "private or internal trace leaked: {banned}"
        );
    }
}

#[test]
fn reasoning_collapsed_shows_concise_summary_and_expand_without_private_traces() {
    let audit = audit_screen("reasoning-collapsed", "session.disclosure.reasoning")
        .expect("reasoning-collapsed must render");
    audit.validate().expect("reasoning-collapsed audit");

    let text = audit.render_txt();
    assert!((text.contains("Current conversation") || text.contains("Daily work · current conversation")));
    assert!(text.contains(
        "Find where the server applies the rate limit and explain the safe path."
    ));
    assert!(
        text.contains("Why this answer") || text.contains("Reasoning"),
        "collapsed reasoning needs a concise rationale summary"
    );
    assert!(
        text.contains("Expand") || (text.contains("Enter to") || text.contains("[enter]")),
        "collapsed reasoning must offer an expand control"
    );
    assert!(
        text.contains("shared guard") || text.contains("The shared request guard"),
        "the final answer must remain fully readable beside collapsed rationale"
    );
    assert!(
        !text.contains("HTTP 500") || text.contains("shared"),
        "collapsed body must not dump long private traces"
    );
    assert_no_private_traces(&text);
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id.contains("reasoning")
            && transition.from_screen_id == "reasoning-collapsed"
            && transition.to_screen_id == "reasoning"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "disclosure"
            && event.identity.detail_level == "summary"
            && event.action.is_some()
    }));
}

#[test]
fn reasoning_expands_beside_the_turn_with_concise_rationale_only() {
    let audit = audit_screen("reasoning", "session.disclosure.reasoning")
        .expect("reasoning must render");
    audit.validate().expect("reasoning audit");

    let text = audit.render_txt();
    assert!(text.contains("You · Find where the server applies the rate limit"));
    assert!(text.contains("Collapse") || (text.contains("Enter to") || text.contains("[enter]")));
    assert!(
        text.contains("applied by both") || text.contains("shared guard"),
        "answer must stay visible while rationale is expanded"
    );
    assert!(
        text.contains("before route work")
            || text.contains("Login and API")
            || text.contains("same safety rule"),
        "expanded content must be a concise user-facing rationale"
    );
    assert_no_private_traces(&text);
    assert!(audit.transitions.iter().any(|transition| {
        transition.from_screen_id == "reasoning"
            && transition.to_screen_id == "reasoning-collapsed"
    }));
}

#[test]
fn reasoning_disclosure_round_trips_preserve_draft_answer_and_scroll() {
    let expand = replay_case("reasoning-expand", 1).expect("reasoning expand");
    expand.validate().expect("expand validation");
    assert_eq!(expand.before.screen_id, "reasoning-collapsed");
    assert_eq!(expand.after.screen_id, "reasoning");
    for field in ["draft", "transcript", "scroll_anchor"] {
        let hashes = expand
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must round-trip");
    }

    let collapse = replay_case("reasoning-collapse", 1).expect("reasoning collapse");
    collapse.validate().expect("collapse validation");
    assert_eq!(collapse.before.screen_id, "reasoning");
    assert_eq!(collapse.after.screen_id, "reasoning-collapsed");
    for field in ["draft", "transcript", "scroll_anchor"] {
        let hashes = collapse
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must round-trip");
    }
}

#[test]
fn tool_calls_form_one_ordered_activity_narrative_with_mixed_outcomes() {
    let audit = audit_screen("tool-calls", "session.disclosure.tools")
        .expect("tool-calls must render");
    audit.validate().expect("tool-calls audit");

    let text = audit.render_txt();
    let read_pos = text
        .find("Read session rules")
        .expect("first tool must appear");
    let fail_pos = text
        .find("Run rate-limit tests")
        .expect("failed tool must appear");
    let search_pos = text
        .find("Search project code")
        .expect("running or third tool must appear");
    assert!(
        read_pos < fail_pos && fail_pos < search_pos,
        "tools must stay in execution order"
    );
    assert!(
        text.contains("Complete") || text.contains("✓"),
        "success must be text-readable"
    );
    assert!(
        text.contains("Needs attention") || text.contains("✗") || text.contains("Failed"),
        "failure must be text-readable"
    );
    assert!(
        text.contains("Running") || text.contains("●") || text.contains("in progress"),
        "incomplete work must be text-readable"
    );
    assert!(
        text.contains("Retry")
            || text.contains("retry")
            || text.contains("Inspect")
            || text.contains("r to retry"),
        "failed tool must expose a nearby recovery action"
    );
    assert!(text.contains("Reply to Code") || text.contains("Reply"));
    assert_no_private_traces(&text);

    let activities = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "activity")
        .count();
    assert!(activities >= 3, "each tool needs its own activity owner");
}

#[test]
fn tool_success_expand_opens_focused_detail_and_round_trips() {
    let expand = replay_case("tool-success-expand", 1).expect("tool success expand");
    expand.validate().expect("expand validation");
    assert_eq!(expand.before.screen_id, "tool-success");
    assert_eq!(expand.after.screen_id, "tool-success-expanded");
    assert_eq!(
        expand.actions[0].action_id,
        "action.tool-success.expand"
    );

    let expanded = expand.audit.render_txt();
    assert!(expanded.contains("Search project code"));
    assert!(
        expanded.contains("middleware")
            || expanded.contains("rate_limit")
            || expanded.contains("matches"),
        "expanded state must add focused result detail"
    );
    assert!(expanded.contains("Reply to Code") || expanded.contains("Continue"));
    assert!(expanded.contains("Collapse") || expanded.contains("Enter to collapse") || expanded.contains("Enter to"));

    let collapse = replay_case("tool-success-collapse", 1).expect("tool success collapse");
    collapse.validate().expect("collapse validation");
    assert_eq!(collapse.before.screen_id, "tool-success-expanded");
    assert_eq!(collapse.after.screen_id, "tool-success");
    for field in ["draft", "transcript", "scroll_anchor"] {
        let hashes = collapse
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must round-trip");
    }

    let again = replay_case("tool-success-expand", 1).expect("repeat expand");
    again.validate().expect("repeat expand validation");
    assert_eq!(again.after.screen_id, "tool-success-expanded");
}

#[test]
fn tool_task_summarizes_delegated_work_without_orchestration_internals() {
    let audit = audit_screen("tool-task", "session.disclosure.tasks")
        .expect("tool-task must render");
    audit.validate().expect("tool-task audit");

    let text = audit.render_txt();
    assert!(text.contains("Audit the auth module") || text.contains("audit"));
    assert!(
        text.contains("Queued")
            || text.contains("Running")
            || text.contains("Complete")
            || text.contains("Blocked")
            || text.contains("Needs attention"),
        "compact tasks must show a readable state"
    );
    assert!(
        text.contains("elapsed") || text.contains("s ·") || text.contains("progress"),
        "compact tasks must include progress or elapsed work"
    );
    for banned in ["wave", "score ladder", "tool_envelope", "worker_id", "branch policy"] {
        assert!(
            !text.to_lowercase().contains(banned),
            "compact task must hide orchestration internals: {banned}"
        );
    }
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "activity" && event.identity.detail_level == "summary"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.from_screen_id == "tool-task" && transition.to_screen_id == "tool-task-expanded"
    }));
}

#[test]
fn tool_task_expanded_keeps_siblings_and_attributable_findings() {
    let audit = audit_screen("tool-task-expanded", "session.disclosure.tasks")
        .expect("tool-task-expanded must render");
    audit.validate().expect("tool-task-expanded audit");

    let text = audit.render_txt();
    assert!(
        text.contains("finding")
            || text.contains("Refresh tokens")
            || text.contains("revoked")
            || text.contains("session"),
        "expanded task must show useful findings"
    );
    assert!(
        text.contains("rate-limit") || text.contains("Add rate") || text.contains("build"),
        "sibling task state must remain visible or recoverable"
    );
    assert!(text.contains("Collapse") || (text.contains("Enter to") || text.contains("[enter]")));
    assert_no_private_traces(&text);

    let expand = replay_case("tool-task-expand", 1).expect("task expand");
    expand.validate().expect("task expand validation");
    assert_eq!(expand.before.screen_id, "tool-task");
    assert_eq!(expand.after.screen_id, "tool-task-expanded");

    let collapse = replay_case("tool-task-collapse", 1).expect("task collapse");
    collapse.validate().expect("task collapse validation");
    assert_eq!(collapse.after.screen_id, "tool-task");
}

#[test]
fn tool_task_states_remain_readable_without_color() {
    let compact = audit_screen("tool-task", "session.disclosure.tasks")
        .expect("tool-task")
        .render_txt();
    let expanded = audit_screen("tool-task-expanded", "session.disclosure.tasks")
        .expect("tool-task-expanded")
        .render_txt();
    for text in [compact, expanded] {
        let states = ["Queued", "Running", "Complete", "Blocked", "Needs attention", "Failed"];
        let found = states.iter().filter(|state| text.contains(*state)).count()
            + text.matches('✓').count()
            + text.matches('✗').count()
            + text.matches('●').count()
            + text.matches('○').count();
        assert!(
            found >= 2,
            "task states must be distinguishable in monochrome text"
        );
    }
}

#[test]
fn compact_preserves_continuity_and_full_history_statement() {
    let audit = audit_screen("compact", "session.disclosure.compact")
        .expect("compact must render");
    audit.validate().expect("compact audit");

    let text = audit.render_txt();
    assert!(
        text.contains("compacted") || text.contains("Compact"),
        "visible compaction statement required"
    );
    assert!(
        text.contains("full") && (text.contains("preserved") || text.contains("history")),
        "full history preservation must be stated"
    );
    assert!(
        text.contains("Next") || text.contains("next"),
        "compact summary needs a next step"
    );
    assert!(
        text.contains("Expand") || text.contains("full transcript"),
        "compact must offer history restore"
    );
    assert!(text.contains("resume") || text.contains("Reply") || text.contains("composer") || text.contains("›"));
    assert_no_private_traces(&text);
}

#[test]
fn compact_history_round_trips_without_losing_turns_or_draft() {
    let expand = replay_case("compact-expand-history", 1).expect("compact expand");
    expand.validate().expect("history expand validation");
    assert_eq!(expand.before.screen_id, "compact");
    assert_eq!(expand.after.screen_id, "chat-long-scroll");
    let draft = expand
        .preserved_field_hashes
        .get("draft")
        .expect("draft preserved on expand");
    assert_eq!(draft.before, draft.after, "draft must survive expand");
    assert!(
        expand
            .preserved_field_hashes
            .contains_key("history_order"),
        "history order evidence must be recorded"
    );
    let history = expand
        .preserved_field_hashes
        .get("history_order")
        .expect("history_order");
    assert_eq!(
        history.before, history.after,
        "turn order must survive expand"
    );
    assert!(
        expand.audit.render_txt().contains("Turn")
            || expand.audit.render_txt().contains("history")
            || expand.audit.render_txt().contains("You ·"),
        "expanded history must restore prior turns"
    );

    let collapse = replay_case("compact-collapse-history", 1).expect("compact collapse");
    collapse.validate().expect("history collapse validation");
    assert_eq!(collapse.before.screen_id, "chat-long-scroll");
    assert_eq!(collapse.after.screen_id, "compact");
    let draft = collapse
        .preserved_field_hashes
        .get("draft")
        .expect("draft preserved");
    assert_eq!(draft.before, draft.after);
    let summary = collapse.audit.render_txt();
    assert!(
        summary.contains("compacted") || summary.contains("Compact"),
        "returning must restore the compact summary"
    );
}
