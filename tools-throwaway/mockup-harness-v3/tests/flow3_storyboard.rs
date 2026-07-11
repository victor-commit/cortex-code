use mockup_harness_v3::{
    audit::audit_screen,
    flow_manifests,
    scenario::replay_case,
    FlowManifest,
    TransitionKind,
};

fn work_configuration() -> &'static FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "work-configuration")
        .expect("the work-configuration storyboard must be public")
}

#[test]
fn work_configuration_storyboard_declares_truthful_edges_and_returns() {
    let flow = work_configuration();
    let expected = [
        ("onboarding", "model-picker", TransitionKind::Direct),
        (
            "model-picker",
            "agent-picker",
            TransitionKind::IllustrativeDetour,
        ),
        ("agent-picker", "mode-switch", TransitionKind::Direct),
        (
            "mode-switch",
            "session-picker",
            TransitionKind::IllustrativeDetour,
        ),
        ("session-picker", "timeline", TransitionKind::Direct),
        ("timeline", "chat-turn", TransitionKind::Direct),
    ];

    assert_eq!(flow.steps.len(), expected.len());
    assert_eq!(
        flow.steps
            .iter()
            .map(|step| step.screen_id)
            .collect::<Vec<_>>(),
        expected.iter().map(|(screen, _, _)| *screen).collect::<Vec<_>>()
    );

    for (step, (screen_id, next_frame, kind)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen_id);
        assert_eq!(step.expected_next_frame, next_frame);
        assert_eq!(step.transition_kind, kind);
        assert!(!step.user_action.is_empty());
        assert!(!step.transition_path.is_empty());
        assert!(!step.origin.is_empty());
        assert!(!step.success_return.is_empty());
        assert!(!step.cancel_return.is_empty());
        assert!(!step.acceptance_checks.is_empty());

        let audit = audit_screen(screen_id, &format!("gate.work-configuration.{screen_id}"))
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen_id} must validate: {error}"));
        assert!(
            !audit.transitions.is_empty(),
            "{screen_id} must expose an explicit action or return transition"
        );
        assert!(
            audit
                .transitions
                .iter()
                .all(|transition| transition.from_screen_id == screen_id),
            "{screen_id} transitions must preserve their origin"
        );
        if kind == TransitionKind::Direct {
            assert!(
                audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == next_frame && transition.kind == "direct"
                }),
                "{screen_id} must expose the declared direct edge to {next_frame}"
            );
        } else {
            assert!(
                audit.transitions.iter().any(|transition| {
                    transition.kind == "cancel"
                        || transition.to_screen_id == step.success_return
                }),
                "{screen_id} illustrative detour must expose its return path"
            );
        }
    }
}

#[test]
fn work_configuration_gate_reconciles_one_applied_work_style() {
    let flow = work_configuration();
    let mut applied: Option<String> = None;

    for step in &flow.steps {
        let audit = audit_screen(
            step.screen_id,
            &format!("gate.work-configuration.{}", step.screen_id),
        )
        .expect("every storyboard frame must produce typed audit evidence");
        let work_style = audit
            .work_style
            .as_ref()
            .expect("every storyboard frame must expose work-style reconciliation");
        assert_eq!(work_style.applied, "Careful edits");
        if let Some(previous) = applied.as_deref() {
            assert_eq!(previous, work_style.applied);
        } else {
            applied = Some(work_style.applied.clone());
        }

        match step.screen_id {
            "agent-picker" | "mode-switch" => {
                assert_eq!(
                    work_style.pending.as_deref(),
                    Some("Autonomous changes")
                );
            }
            _ => assert_eq!(work_style.pending, None),
        }
        assert_eq!(
            work_style.access_scope,
            "asks before editing project files"
        );
    }
}

#[test]
fn work_configuration_bridges_and_returns_preserve_the_same_origin() {
    let model_cancel =
        replay_case("model-picker-cancel", 1).expect("model cancellation must be replayable");
    assert_eq!(model_cancel.before.screen_id, "model-picker");
    assert_eq!(model_cancel.after.screen_id, "chat-turn");
    assert_eq!(model_cancel.before.draft, model_cancel.after.draft);

    let access_pending = replay_case("work-style-access-pending", 1)
        .expect("access-changing selection must remain pending");
    assert_eq!(access_pending.after.screen_id, "mode-switch");
    assert_eq!(
        access_pending.after.pending_work_style.as_deref(),
        Some("Autonomous changes")
    );
    assert_eq!(
        access_pending.before.access_scope,
        access_pending.after.access_scope
    );

    let access_refused =
        replay_case("work-style-access-refused", 1).expect("refusal must return safely");
    assert_eq!(access_refused.after.screen_id, "mode-switch-refused");
    assert_eq!(access_refused.after.work_style, "Careful edits");
    assert_eq!(
        access_refused.before.origin_stack,
        access_refused.after.origin_stack
    );

    let timeline_back =
        replay_case("session-timeline-back", 1).expect("timeline Back must be replayable");
    assert_eq!(timeline_back.before.screen_id, "timeline");
    assert_eq!(timeline_back.after.screen_id, "session-picker");
    assert_eq!(
        timeline_back.before.draft,
        timeline_back.after.draft
    );

    let resume =
        replay_case("session-resume-latest", 1).expect("latest resume must be replayable");
    assert_eq!(resume.before.screen_id, "session-picker");
    assert_eq!(resume.after.screen_id, "chat-turn");
    assert_eq!(resume.after.work_style, "Careful edits");
    assert_eq!(resume.after.return_policy, "return-to-session-picker");
}
