//! Flow 5 turn-error and error-network pilot coverage.
//!
//! Owned contract assertions:
//! VAL-V3-F5-021 through VAL-V3-F5-027 and VAL-V3-F5-057.

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const PARTIAL_KEPT: &str = "The partial reply was kept";
const TURN_CAUSE: &str = "The reply stopped before it finished";

fn recovery_flow() -> &'static mockup_harness_v3::FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "recovery")
        .expect("recovery storyboard must exist")
}

#[test]
fn turn_error_keeps_partial_reply_inline_with_visible_seam() {
    let audit = audit_screen("turn-error", "gate.recovery.turn-error")
        .expect("turn-error must render");
    audit
        .validate()
        .expect("turn-error audit must stay bijective");

    let text = audit.render_txt();
    assert!(text.contains(USER_PROMPT));
    assert!(text.contains(PARTIAL_REPLY));
    assert!(text.contains(PARTIAL_KEPT));
    assert!(text.contains(TURN_CAUSE));
    assert!(
        text.contains("──") || text.contains("│"),
        "partial reply must sit above a visible seam before the failure card"
    );
    assert!(!text.to_lowercase().contains("toast"));
    assert!(!text.to_lowercase().contains("modal"));
    assert!(!text.contains("NETWORK_ERR"));
    let partial_row = text
        .lines()
        .position(|line| line.contains(PARTIAL_REPLY))
        .expect("partial");
    let cause_row = text
        .lines()
        .position(|line| line.contains(TURN_CAUSE))
        .expect("cause");
    assert!(
        partial_row < cause_row,
        "partial reply must appear above the cause"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "recovery-cause"),
        "typed recovery cause event required"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "partial-reply"),
        "typed partial-reply event required"
    );
}

#[test]
fn turn_error_primary_labels_retry_versus_continue_with_consequence() {
    let audit = audit_screen("turn-error", "gate.recovery.turn-error")
        .expect("turn-error primary hierarchy must render");
    let text = audit.render_txt();
    assert!(text.contains("› [enter] Retry reply"));
    assert!(text.contains("[c] Continue from partial reply"));
    assert!(text.contains("Retry starts a new reply and replaces the partial text."));
    assert!(text.contains("Continue keeps the partial reply and restores the composer."));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.turn-error.retry"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.turn-error.continue"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));
    assert!(
        !text.contains("[enter] Retry") || text.contains("Retry reply"),
        "ambiguous bare Retry must not lead"
    );
}

#[test]
fn turn_error_retry_and_cancel_preserve_conversation_identity() {
    let retry = replay_case("turn-error-retry", 1).expect("retry must restore chat-turn");
    assert_eq!(retry.before.screen_id, "turn-error");
    assert_eq!(retry.after.screen_id, "chat-turn");
    assert_eq!(retry.actions[0].action_id, "action.turn-error.retry");
    assert_eq!(retry.before.user_task, RECOVERY_TASK);
    assert_eq!(retry.after.user_task, RECOVERY_TASK);
    assert_eq!(retry.before.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(retry.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(retry.before.session_id, retry.after.session_id);
    assert_eq!(retry.before.draft, retry.after.draft);
    assert!(!retry.after.partial_response.is_empty() || !retry.before.partial_response.is_empty());
    let retry_text = retry.audit.render_txt();
    assert!(retry_text.contains("Reply to Code") || retry_text.contains("›"));
    assert!(!retry_text.contains("home"));
    for field in ["draft", "session_id", "transcript"] {
        let hashes = retry
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be hashed"));
        if field == "draft" || field == "session_id" {
            assert_eq!(hashes.before, hashes.after, "{field} must survive recovery");
        }
    }

    let cancel = replay_case("turn-error-cancel", 1).expect("cancel must preserve failure");
    assert_eq!(cancel.before.screen_id, "turn-error");
    assert_eq!(cancel.after.screen_id, "turn-error");
    assert_eq!(cancel.actions[0].action_id, "action.turn-error.cancel");
    assert_eq!(cancel.before.partial_response, cancel.after.partial_response);
    assert!(!cancel.after.partial_response.is_empty());
    assert!(cancel.audit.render_txt().contains(PARTIAL_KEPT));
}

#[test]
fn error_network_summary_is_human_readable_and_preserves_work() {
    let audit = audit_screen("error-network", "canonical.error-network")
        .expect("error-network must render");
    audit
        .validate()
        .expect("error-network audit must stay bijective");
    let text = audit.render_txt();
    assert!(
        text.contains("Cortex could not reach the provider")
            || text.contains("The connection ended before the reply finished")
            || text.contains("Network access is off")
    );
    assert!(
        text.contains("Your work and partial output are saved")
            || text.contains("work and partial output are saved")
    );
    assert!(!text.contains("NETWORK_ERR"));
    assert!(!text.contains("api.anthropic.com"));
    assert!(!text.contains("TLS"));
    assert!(!text.contains("DNS"));
    assert!(!text.contains("status 503"));
    assert!(!text.contains("ECONNRESET"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn error_network_attempts_live_only_in_evidence() {
    let collapsed = audit_screen("error-network", "canonical.error-network")
        .expect("collapsed network must render");
    let collapsed_text = collapsed.render_txt();
    assert!(!collapsed_text.contains("Attempt 3 of 3"));
    assert!(!collapsed_text.contains("req_"));
    assert!(!collapsed_text.contains("backoff 2s"));
    assert!(
        collapsed_text.contains("Evidence")
            || collapsed_text.contains("[d] Show evidence")
            || collapsed_text.contains("Show evidence")
    );

    let evidence = audit_screen("error-network", "error-network.evidence")
        .expect("expanded evidence must render");
    evidence
        .validate()
        .expect("evidence audit must stay bijective");
    let evidence_text = evidence.render_txt();
    assert!(evidence_text.contains("Anthropic"));
    assert!(evidence_text.contains("3 of 3"));
    assert!(evidence_text.contains("2s → 4s → 8s") || evidence_text.contains("2s -> 4s -> 8s"));
    assert!(evidence_text.contains("req_"));
    assert!(!evidence_text.contains("sk-"));
    assert!(!evidence_text.contains("Bearer "));
    assert!(
        evidence
            .events
            .iter()
            .any(|event| event.identity.detail_level == "evidence")
    );
}

#[test]
fn network_offline_is_manual_only_with_zero_provider_attempts() {
    let audit = audit_screen("error-network", "error-network.offline")
        .expect("offline network state must render");
    audit
        .validate()
        .expect("offline audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains("Network access is off"));
    assert!(text.contains("› [enter] Review network setting"));
    assert!(!text.contains("Retry request"));
    assert!(!text.contains("Retry reply"));
    assert!(!text.contains("Waiting"));
    assert!(!text.contains("countdown"));
    assert!(!text.contains("s remaining"));
    assert!(
        !audit
            .events
            .iter()
            .any(|event| event.action.as_ref().is_some_and(|action| action.automatic)),
        "offline branch must not own an automatic timer"
    );

    let offline = replay_case("network-offline-review-setting", 1)
        .expect("offline review setting detour must replay");
    assert_eq!(offline.before.screen_id, "error-network");
    assert_eq!(offline.after.screen_id, "connect-provider");
    assert_eq!(
        offline.actions[0].action_id,
        "action.error-network.review-setting"
    );
    assert_eq!(offline.attempts[0].count, 0);
    assert_eq!(offline.attempts[0].outcome, "none");
    assert_eq!(offline.before.session_id, offline.after.session_id);

    let resume = replay_case("network-offline-resume", 1)
        .expect("offline return must apply provider resume policy");
    assert_eq!(resume.before.screen_id, "connect-provider");
    assert_eq!(resume.after.screen_id, "error-network");
    assert_eq!(
        resume.actions[0].action_id,
        "action.connect-provider.back"
    );
    assert_eq!(resume.attempts[0].count, 0);
    assert_eq!(resume.before.pending_action, resume.after.pending_action);
}

#[test]
fn network_automatic_backoff_has_one_owner_limit_and_cancel() {
    let audit = audit_screen("error-network", "error-network.backoff")
        .expect("backoff state must render");
    audit
        .validate()
        .expect("backoff audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains("Cortex could not reach the provider") || text.contains("connection"));
    assert!(text.contains("Trying again automatically"));
    assert!(text.contains("[esc] Cancel"));
    assert!(!text.contains("› [enter] Retry"));
    assert!(!text.contains("[enter] Retry request"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        0,
        "automatic backoff must not expose a primary action"
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "timer-owner"
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.automatic && action.action_id.contains("backoff"))
        }),
        "one automatic timer-owner event is required"
    );

    let cancel = replay_case("network-backoff-cancel", 1).expect("cancel must stop retries");
    assert_eq!(cancel.before.screen_id, "error-network");
    assert_eq!(cancel.after.screen_id, "error-network");
    assert_eq!(
        cancel.actions[0].action_id,
        "action.error-network.cancel-backoff"
    );
    assert_eq!(cancel.attempts[0].owner_id, "timer.network.backoff");
    assert_eq!(cancel.attempts[0].outcome, "cancelled");
    assert!(cancel.audit.render_txt().contains("Retry request"));
    assert!(!cancel.audit.render_txt().contains("Trying again automatically"));

    let exp = replay_case("network-backoff-exhaust", 1).expect("exhaustion must stabilize");
    assert_eq!(exp.before.screen_id, "error-network");
    assert_eq!(exp.after.screen_id, "error-network");
    assert!(exp.actions[0].automatic);
    assert_eq!(exp.attempts[0].owner_id, "timer.network.backoff");
    assert_eq!(exp.attempts[0].count, 3);
    assert_eq!(exp.attempts[0].outcome, "exhausted");
    let exhausted_text = exp.audit.render_txt();
    assert!(exhausted_text.contains("Retry request"));
    assert!(!exhausted_text.contains("s remaining"));
    assert!(!exhausted_text.contains("Trying again automatically"));

    let race_retry = replay_case("network-backoff-race-retry-wins", 1)
        .expect("same-tick retry-now must win once");
    assert_eq!(race_retry.transitions.len(), 1);
    assert_eq!(
        race_retry.actions[0].action_id,
        "action.error-network.retry-now"
    );
    assert_eq!(race_retry.attempts[0].count, 1);

    let race_cancel = replay_case("network-backoff-race-cancel-wins", 1)
        .expect("same-tick cancel must win once");
    assert_eq!(
        race_cancel.actions[0].action_id,
        "action.error-network.cancel-backoff"
    );
    assert_eq!(race_cancel.attempts[0].outcome, "cancelled");
}

#[test]
fn network_check_connection_returns_to_same_paused_request() {
    let audit = audit_screen("error-network", "canonical.error-network")
        .expect("network secondary action must render");
    let text = audit.render_txt();
    assert!(text.contains("[d] Check connection") || text.contains("Check connection"));
    assert!(
        text.matches("› [enter]").count() <= 1,
        "connection inspection must not share primary weight"
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.error-network.check-connection"
            && transition.to_screen_id == "connect-provider"
            && transition.kind == "direct"
    }));

    let open = replay_case("network-check-connection", 1).expect("check connection must open");
    assert_eq!(open.before.screen_id, "error-network");
    assert_eq!(open.after.screen_id, "connect-provider");
    assert_eq!(
        open.actions[0].action_id,
        "action.error-network.check-connection"
    );
    assert_eq!(open.before.session_id, open.after.session_id);
    assert_eq!(open.before.pending_action, open.after.pending_action);
    assert_eq!(open.side_effects[0].status, "no-submit");

    let back = replay_case("network-check-connection-return", 1)
        .expect("return must restore paused network recovery");
    assert_eq!(back.before.screen_id, "connect-provider");
    assert_eq!(back.after.screen_id, "error-network");
    assert_eq!(back.before.draft, back.after.draft);
    assert_eq!(back.before.pending_action, back.after.pending_action);
    assert_eq!(back.side_effects[0].status, "no-submit");
    assert_eq!(back.attempts[0].count, 0);
}

#[test]
fn recovery_storyboard_binds_gate_fixtures_for_implemented_states() {
    let flow = recovery_flow();
    assert_eq!(flow.steps[0].screen_id, "turn-error");
    assert_eq!(flow.steps[1].screen_id, "error-network");
    assert_eq!(flow.steps[0].fixture_id, "gate.recovery.turn-error");
    assert_eq!(flow.steps[1].fixture_id, "gate.recovery.error-network");
    assert_eq!(flow.steps[0].user_task, RECOVERY_TASK);
    assert_eq!(flow.steps[0].visible_origin, RECOVERY_ORIGIN);

    for step in flow.steps.iter().take(2) {
        let audit = audit_screen(step.screen_id, step.fixture_id)
            .unwrap_or_else(|error| panic!("{} must render: {error}", step.screen_id));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{} must validate: {error}", step.screen_id));
        assert!(
            audit
                .transitions
                .iter()
                .all(|transition| transition.from_screen_id == step.screen_id)
        );
        if step.transition_kind == TransitionKind::Direct {
            assert!(
                audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == step.expected_next_frame
                        || transition.to_screen_id == step.success_return
                        || transition.kind == "automatic"
                        || transition.kind == "direct"
                }),
                "{} must expose a recovery edge",
                step.screen_id
            );
        }
    }
}
