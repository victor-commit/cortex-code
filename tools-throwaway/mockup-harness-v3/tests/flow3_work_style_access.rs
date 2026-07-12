use mockup_harness_v3::{
    audit::audit_screen,
    scenario::{replay_case, CaseReplay},
};

fn assert_reconciled_hash(replay: &CaseReplay, field: &str, unchanged: bool) {
    let hash = replay
        .preserved_field_hashes
        .get(field)
        .unwrap_or_else(|| panic!("{field} must have before/after SHA-256 evidence"));
    assert_eq!(hash.before.len(), 64);
    assert_eq!(hash.after.len(), 64);
    if unchanged {
        assert_eq!(
            hash.before, hash.after,
            "{field} must remain byte-identical across the replay"
        );
    } else {
        assert_ne!(
            hash.before, hash.after,
            "{field} must record the applied state change"
        );
    }
}

#[test]
fn agent_picker_is_the_single_how_code_should_work_choice() {
    let audit = audit_screen("agent-picker", "flow3-work-style-access")
        .expect("agent picker must render typed evidence");
    audit
        .validate()
        .expect("work-style audit must be bijective");

    let text = audit.render_txt();
    assert!(text.contains("How Code should work"));
    assert!(text.contains("Careful edits"));
    assert!(text.contains("Quick implementation"));
    assert!(text.contains("Autonomous changes"));
    assert!(text.contains("Current · Careful edits"));
    assert!(text.contains("Pending selection · Autonomous changes"));
    assert!(text.contains("approve each project-file edit"));
    assert!(text.contains("Project-file edits can happen without asking each time"));
    assert!(!text.contains("Choose an agent"));
    assert!(!text.contains("Choose a mode"));
    assert!(!text.contains("primary agent"));
    assert!(!text.contains("subagent"));

    let work_style_events = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "work-style")
        .count();
    assert_eq!(work_style_events, 1);
    assert!(audit.to_json().contains("\"work_style\""));
    assert!(audit.to_json().contains("\"applied\":\"Careful edits\""));
}

#[test]
fn unchanged_access_applies_directly_without_nested_confirmation() {
    let audit = audit_screen("agent-picker", "flow3-work-style-unchanged")
        .expect("unchanged-access work style must render");
    let text = audit.render_txt();
    assert!(text.contains("Current · Careful edits"));
    assert!(text.contains("Pending selection · Quick implementation"));
    assert!(text.contains("Keeps current access"));
    assert!(text.contains("[enter] Use Quick implementation"));
    assert!(!text.contains("Review access change"));
    assert!(audit
        .transitions
        .iter()
        .any(|transition| transition.to_screen_id != "mode-switch"));
}

#[test]
fn access_change_discloses_scope_and_waits_for_confirmation() {
    let audit = audit_screen("mode-switch", "flow3-work-style-access")
        .expect("access change confirmation must render");
    audit
        .validate()
        .expect("confirmation audit must be bijective");
    let text = audit.render_txt();
    assert!(text.contains("Requested: Autonomous changes"));
    assert!(text.contains("edit project files without asking each time"));
    assert!(text.contains("Scope: this project"));
    assert!(text.contains("until you choose another work style"));
    assert!(text.contains("Your transcript and draft stay unchanged"));
    assert!(text.contains("[enter] Apply access change"));
    assert!(text.contains("[esc] Keep Careful edits"));
    assert!(!text.contains("permission matrix"));
    assert!(!text.contains("tool envelope"));
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
fn refusal_keeps_current_style_and_offers_one_safe_next_action() {
    let audit = audit_screen("mode-switch-refused", "flow3-work-style-refused")
        .expect("refusal bridge must render");
    audit.validate().expect("refusal audit must be bijective");
    let text = audit.render_txt();
    assert!(text.contains("Not applied"));
    assert!(text.contains("still using Careful edits"));
    assert!(text.contains("transcript and draft are unchanged"));
    assert!(text.contains("Quick implementation keeps the same access"));
    assert!(text.contains("[enter] Choose Quick implementation · same access"));
    assert!(text.contains("[esc] Return to work"));
    let primary_line = text
        .lines()
        .find(|line| line.contains("[enter] Choose Quick implementation · same access"))
        .expect("primary refusal recovery action must render");
    let return_line = text
        .lines()
        .find(|line| line.contains("[esc] Return to work"))
        .expect("secondary refusal return action must render");
    assert_ne!(
        primary_line, return_line,
        "primary and secondary refusal actions need distinct visual rows"
    );
    assert!(!text.contains("generic retry"));
    assert!(!text.contains("mode error"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "secondary-action")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mode-switch-refused.choose-unchanged"
            && transition.to_screen_id == "agent-picker"
    }));
}

#[test]
fn returned_conversation_visibly_reconciles_applied_style_and_access() {
    let unchanged =
        replay_case("work-style-unchanged", 1).expect("unchanged work style must replay");
    assert_eq!(unchanged.before.screen_id, "agent-picker");
    assert_eq!(unchanged.after.screen_id, "chat-turn");
    assert_eq!(unchanged.before.work_style, "Careful edits");
    assert_eq!(unchanged.after.work_style, "Quick implementation");
    assert_eq!(unchanged.before.access_scope, unchanged.after.access_scope);
    assert_eq!(unchanged.before.draft, unchanged.after.draft);
    let unchanged_text = unchanged.audit.render_txt();
    assert!(unchanged_text.contains("Applied work style · Quick implementation"));
    assert!(unchanged_text.contains("Access · Asks before editing project files"));
    assert_eq!(
        unchanged
            .audit
            .events
            .iter()
            .filter(|event| event.identity.family == "work-style")
            .count(),
        1
    );
    assert_reconciled_hash(&unchanged, "work_style", false);
    assert_reconciled_hash(&unchanged, "pending_work_style", true);
    assert_reconciled_hash(&unchanged, "access_scope", true);

    let pending = replay_case("work-style-access-pending", 1)
        .expect("access-changing selection must remain pending");
    assert_eq!(pending.before.screen_id, "agent-picker");
    assert_eq!(pending.after.screen_id, "mode-switch");
    assert_eq!(pending.before.work_style, "Careful edits");
    assert_eq!(
        pending.after.pending_work_style.as_deref(),
        Some("Autonomous changes")
    );
    assert_eq!(pending.after.access_scope, pending.before.access_scope);
    let pending_text = pending.audit.render_txt();
    assert!(pending_text.contains("Careful edits remains active"));
    assert!(pending_text.contains("Requested: Autonomous changes"));
    assert_reconciled_hash(&pending, "work_style", true);
    assert_reconciled_hash(&pending, "pending_work_style", false);
    assert_reconciled_hash(&pending, "access_scope", true);

    let confirmed =
        replay_case("work-style-access-confirm", 1).expect("confirmed access change must replay");
    assert_eq!(confirmed.before.screen_id, "mode-switch");
    assert_eq!(confirmed.after.screen_id, "chat-turn");
    assert_eq!(confirmed.after.work_style, "Autonomous changes");
    assert_ne!(confirmed.after.access_scope, confirmed.before.access_scope);
    assert_eq!(confirmed.before.draft, confirmed.after.draft);
    let confirmed_text = confirmed.audit.render_txt();
    assert!(confirmed_text.contains("Applied work style · Autonomous changes"));
    assert!(confirmed_text
        .contains("Access · Can edit project files without asking each time · this project"));
    assert_eq!(
        confirmed
            .audit
            .work_style
            .as_ref()
            .expect("returned chat must expose applied work-style evidence")
            .pending,
        None
    );
    assert_reconciled_hash(&confirmed, "work_style", false);
    assert_reconciled_hash(&confirmed, "pending_work_style", false);
    assert_reconciled_hash(&confirmed, "access_scope", false);
}

#[test]
fn cancel_and_refusal_preserve_old_configuration_with_safe_recovery() {
    let cancelled =
        replay_case("work-style-access-cancel", 1).expect("access cancellation must replay");
    assert_eq!(cancelled.before.screen_id, "mode-switch");
    assert_eq!(cancelled.after.screen_id, "agent-picker");
    assert_eq!(cancelled.after.work_style, "Careful edits");
    assert_eq!(cancelled.after.pending_work_style, None);
    assert_eq!(cancelled.before.access_scope, cancelled.after.access_scope);
    assert!(cancelled
        .audit
        .render_txt()
        .contains("Current · Careful edits"));
    assert_reconciled_hash(&cancelled, "work_style", true);
    assert_reconciled_hash(&cancelled, "pending_work_style", false);
    assert_reconciled_hash(&cancelled, "access_scope", true);

    let refused = replay_case("work-style-access-refused", 1).expect("refusal must replay safely");
    assert_eq!(refused.after.screen_id, "mode-switch-refused");
    assert_eq!(refused.after.work_style, "Careful edits");
    assert_eq!(refused.after.access_scope, refused.before.access_scope);
    assert_eq!(refused.before.draft, refused.after.draft);
    assert_eq!(
        refused.actions[0].label,
        "Choose Quick implementation · same access"
    );
    assert_eq!(
        refused.actions[0].action_id,
        "action.mode-switch-refused.choose-unchanged"
    );
    assert_eq!(
        refused.transitions[0].action_id,
        "action.mode-switch-refused.choose-unchanged"
    );
    assert!(refused
        .audit
        .events
        .iter()
        .any(|event| event.action.as_ref().is_some_and(|action| {
            action.action_id == "action.mode-switch-refused.choose-unchanged"
        })));
    assert!(refused.audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mode-switch-refused.choose-unchanged"
    }));
    assert!(!refused.to_json().contains("choose-safer"));
    assert_reconciled_hash(&refused, "work_style", true);
    assert_reconciled_hash(&refused, "pending_work_style", false);
    assert_reconciled_hash(&refused, "access_scope", true);

    let unchanged_recovery = replay_case("work-style-refusal-choose-unchanged", 1)
        .expect("refusal must offer an unchanged-access recovery");
    assert_eq!(unchanged_recovery.before.screen_id, "mode-switch-refused");
    assert_eq!(unchanged_recovery.after.screen_id, "agent-picker");
    assert_eq!(unchanged_recovery.after.work_style, "Careful edits");
    assert_eq!(
        unchanged_recovery.after.pending_work_style.as_deref(),
        Some("Quick implementation")
    );
    assert_eq!(
        unchanged_recovery.before.access_scope,
        unchanged_recovery.after.access_scope
    );
    assert_eq!(
        unchanged_recovery.actions[0].action_id,
        "action.mode-switch-refused.choose-unchanged"
    );
    let recovery_text = unchanged_recovery.audit.render_txt();
    assert!(recovery_text.contains("Current · Careful edits"));
    assert!(recovery_text.contains("Pending selection · Quick implementation"));
    assert!(recovery_text.contains("Keeps current access"));
    assert!(!recovery_text.contains("Review access change"));
    assert_reconciled_hash(&unchanged_recovery, "work_style", true);
    assert_reconciled_hash(&unchanged_recovery, "pending_work_style", false);
    assert_reconciled_hash(&unchanged_recovery, "access_scope", true);

    let return_to_work = replay_case("work-style-refusal-return", 1)
        .expect("refusal return must restore the paused conversation");
    assert_eq!(return_to_work.before.screen_id, "mode-switch-refused");
    assert_eq!(return_to_work.after.screen_id, "chat-turn");
    assert_eq!(return_to_work.after.work_style, "Careful edits");
    assert_eq!(return_to_work.after.pending_work_style, None);
    assert_eq!(
        return_to_work.before.access_scope,
        return_to_work.after.access_scope
    );
    let return_text = return_to_work.audit.render_txt();
    assert!(return_text.contains("Applied work style · Careful edits"));
    assert!(return_text.contains("Access · Asks before editing project files"));
    assert_reconciled_hash(&return_to_work, "work_style", true);
    assert_reconciled_hash(&return_to_work, "pending_work_style", true);
    assert_reconciled_hash(&return_to_work, "access_scope", true);
}

#[test]
fn stale_refusal_action_identity_is_rejected_by_replay_validation() {
    let mut replay =
        replay_case("work-style-access-refused", 1).expect("refusal must replay safely");
    replay.actions[0].action_id = "action.mode-switch-refused.choose-safer".to_owned();
    replay.transitions[0].action_id = "action.mode-switch-refused.choose-safer".to_owned();

    assert_eq!(
        replay
            .validate()
            .expect_err("stale action ID must fail closed"),
        "work-style refusal replay must use action.mode-switch-refused.choose-unchanged"
    );
}
