use mockup_harness_v3::{
    audit::{audit_screen, validate_conversation_identity, ConversationIdentity},
    flow_manifests,
    scenario::replay_case,
};

const DAILY_SCREENS: [&str; 6] = [
    "home",
    "chat-turn",
    "streaming",
    "tool-running",
    "tool-success",
    "rich-response",
];

#[test]
fn daily_work_declares_one_fixture_and_one_conversation_identity() {
    let flow = flow_manifests()
        .iter()
        .find(|flow| flow.name == "daily-work")
        .expect("daily-work must be declared");
    let fixture_ids = flow
        .steps
        .iter()
        .map(|step| step.fixture_id)
        .collect::<Vec<_>>();
    assert_eq!(fixture_ids.len(), DAILY_SCREENS.len());
    assert!(fixture_ids.iter().all(|fixture| *fixture == fixture_ids[0]));

    let mut identities = Vec::new();
    for screen in DAILY_SCREENS {
        let audit = audit_screen(
            screen,
            flow.steps
                .iter()
                .find(|step| step.screen_id == screen)
                .unwrap()
                .fixture_id,
        )
        .unwrap_or_else(|error| panic!("{screen} must render: {error}"));
        let identity = audit
            .conversation
            .as_ref()
            .unwrap_or_else(|| panic!("{screen} must expose conversation identity"));
        let expected = ConversationIdentity {
            conversation_id: "conversation.daily-work".to_owned(),
            transcript_id: "transcript.daily-work".to_owned(),
            session_id: "session.daily-work.1".to_owned(),
            turn_id: "turn.daily-work.18".to_owned(),
            user_task: "Find where the server applies the rate limit and explain the safe path."
                .to_owned(),
            visible_origin: "Daily work · current conversation".to_owned(),
            submitted_prompt: None,
            paused_activity_id: None,
            later_draft: None,
        };
        validate_conversation_identity(identity, &expected, &audit.render_txt())
            .unwrap_or_else(|error| panic!("{screen} identity must be visible: {error}"));
        identities.push((
            identity.conversation_id.clone(),
            identity.transcript_id.clone(),
            identity.session_id.clone(),
            identity.turn_id.clone(),
            identity.user_task.clone(),
            identity.visible_origin.clone(),
        ));
    }
    assert!(identities.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn automatic_daily_work_handoffs_preserve_identity_and_emit_hashes() {
    for case_id in [
        "streaming-interrupt",
        "tool-running-interrupt",
        "tool-running-complete",
        "tool-success-continue",
    ] {
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|error| panic!("{case_id} must replay: {error}"));
        replay
            .validate()
            .unwrap_or_else(|error| panic!("{case_id} must validate: {error}"));
        for field in [
            "conversation_id",
            "transcript_id",
            "session_id",
            "turn_id",
            "user_task",
            "visible_origin",
        ] {
            let hashes = replay
                .preserved_field_hashes
                .get(field)
                .unwrap_or_else(|| panic!("{case_id} must hash {field}"));
            assert_eq!(hashes.before, hashes.after, "{case_id} changed {field}");
        }
        assert!(replay.audit.to_json().contains("\"conversation\""));
    }
}
