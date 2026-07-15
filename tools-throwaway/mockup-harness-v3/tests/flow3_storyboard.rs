use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, FlowManifest, TransitionKind,
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
        expected
            .iter()
            .map(|(screen, _, _)| *screen)
            .collect::<Vec<_>>()
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

        let audit = audit_screen(screen_id, step.fixture_id)
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
                    transition.kind == "cancel" || transition.to_screen_id == step.success_return
                }),
                "{screen_id} illustrative detour must expose its return path"
            );
        }
    }
}

#[test]
fn published_onboarding_and_model_frames_bind_the_declared_origin_and_continuation() {
    let flow = work_configuration();
    let onboarding = &flow.steps[0];
    let model_picker = &flow.steps[1];

    assert_eq!(onboarding.fixture_id, "gate.work-configuration.onboarding");
    assert_eq!(
        model_picker.fixture_id,
        "gate.work-configuration.model-picker"
    );
    assert_eq!(onboarding.visible_origin, "Work configuration · onboarding");
    assert_eq!(
        model_picker.visible_origin,
        "Work configuration · onboarding"
    );
    for field in [
        "visible_origin",
        "model_choice",
        "pending_action",
        "return_policy",
    ] {
        assert!(
            onboarding.preservation_fields.contains(&field),
            "onboarding must preserve {field}"
        );
        assert!(
            model_picker.preservation_fields.contains(&field),
            "model-picker must preserve {field}"
        );
    }

    let onboarding_audit = audit_screen(onboarding.screen_id, onboarding.fixture_id)
        .expect("declared onboarding fixture must render");
    assert!(onboarding_audit.transitions.iter().any(|transition| {
        transition.action_id == "action.onboarding.review-models"
            && transition.to_screen_id == "model-picker"
            && transition.kind == "direct"
    }));
    let onboarding_choice = onboarding_audit
        .model_choice
        .as_ref()
        .expect("onboarding must expose the recommended model continuation");
    assert_eq!(onboarding_choice.applied, "Claude Sonnet 4.5");
    assert_eq!(onboarding_choice.pending, None);
    assert_eq!(onboarding_choice.origin, "Onboarding");
    assert_eq!(
        onboarding_choice.pending_action.as_deref(),
        Some("Review model choices")
    );
    assert_eq!(onboarding_choice.return_policy, "Return to onboarding");

    let model_audit = audit_screen(model_picker.screen_id, model_picker.fixture_id)
        .expect("declared model-picker fixture must render");
    let model_text = model_audit.render_txt();
    // Phase-A paints subject titles / first-run subject, not journey crumbs.
    // Manifest/audit identity keeps the chapter-true long form.
    assert!(
        model_text.contains("First-run setup")
            || model_text.contains("Work configuration")
            || model_text.contains("Work configuration · onboarding"),
        "model-picker must paint an onboarding-chapter subject title"
    );
    assert!(
        !model_text.contains("Work configuration · current conversation"),
        "onboarding-chapter model-picker must not paint mature current-conversation crumb"
    );
    assert!(
        !model_text.contains("Work configuration · onboarding"),
        "model-picker must not reintroduce banned journey crumb paint"
    );
    assert!(model_text.contains("Your first conversation is waiting for this choice."));
    assert!(!model_text.contains("Continue the paused conversation."));
    let model_choice = model_audit
        .model_choice
        .as_ref()
        .expect("model-picker must expose its pending continuation");
    assert_eq!(model_choice.applied, "Claude Sonnet 4.5");
    assert_eq!(model_choice.pending.as_deref(), Some("Claude Opus 4.8"));
    assert_eq!(model_choice.origin, "Onboarding");
    assert_eq!(model_choice.pending_action.as_deref(), Some("Apply model"));
    assert_eq!(model_choice.return_policy, "Return to onboarding");
}

#[test]
fn work_configuration_chapters_use_chapter_true_origin_and_task() {
    let flow = work_configuration();
    let expected = [
        (
            "onboarding",
            "Work configuration · onboarding",
            "Choose a model and how Code should work.",
            true,
        ),
        (
            "model-picker",
            "Work configuration · onboarding",
            "Choose a model and how Code should work.",
            true,
        ),
        (
            "agent-picker",
            "Work configuration · current conversation",
            "Choose a model and how Code should work.",
            true,
        ),
        (
            "mode-switch",
            "Work configuration · current conversation",
            "Choose a model and how Code should work.",
            true,
        ),
        (
            "session-picker",
            "Work configuration · current conversation",
            "Resume a conversation or inspect its history.",
            false,
        ),
        (
            "timeline",
            "Work configuration · current conversation",
            "Resume a conversation or inspect its history.",
            false,
        ),
    ];

    assert_eq!(flow.steps.len(), expected.len());
    for (step, (screen_id, origin, task, keep_setup_task)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen_id);
        assert_eq!(step.visible_origin, origin);
        assert_eq!(step.user_task, task);
        assert!(
            step.preservation_fields.contains(&"visible_origin"),
            "{screen_id} must preserve visible_origin"
        );

        let audit = audit_screen(step.screen_id, step.fixture_id)
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        let text = audit.render_txt();
        if screen_id == "onboarding" {
            // Entry brand screen: journey chip is omitted in favor of the ASCII entry brand.
            assert!(text.contains("Set up your first conversation"));
            assert!(!text.contains("Work configuration · current conversation"));
        } else {
            // Phase-A subject title paint; audit identity remains the long `origin` form.
            let paints_identity = match screen_id {
                "model-picker" => {
                    text.contains("First-run setup")
                        || text.contains("Work configuration")
                        || text.contains(origin)
                }
                _ => {
                    text.contains("Work configuration")
                        || text.contains(origin)
                }
            };
            assert!(
                paints_identity,
                "{screen_id} must render chapter-true origin (subject title or `{origin}`)"
            );
            // Subject-title path must not paint the opposite chapter crumb when Phase-A is sealed.
            if origin == "Work configuration · onboarding" {
                assert!(
                    !text.contains("Work configuration · current conversation"),
                    "{screen_id} first-setup chapter must not paint current-conversation crumb"
                );
            }
            assert!(
                !text.contains("Work configuration · onboarding")
                    && !text.contains("Work configuration · current conversation"),
                "{screen_id} must not reintroduce banned journey-crumb paint"
            );
        }
        if matches!(
            screen_id,
            "agent-picker" | "mode-switch" | "session-picker" | "timeline"
        ) {
            assert!(
                !text.contains("Work configuration · onboarding"),
                "{screen_id} must not brand a mature chapter as onboarding"
            );
        }
        if !keep_setup_task {
            assert!(
                text.contains(task),
                "{screen_id} must use resume-facing task language"
            );
            assert!(
                !text.contains("Choose a model and how Code should work."),
                "{screen_id} must not claim the first-setup task while resuming sessions"
            );
        } else if matches!(screen_id, "agent-picker" | "mode-switch") {
            assert!(
                text.contains(task)
                    || text.contains("Choose how Code should work for this conversation.")
                    || text.contains("Review a work-style change"),
                "{screen_id} setup chapter must keep first-setup configuration language"
            );
        }

        if let Some(conversation) = audit.conversation.as_ref() {
            assert_eq!(conversation.visible_origin, origin);
            assert_eq!(conversation.user_task, task);
        }
    }

    // storyboard_identity contract for the journey root is current conversation;
    // only the earliest chapter may still show onboarding.
    assert_eq!(
        flow.steps[0].visible_origin,
        "Work configuration · onboarding"
    );
    assert!(flow.steps[2..]
        .iter()
        .all(|step| step.visible_origin == "Work configuration · current conversation"));
}

#[test]
fn work_configuration_gate_reconciles_one_applied_work_style() {
    let flow = work_configuration();
    let mut applied: Option<String> = None;

    for step in &flow.steps {
        let audit = audit_screen(step.screen_id, step.fixture_id)
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
                assert_eq!(work_style.pending.as_deref(), Some("Autonomous changes"));
            }
            _ => assert_eq!(work_style.pending, None),
        }
        assert_eq!(work_style.access_scope, "asks before editing project files");
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
    assert_eq!(timeline_back.before.draft, timeline_back.after.draft);

    let resume = replay_case("session-resume-latest", 1).expect("latest resume must be replayable");
    assert_eq!(resume.before.screen_id, "session-picker");
    assert_eq!(resume.after.screen_id, "chat-turn");
    assert_eq!(resume.after.work_style, "Careful edits");
    assert_eq!(resume.after.return_policy, "return-to-session-picker");
}
