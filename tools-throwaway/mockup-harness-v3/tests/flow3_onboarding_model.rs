use mockup_harness_v3::{audit::audit_screen, chrome::tokens::ColorToken, scenario::replay_case};

#[test]
fn onboarding_leads_with_one_decision_and_a_safe_default() {
    let audit = audit_screen("onboarding", "flow3-onboarding")
        .expect("onboarding must render typed evidence");
    audit
        .validate()
        .expect("onboarding audit must be bijective");

    let text = audit.render_txt();
    assert!(text.contains("Set up your first conversation"));
    assert!(text.contains("Choose the model Code will use when you begin."));
    assert!(text.contains("The recommended model is a safe everyday default."));
    assert!(text.contains("Recommended"));
    assert!(text.contains("Claude Sonnet 4.5"));
    assert!(text.contains("Balanced speed and depth"));
    assert!(text.contains("› [enter] Continue"));
    assert!(text.contains("[esc] Back to the previous setup step"));
    assert!(!text.contains("first setup returns here"));
    assert!(!text.contains("capability matrix"));
    assert!(!text.contains("provider/model"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.onboarding.apply-default"
            && transition.to_screen_id == "empty"
    }));
}

#[test]
fn onboarding_uses_a_multiline_accent_banner_with_compact_dock_spacing() {
    let audit = audit_screen("onboarding", "flow3-onboarding")
        .expect("onboarding must render typed evidence");
    let text = audit.render_txt();
    let lines = text.lines().collect::<Vec<_>>();

    let banner_rows = audit
        .styled_cells
        .iter()
        .filter(|cell| {
            (2..=7).contains(&cell.row)
                && cell.glyph != ' '
                && cell.style.foreground == ColorToken::Accent
        })
        .map(|cell| cell.row)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        banner_rows,
        (2..=7).collect(),
        "the approved Code entry brand must span six accent rows"
    );
    assert!(
        lines[2..=7]
            .iter()
            .all(|line| line.contains('█') && !line.trim().eq("Code")),
        "the onboarding brand must be restrained ASCII art, not a single Code row"
    );

    let dock_top = lines
        .iter()
        .position(|line| line.contains('╭'))
        .expect("onboarding must retain a bottom action dock");
    let last_instruction = lines[..dock_top]
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .expect("onboarding must contain instructions before the dock");
    assert!(
        dock_top - last_instruction <= 2,
        "instruction content and the dock must not be separated by a large unexplained gap"
    );
    let dock_bottom = lines
        .iter()
        .position(|line| line.contains('╰'))
        .expect("onboarding dock must have a closing border");
    assert_eq!(
        dock_bottom, 37,
        "the action dock must remain bottom-anchored"
    );
    assert!(
        lines[dock_top + 1..dock_bottom]
            .iter()
            .all(|line| line.chars().nth(2) == Some('│') && line.chars().nth(117) == Some('│')),
        "every action-dock content row must retain both side borders"
    );
}

#[test]
fn model_picker_compares_human_models_and_focuses_one_pending_change() {
    let audit = audit_screen("model-picker", "flow3-model-picker")
        .expect("model picker must render typed evidence");
    audit
        .validate()
        .expect("model picker audit must be bijective");

    let text = audit.render_txt();
    assert!(text.contains("Choose a model"));
    assert!(text.contains("Claude Opus 4.8"));
    assert!(text.contains("GPT-5.2 Codex"));
    assert!(text.contains("Claude Sonnet 4.5"));
    assert!(text.contains("Current"));
    assert!(text.contains("Balanced speed and depth"));
    assert!(text.contains("Switching applies this choice"));
    assert!(text.contains("[enter] Apply model"));
    assert!(text.contains("[esc] Cancel"));
    assert!(!text.contains("anthropic/"));
    assert!(!text.contains("openai/"));
    assert!(!text.contains("ctx "));

    let selected_details = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "selected-detail")
        .count();
    assert_eq!(selected_details, 1, "only the focused row owns detail");
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "model-row"
                && event.identity.detail_level == "focused")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.model-picker.apply"
            && transition.to_screen_id == "chat-turn"
    }));
}

#[test]
fn onboarding_scenarios_preserve_origin_and_route_blocked_choices_safely() {
    let complete =
        replay_case("onboarding-complete", 1).expect("onboarding completion must replay");
    assert_eq!(complete.before.screen_id, "onboarding");
    assert_eq!(complete.after.screen_id, "empty");
    assert_eq!(
        complete.actions[0].action_id,
        "action.onboarding.apply-default"
    );
    assert_eq!(complete.before.origin_stack, complete.after.origin_stack);
    assert_eq!(
        complete
            .preserved_field_hashes
            .get("pending_configuration")
            .unwrap()
            .before,
        complete
            .preserved_field_hashes
            .get("pending_configuration")
            .unwrap()
            .after
    );

    let skipped = replay_case("onboarding-skip", 1).expect("safe-default skip must replay");
    assert_eq!(skipped.after.screen_id, "empty");
    assert_eq!(skipped.actions[0].action_id, "action.onboarding.skip");
    assert!(skipped.after.transcript.contains("safe default"));

    let cancelled = replay_case("model-picker-cancel", 1).expect("model cancellation must replay");
    assert_eq!(cancelled.before.screen_id, "model-picker");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.before.draft, cancelled.after.draft);
    assert_ne!(cancelled.before.selection, cancelled.after.selection);
    assert_eq!(
        cancelled
            .preserved_field_hashes
            .get("selection")
            .unwrap()
            .after,
        mockup_harness_v3::audit::sha256_hex("claude-sonnet-4-5".as_bytes())
    );

    let blocked =
        replay_case("model-picker-connect", 1).expect("blocked model connection must replay");
    assert_eq!(blocked.after.screen_id, "connect-provider");
    assert_eq!(blocked.actions[0].action_id, "action.model-picker.connect");
    assert_eq!(blocked.before.draft, blocked.after.draft);
    assert_eq!(
        blocked.before.queued_follow_up,
        blocked.after.queued_follow_up
    );

    let first_run_back =
        replay_case("onboarding-back", 1).expect("first-run Back must replay safely");
    assert_eq!(first_run_back.after.screen_id, "onboarding");

    let existing_cancel = replay_case("onboarding-existing-cancel", 1)
        .expect("existing-origin onboarding cancel must replay");
    assert_eq!(existing_cancel.after.screen_id, "chat-turn");
    assert_eq!(existing_cancel.before.draft, existing_cancel.after.draft);

    let unusable =
        replay_case("onboarding-unusable-model", 1).expect("unusable model bridge must replay");
    assert_eq!(unusable.after.screen_id, "connect-provider");
    assert_eq!(unusable.before.draft, unusable.after.draft);
}
