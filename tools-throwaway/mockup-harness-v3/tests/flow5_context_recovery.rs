//! Flow 5 context exhaustion and compaction recovery.
//!
//! Owned contract assertions:
//! VAL-V3-F5-028, VAL-V3-F5-029, VAL-V3-F5-030, VAL-V3-F5-031.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const COMPLETED_EDITS: &str = "Completed edits · notes/release.md · notes/risks.md";
const PENDING_TASK: &str = "Pending · finish the short risk note before publish";
const DRAFT: &str = "Keep the risk note short before we publish.";
const CAUSE: &str = "This conversation is too large to continue as-is";
const FULL_HISTORY: &str = "full";
const PRIMARY: &str = "Compact and continue";
const SECONDARY: &str = "Start new session";

fn assert_no_token_diagnostics(text: &str) {
    for banned in [
        "96%",
        "51k",
        "18k",
        "token total",
        "ctx 9",
        "usage bar",
        "model limit",
        "Tokens used",
        "Breakdown · earlier turns",
    ] {
        assert!(
            !text.contains(banned),
            "token diagnostics must stay collapsed until Evidence expands: saw {banned}"
        );
    }
}

#[test]
fn error_context_preserves_refactor_state_and_states_plain_exhaustion() {
    // VAL-V3-F5-028
    let audit = audit_screen("error-context", "canonical.error-context")
        .expect("error-context must render");
    audit.validate().expect("error-context audit");

    let text = audit.render_txt();
    assert!(
        text.contains(USER_PROMPT) || text.contains(RECOVERY_TASK),
        "active user request must remain visible"
    );
    assert!(text.contains(RECOVERY_ORIGIN));
    assert!(text.contains(COMPLETED_EDITS), "completed edits stay visible");
    assert!(text.contains(PENDING_TASK), "pending task stays visible");
    assert!(text.contains(PARTIAL_REPLY), "partial answer stays visible");
    assert!(text.contains(DRAFT), "composer draft stays visible");
    assert!(text.contains(CAUSE), "plain English exhaustion cause required");
    assert!(
        text.contains("saved") || text.contains("kept") || text.contains("preserved"),
        "preservation of work must be stated"
    );
    assert!(
        !text.to_lowercase().contains("starting a new session automatically")
            && !text.to_lowercase().contains("auto-compact"),
        "visible exhaustion must not claim automatic session or compact"
    );

    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "recovery-cause"),
        "typed recovery cause required"
    );
    assert!(
        !audit
            .events
            .iter()
            .any(|event| event.action.as_ref().is_some_and(|action| action.automatic)),
        "visible error-context must never auto-compact or auto-start a session"
    );
    assert!(
        !audit
            .transitions
            .iter()
            .any(|transition| transition.kind == "automatic"),
        "no automatic transitions while exhaustion is visible"
    );
}

#[test]
fn error_context_compact_is_single_primary_with_secondary_new_session() {
    // VAL-V3-F5-029
    let audit = audit_screen("error-context", "canonical.error-context")
        .expect("error-context primary must render");
    let text = audit.render_txt();

    assert!(
        text.contains(&format!("› [enter] {PRIMARY}")) || text.contains(PRIMARY),
        "Compact and continue must be the explicit primary"
    );
    assert!(text.contains(SECONDARY), "Start new session is secondary only");
    assert!(
        text.contains("summary")
            && (text.contains("older") || text.contains("visible context") || text.contains("replaces")),
        "consequence must explain that a summary replaces older visible context"
    );
    assert!(
        text.contains(FULL_HISTORY)
            && (text.contains("preserved") || text.contains("stays") || text.contains("kept")),
        "consequence must say full transcript remains preserved"
    );

    let primaries: Vec<_> = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "primary-action")
        .collect();
    assert_eq!(primaries.len(), 1, "exactly one primary action:");
    assert!(
        primaries[0]
            .action
            .as_ref()
            .is_some_and(|action| action.label.contains("Compact") && !action.automatic),
        "primary action is Compact and continue and never automatic"
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "secondary-action"
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.label.contains("new session") && !action.automatic)
        }),
        "Start new session remains secondary and not automatic"
    );
}

#[test]
fn error_context_token_diagnostics_are_progressive() {
    // VAL-V3-F5-030
    let collapsed = audit_screen("error-context", "canonical.error-context")
        .expect("collapsed error-context");
    let collapsed_text = collapsed.render_txt();
    assert_no_token_diagnostics(&collapsed_text);
    assert!(
        collapsed_text.contains("Evidence")
            || collapsed_text.contains("Expand technical")
            || collapsed_text.contains("[d]"),
        "collapsed state exposes an Evidence disclosure"
    );
    assert!(
        collapsed_text.contains(CAUSE),
        "cause appears before any diagnostics"
    );
    let cause_pos = collapsed_text.find(CAUSE).expect("cause");
    let primary_pos = collapsed_text
        .find(PRIMARY)
        .expect("primary compact action");
    assert!(
        cause_pos < primary_pos,
        "cause and primary precede secondary disclosure content"
    );

    let evidence = audit_screen("error-context", "error-context.evidence")
        .expect("expanded evidence must render");
    evidence.validate().expect("evidence audit must stay bijective");
    let evidence_text = evidence.render_txt();
    assert!(
        evidence_text.contains("Tokens used") || evidence_text.contains("token"),
        "expanded evidence exposes token usage"
    );
    assert!(
        evidence_text.contains('%') || evidence_text.contains("limit"),
        "expanded evidence includes a concise usage explanation"
    );
    assert!(
        evidence
            .events
            .iter()
            .any(|event| event.identity.detail_level == "evidence"
                || event.identity.family == "evidence"),
        "expanded diagnostics are typed as evidence"
    );
}

#[test]
fn preventive_compaction_may_run_before_visible_exhaustion() {
    // VAL-V3-F5-028 / VAL-V3-F5-029 prevention branch
    let preventive = replay_case("context-preventive-compact", 1)
        .expect("preventive compact before visible exhaustion");
    preventive.validate().expect("preventive validation");
    assert_eq!(preventive.before.screen_id, "chat-turn");
    assert_eq!(preventive.after.screen_id, "compact");
    assert!(
        preventive.actions[0].automatic,
        "preventive compaction may be automatic"
    );
    assert_eq!(
        preventive.actions[0].action_id,
        "action.context.preventive-compact"
    );
    for field in ["draft", "session_id"] {
        let hashes = preventive
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved across preventive compact"));
        assert_eq!(hashes.before, hashes.after, "{field}");
    }
    assert!(
        preventive
            .preserved_field_hashes
            .contains_key("transcript"),
        "transcript hash evidence remains available after preventive compact"
    );
    assert!(
        preventive
            .preserved_field_hashes
            .contains_key("completed_edits"),
        "completed edit identities must be preserved"
    );
    assert!(
        preventive
            .preserved_field_hashes
            .contains_key("pending_task"),
        "pending task identity must be preserved"
    );
}

#[test]
fn visible_exhaustion_never_auto_compacts_or_opens_new_session() {
    // VAL-V3-F5-028 fail-closed after visibility
    let stuck = replay_case("error-context-no-auto", 1).expect("visible no-auto");
    stuck.validate().expect("visible no-auto validation");
    assert_eq!(stuck.before.screen_id, "error-context");
    assert_eq!(stuck.after.screen_id, "error-context");
    assert!(!stuck.actions[0].automatic);
    assert_eq!(stuck.attempts[0].count, 0);
    assert_eq!(stuck.side_effects[0].status, "no-automatic-compact");
    for field in [
        "draft",
        "session_id",
        "transcript",
        "partial_response",
        "completed_edits",
        "pending_task",
    ] {
        let hashes = stuck
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must stay stable while visible"));
        assert_eq!(hashes.before, hashes.after, "{field}");
    }
    assert_ne!(
        stuck.before.state_hash, "",
        "state hash evidence must exist"
    );
    assert_ne!(stuck.after.state_hash, "", "after state hash evidence must exist");
    assert_eq!(
        stuck.before.draft, stuck.after.draft,
        "visible exhaustion leaves the draft unchanged"
    );
    assert_eq!(
        stuck.before.paused_activity_id, stuck.after.paused_activity_id,
        "paused activity identity stays exact while visible"
    );
}

#[test]
fn compact_and_continue_returns_through_compact_to_paused_activity() {
    // VAL-V3-F5-031
    let compact = replay_case("error-context-compact-continue", 1)
        .expect("compact and continue from exhaustion");
    compact.validate().expect("compact continue validation");
    assert_eq!(compact.before.screen_id, "error-context");
    assert_eq!(compact.after.screen_id, "compact");
    assert_eq!(
        compact.actions[0].action_id,
        "action.error-context.compact-continue"
    );
    assert!(!compact.actions[0].automatic);
    assert!(
        compact.audit.render_txt().contains("compact")
            || compact.audit.render_txt().contains("Compact")
            || compact.audit.render_txt().contains("summary"),
        "compact surface must present a readable summary"
    );
    for field in ["draft", "session_id", "completed_edits", "pending_task"] {
        let hashes = compact
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} preserved across compact"));
        assert_eq!(hashes.before, hashes.after, "{field}");
    }
    assert!(
        compact
            .preserved_field_hashes
            .contains_key("history_order"),
        "full transcript remains recoverable"
    );

    let resume = replay_case("compact-context-resume-chat", 1).expect("resume chat-turn");
    resume.validate().expect("resume validation");
    assert_eq!(resume.before.screen_id, "compact");
    assert_eq!(resume.after.screen_id, "chat-turn");
    assert_eq!(
          resume.actions[0].action_id,
          "action.compact.continue-context"
    );
    assert!(
        resume.after.draft.contains("risk note")
            || resume
                .preserved_field_hashes
                .get("draft")
                .is_some_and(|hashes| hashes.before == hashes.after),
        "composer draft must restore the paused activity"
    );
    assert_eq!(
        resume.after.user_task, RECOVERY_TASK,
        "exact paused request/task identity restores"
    );
    assert!(
        resume.after.paused_activity_id.contains("context")
            || resume.after.paused_activity_id.contains("release")
            || resume.after.paused_activity_id.contains("risk"),
        "paused activity returns to the exact interrupted work"
    );
    let resume_text = resume.audit.render_txt();
    assert!(
        resume_text.contains("risk")
            || resume_text.contains("release")
            || resume_text.contains("Reply")
            || resume_text.contains("composer")
            || resume_text.contains("›"),
        "returned chat-turn must be a usable continuation"
    );
}

#[test]
fn context_recovery_evidence_round_trips() {
    let open = replay_case("error-context-evidence-open", 1).expect("open evidence");
    open.validate().expect("evidence open");
    assert_eq!(open.before.screen_id, "error-context");
    assert_eq!(open.after.screen_id, "error-context");
    assert_eq!(
        open.actions[0].action_id,
        "action.error-context.expand-evidence"
    );
    assert!(
        open.audit.render_txt().contains("Tokens used")
            || open.audit.render_txt().contains("token")
            || open.audit.render_txt().contains('%')
    );

    let close = replay_case("error-context-evidence-close", 1).expect("close evidence");
    close.validate().expect("evidence close");
    assert_eq!(close.before.screen_id, "error-context");
    assert_eq!(close.after.screen_id, "error-context");
    assert_eq!(
        close.actions[0].action_id,
        "action.error-context.collapse-evidence"
    );
    assert_no_token_diagnostics(&close.audit.render_txt());
}
