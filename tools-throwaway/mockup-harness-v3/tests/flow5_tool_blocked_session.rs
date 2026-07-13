//! Flow 5 tool-failed and blocked-command recovery.
//!
//! Owned contract assertions:
//! VAL-V3-F5-013 through VAL-V3-F5-020 and VAL-V3-F5-056.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const COMPLETED_TOOL: &str = "Read session rules";
const FAILED_TOOL: &str = "Write release notes patch";
const INTENDED_EFFECT: &str = "Update notes/release.md with the accepted retry wording";
const PARTIAL_OUTPUT: &str = "Prepared the first two bullet lines";
const MISSING_FILE_CAUSE: &str = "The repo map file is missing";
const CAPABILITY_NAME: &str = "Docs search";
const CAPABILITY_CAUSE: &str = "Docs search is unavailable until you connect it";
const UNKNOWN_CAUSE: &str = "The outcome of this tool is unknown";
const BLOCKED_COMMAND: &str = "rm -rf notes/archive && printf 'cleared\\n' >> notes/release.md";
const BLOCKED_POLICY_CAUSE: &str = "The current work style does not allow write commands";
const BLOCKED_PERMISSION_CAUSE: &str = "This command needs approval before it can run";
const DRAFT: &str = "Keep the risk note short before we publish.";

fn assert_no_raw_tool_detail(text: &str) {
    for banned in [
        "exit code",
        "Exit status",
        "stderr",
        "EACCES",
        "ENOENT",
        "MCP_TRANSPORT",
        "tool_id=",
        "permission_bitmask",
        "PolicyEnum",
        "DENY_CODE",
    ] {
        assert!(
            !text.contains(banned),
            "raw technical detail must stay disclosed-only: {banned}"
        );
    }
}

#[test]
fn tool_failed_preserves_invocation_partial_output_and_completed_siblings() {
    // VAL-V3-F5-013
    let audit = audit_screen("tool-failed", "canonical.tool-failed")
        .expect("tool-failed must render");
    audit.validate().expect("tool-failed audit");

    let text = audit.render_txt();
    assert!(text.contains(USER_PROMPT) || text.contains(RECOVERY_TASK));
    assert!(text.contains(COMPLETED_TOOL));
    assert!(text.contains("Complete") || text.contains("✓"));
    assert!(text.contains(FAILED_TOOL));
    assert!(text.contains(INTENDED_EFFECT));
    assert!(text.contains(PARTIAL_OUTPUT));
    assert!(text.contains(RECOVERY_ORIGIN));

    let completed_pos = text.find(COMPLETED_TOOL).expect("completed tool");
    let failed_pos = text.find(FAILED_TOOL).expect("failed tool");
    assert!(
        completed_pos < failed_pos,
        "earlier successful work must stay above the failure"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "activity"
                && event.identity.subject_id.contains("tool-write")),
        "typed activity must preserve the failed invocation identity"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "partial-output"
                || event.identity.subject_id.contains("partial")),
        "partial output must be typed in the audit"
    );
}

#[test]
fn tool_failed_explains_one_plain_cause_without_stderr() {
    // VAL-V3-F5-014
    let audit = audit_screen("tool-failed", "canonical.tool-failed")
        .expect("tool-failed cause must render");
    let text = audit.render_txt();
    assert!(text.contains(MISSING_FILE_CAUSE));
    assert_no_raw_tool_detail(&text);
    let cause_lines = text
        .lines()
        .filter(|line| line.contains(MISSING_FILE_CAUSE))
        .count();
    assert_eq!(cause_lines, 1, "exactly one plain cause sentence");
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "recovery-cause"),
        "cause requires a recovery-cause event"
    );
}

#[test]
fn tool_failed_offers_one_best_cause_specific_primary_action() {
    // VAL-V3-F5-015
    let missing = audit_screen("tool-failed", "canonical.tool-failed")
        .expect("missing-file tool-failed");
    let missing_text = missing.render_txt();
    assert!(missing_text.contains("› [enter] Choose a file"));
    assert!(missing_text.contains("[esc] Cancel") || missing_text.contains("[d] "));
    assert_eq!(
        missing
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(
        !missing_text.contains("› [enter] Retry tool"),
        "impossible generic Retry must not lead for a missing-file failure"
    );

    let capability = audit_screen("tool-failed", "tool-failed.capability")
        .expect("capability failure must render");
    let capability_text = capability.render_txt();
    assert!(capability_text.contains(CAPABILITY_NAME));
    assert!(capability_text.contains(CAPABILITY_CAUSE));
    assert!(
        capability_text.contains("› [enter] Connect Docs search")
            || capability_text.contains("› [enter] Connect capability")
    );
    assert!(!capability_text.to_lowercase().contains("transport"));
    assert!(!capability_text.to_lowercase().contains("endpoint"));
}

#[test]
fn tool_retry_respects_side_effect_boundaries_and_unknowns() {
    // VAL-V3-F5-016 and VAL-V3-F5-056
    let safe = replay_case("tool-failed-retry-safe", 1).expect("safe retry");
    assert_eq!(safe.before.screen_id, "tool-failed");
    assert_eq!(safe.after.screen_id, "tool-running");
    assert_eq!(safe.actions[0].action_id, "action.tool-failed.retry");
    assert_eq!(safe.attempts[0].count, 1);
    assert_eq!(safe.side_effects[0].status, "retry-no-duplicate");
    assert!(
        safe.before
            .paused_activity_id
            .contains("tool.write-release-notes")
            || safe.before.paused_activity_id.contains("tool-"),
        "retry must reuse the paused invocation identity"
    );
    for field in ["draft", "session_id", "transcript"] {
        let hashes = safe
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved across safe retry"));
        assert_eq!(hashes.before, hashes.after, "{field}");
    }

    let completed = replay_case("tool-failed-retry-completed-effect", 1)
        .expect("completed-effect boundary");
    assert_eq!(completed.before.screen_id, "tool-failed");
    assert_eq!(completed.after.screen_id, "tool-failed");
    assert_eq!(
        completed.actions[0].action_id,
        "action.tool-failed.inspect-result"
    );
    assert_eq!(completed.attempts[0].count, 0);
    assert_eq!(completed.side_effects[0].status, "no-retry-completed-effect");
    assert!(!completed.actions[0].label.to_lowercase().contains("retry"));

    let unknown = replay_case("tool-failed-unknown-outcome", 1).expect("unknown outcome");
    assert_eq!(unknown.before.screen_id, "tool-failed");
    assert_eq!(unknown.after.screen_id, "tool-failed");
    assert_eq!(
        unknown.actions[0].action_id,
        "action.tool-failed.inspect-result"
    );
    assert_eq!(unknown.attempts[0].count, 0);
    assert_eq!(unknown.side_effects[0].status, "inspection-required");
    let unknown_audit = unknown.audit.render_txt();
    assert!(unknown_audit.contains(UNKNOWN_CAUSE));
    assert!(unknown_audit.contains("› [enter] Inspect result"));
    assert!(
        !unknown_audit.contains("› [enter] Retry"),
        "unknown outcomes must never offer Retry as primary"
    );
    assert!(
        unknown
            .actions
            .iter()
            .all(|action| action.action_id != "action.tool-failed.retry" || !action.enabled),
        "Retry stays disabled until inspection"
    );
}

#[test]
fn capability_failure_routes_to_connection_with_resume_policy() {
    // VAL-V3-F5-017
    let open = replay_case("tool-failed-connect-capability", 1).expect("open connection");
    assert_eq!(open.before.screen_id, "tool-failed");
    assert_eq!(open.after.screen_id, "mcp");
    assert_eq!(
        open.actions[0].action_id,
        "action.tool-failed.connect-capability"
    );
    assert_eq!(open.before.user_task, RECOVERY_TASK);
    assert_eq!(open.after.user_task, RECOVERY_TASK);
    assert_eq!(open.before.session_id, open.after.session_id);
    assert_eq!(open.attempts[0].count, 0);

    let auto = replay_case("tool-failed-capability-return-automatic-once", 1)
        .expect("safe automatic resume once");
    assert_eq!(auto.before.screen_id, "mcp");
    assert_eq!(auto.after.screen_id, "tool-running");
    assert_eq!(auto.attempts[0].count, 1);
    assert_eq!(auto.side_effects[0].status, "automatic-once-resume");
    assert_eq!(auto.after.user_task, RECOVERY_TASK);

    let confirm = replay_case("tool-failed-capability-return-confirm", 1)
        .expect("side-effect resume requires confirmation");
    assert_eq!(confirm.before.screen_id, "mcp");
    assert_eq!(confirm.after.screen_id, "tool-failed");
    assert_eq!(
        confirm.actions[0].action_id,
        "action.tool-failed.confirm-resume"
    );
    assert_eq!(confirm.attempts[0].count, 0);
    assert_eq!(confirm.side_effects[0].status, "confirmation-required");

    let cancel = replay_case("tool-failed-capability-return-cancel", 1)
        .expect("cancel returns with zero invocation");
    assert_eq!(cancel.before.screen_id, "mcp");
    assert_eq!(cancel.after.screen_id, "tool-failed");
    assert_eq!(cancel.attempts[0].count, 0);
    assert_eq!(cancel.side_effects[0].status, "zero-invocation");

    let policy = replay_case("tool-failed-capability-policy-blocked", 1)
        .expect("policy-blocked recovery stays zero-invocation");
    assert_eq!(policy.after.screen_id, "tool-failed");
    assert_eq!(policy.attempts[0].count, 0);
    assert_eq!(policy.side_effects[0].status, "policy-blocked");
    let policy_text = policy.audit.render_txt();
    assert!(
        policy_text.contains("Review policy") || policy_text.contains("policy"),
        "policy-blocked path must stay on policy language"
    );
}

#[test]
fn blocked_command_keeps_command_draft_and_completed_work() {
    // VAL-V3-F5-018
    let audit = audit_screen("blocked-command", "canonical.blocked-command")
        .expect("blocked-command must render");
    audit.validate().expect("blocked-command audit");
    let text = audit.render_txt();
    assert!(text.contains(BLOCKED_COMMAND));
    assert!(text.contains(DRAFT));
    assert!(text.contains("did not run") || text.contains("was not run"));
    assert!(text.contains(COMPLETED_TOOL) || text.contains("Completed earlier"));
    assert!(!text.to_lowercase().contains("exit 1"));
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "blocked-command"
                || event.identity.subject_id.contains("command")),
        "blocked command must be typed"
    );
}

#[test]
fn blocked_command_states_one_policy_cause() {
    // VAL-V3-F5-019
    let audit = audit_screen("blocked-command", "canonical.blocked-command")
        .expect("blocked-command cause");
    let text = audit.render_txt();
    assert!(
        text.contains(BLOCKED_POLICY_CAUSE) || text.contains(BLOCKED_PERMISSION_CAUSE)
    );
    assert_no_raw_tool_detail(&text);
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "recovery-cause")
            .count(),
        1
    );
}

#[test]
fn blocked_command_routes_through_approval_or_work_style_and_returns() {
    // VAL-V3-F5-020
    let permission = replay_case("blocked-command-request-approval", 1)
        .expect("permission route");
    assert_eq!(permission.before.screen_id, "blocked-command");
    assert_eq!(permission.after.screen_id, "permission-shell");
    assert_eq!(
        permission.actions[0].action_id,
        "action.blocked-command.request-approval"
    );
    assert_eq!(permission.attempts[0].count, 0);
    assert_eq!(permission.side_effects[0].status, "zero-execution");
    assert!(permission.before.requested_command.contains("rm -rf"));
    assert_eq!(
        permission.before.requested_command,
        permission.after.requested_command
    );

    let permission_return = replay_case("blocked-command-approval-return", 1)
        .expect("approval return");
    assert_eq!(permission_return.before.screen_id, "permission-shell");
    assert_eq!(permission_return.after.screen_id, "blocked-command");
    assert_eq!(
        permission_return.actions[0].action_id,
        "action.blocked-command.return-for-run"
    );
    assert_eq!(permission_return.attempts[0].count, 0);
    assert_eq!(
        permission_return.side_effects[0].status,
        "awaiting-explicit-run"
    );
    assert!(permission_return.audit.render_txt().contains(BLOCKED_COMMAND));

    let work_style = replay_case("blocked-command-change-work-style", 1)
        .expect("configuration route");
    assert_eq!(work_style.before.screen_id, "blocked-command");
    assert_eq!(work_style.after.screen_id, "agent-picker");
    assert_eq!(
        work_style.actions[0].action_id,
        "action.blocked-command.change-work-style"
    );
    assert_eq!(work_style.attempts[0].count, 0);

    let work_style_return = replay_case("blocked-command-work-style-return", 1)
        .expect("work-style return");
    assert_eq!(work_style_return.before.screen_id, "agent-picker");
    assert_eq!(work_style_return.after.screen_id, "blocked-command");
    assert_eq!(work_style_return.attempts[0].count, 0);
    assert_eq!(
        work_style_return.side_effects[0].status,
        "awaiting-explicit-run"
    );
    assert!(work_style_return.audit.render_txt().contains(BLOCKED_COMMAND));
    assert_eq!(
        work_style_return.before.session_id,
        work_style_return.after.session_id
    );
}

#[test]
fn every_tool_and_blocked_recovery_bridge_preserves_paused_session_state() {
    for case_id in [
        "tool-failed-retry-safe",
        "tool-failed-connect-capability",
        "tool-failed-capability-return-cancel",
        "blocked-command-request-approval",
        "blocked-command-change-work-style",
        "blocked-command-approval-return",
        "blocked-command-work-style-return",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|error| panic!("{case_id}: {error}"));
        assert_eq!(
            replay.before.user_task, RECOVERY_TASK,
            "{case_id} keeps the task"
        );
        if case_id.starts_with("tool-failed") {
            assert_eq!(
                replay.before.visible_origin, RECOVERY_ORIGIN,
                "{case_id} keeps the origin"
            );
        }
        assert_eq!(
            replay.before.session_id, replay.after.session_id,
            "{case_id} never rebinds the session"
        );
        let draft = replay
            .preserved_field_hashes
            .get("draft")
            .unwrap_or_else(|| panic!("{case_id} must hash the draft"));
        assert_eq!(draft.before, draft.after, "{case_id} draft");
    }
}
