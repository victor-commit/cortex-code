use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

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
    assert!(text.contains("Asks before editing files"));
    assert!(text.contains("edit project files without asking each time"));
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
    assert!(text.contains("[enter] Choose a safer work style"));
    assert!(text.contains("[esc] Return to work"));
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
}

#[test]
fn work_style_cases_preserve_origin_and_apply_only_after_confirmation() {
    let unchanged =
        replay_case("work-style-unchanged", 1).expect("unchanged work style must replay");
    assert_eq!(unchanged.before.screen_id, "agent-picker");
    assert_eq!(unchanged.after.screen_id, "chat-turn");
    assert_eq!(unchanged.before.work_style, "Careful edits");
    assert_eq!(unchanged.after.work_style, "Quick implementation");
    assert_eq!(unchanged.before.access_scope, unchanged.after.access_scope);
    assert_eq!(unchanged.before.draft, unchanged.after.draft);

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

    let confirmed =
        replay_case("work-style-access-confirm", 1).expect("confirmed access change must replay");
    assert_eq!(confirmed.before.screen_id, "mode-switch");
    assert_eq!(confirmed.after.screen_id, "chat-turn");
    assert_eq!(confirmed.after.work_style, "Autonomous changes");
    assert_ne!(confirmed.after.access_scope, confirmed.before.access_scope);
    assert_eq!(confirmed.before.draft, confirmed.after.draft);

    let refused = replay_case("work-style-access-refused", 1).expect("refusal must replay safely");
    assert_eq!(refused.after.screen_id, "mode-switch-refused");
    assert_eq!(refused.after.work_style, "Careful edits");
    assert_eq!(refused.after.access_scope, refused.before.access_scope);
    assert_eq!(refused.before.draft, refused.after.draft);
}
