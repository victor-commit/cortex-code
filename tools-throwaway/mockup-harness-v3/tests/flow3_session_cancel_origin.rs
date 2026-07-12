use mockup_harness_v3::{audit::audit_screen, flow_manifests, scenario::replay_case};

fn work_configuration_step(screen_id: &str) -> &'static mockup_harness_v3::FlowStep {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "work-configuration")
        .and_then(|flow| flow.steps.iter().find(|step| step.screen_id == screen_id))
        .expect("work-configuration step must be present")
}

#[test]
fn published_session_frames_render_the_manifest_onboarding_origin() {
    for screen_id in ["session-picker", "timeline"] {
        let step = work_configuration_step(screen_id);
        let audit =
            audit_screen(screen_id, step.fixture_id).expect("published session frame must render");
        audit
            .validate()
            .expect("published session frame must validate");
        let text = audit.render_txt();
        assert!(
            text.contains(step.visible_origin),
            "{screen_id} must show its declared origin"
        );
        assert!(
            !text.contains("Continue the paused conversation."),
            "{screen_id} must not replace onboarding with a chat origin"
        );
        assert!(
            audit
                .to_json()
                .contains("\"visible_origin\":\"Work configuration · onboarding\""),
            "{screen_id} audit must expose the declared origin"
        );
        if screen_id == "session-picker" {
            assert!(audit.transitions.iter().any(|transition| {
                transition.action_id == "action.session-picker.back"
                    && transition.kind == "cancel"
                    && transition.to_screen_id == "onboarding"
            }));
        }
    }
}

#[test]
fn mode_switch_cancel_metadata_matches_the_rendered_action_and_origin() {
    let step = work_configuration_step("mode-switch");
    assert_eq!(step.cancel_return, "agent-picker");

    let audit = audit_screen("mode-switch", step.fixture_id)
        .expect("published mode-switch frame must render");
    audit
        .validate()
        .expect("published mode-switch frame must validate");
    assert!(audit.render_txt().contains(step.visible_origin));
    let cancel = audit
        .events
        .iter()
        .find(|event| event.identity.family == "secondary-action")
        .and_then(|event| event.action.as_ref())
        .expect("mode-switch must expose a cancel action");
    let cancel_transition = audit
        .transitions
        .iter()
        .find(|transition| transition.action_id == cancel.action_id)
        .expect("cancel action must expose a transition");
    assert_eq!(cancel.label, "Keep Careful edits");
    assert_eq!(cancel_transition.kind, "cancel");
    assert_eq!(cancel_transition.to_screen_id, step.cancel_return);
    assert!(cancel_transition.path.contains("onboarding"));

    let replay = replay_case("work-style-access-cancel", 1)
        .expect("mode-switch cancel must be publicly replayable");
    assert_eq!(replay.actions[0].label, cancel.label);
    assert_eq!(replay.transitions[0].to_screen_id, step.cancel_return);
    assert_eq!(replay.transitions[0].kind, cancel_transition.kind);
    assert_eq!(replay.before.visible_origin, step.visible_origin);
    assert_eq!(replay.after.visible_origin, step.visible_origin);
    assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
    assert!(replay.audit.render_txt().contains(step.visible_origin));
}

#[test]
fn onboarding_origin_replays_survive_preview_timeline_back_and_resume() {
    for case_id in [
        "session-preview-open",
        "session-preview-back",
        "session-timeline-open",
        "session-timeline-back",
        "session-resume-latest",
    ] {
        let replay = replay_case(case_id, 1).expect("session origin replay must be public");
        assert_eq!(
            replay.before.visible_origin,
            "Work configuration · onboarding"
        );
        assert_eq!(
            replay.after.visible_origin,
            "Work configuration · onboarding"
        );
        assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
        assert!(
            replay
                .before
                .origin_stack
                .starts_with(&["onboarding".to_owned(), "session-picker".to_owned()])
        );
        if case_id != "session-resume-latest" {
            assert_eq!(replay.before.draft, replay.after.draft);
            assert_eq!(replay.before.scroll_anchor, replay.after.scroll_anchor);
        }
        assert!(
            replay
                .audit
                .to_json()
                .contains("\"visible_origin\":\"Work configuration · onboarding\"")
        );
    }
}

#[test]
fn session_picker_and_timeline_fork_actions_keep_the_onboarding_origin() {
    for (screen_id, action_id, target) in [
        (
            "session-picker",
            "action.session-picker.fork",
            "fork-picker",
        ),
        ("timeline", "action.timeline.fork", "fork-picker"),
    ] {
        let step = work_configuration_step(screen_id);
        let audit =
            audit_screen(screen_id, step.fixture_id).expect("published fork source must render");
        let fork = audit
            .events
            .iter()
            .find(|event| {
                event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.action_id == action_id)
            })
            .expect("fork action must be visible in audit evidence");
        let action = fork.action.as_ref().expect("fork action must be present");
        let transition = audit
            .transitions
            .iter()
            .find(|transition| transition.action_id == action.action_id)
            .expect("fork action must have a transition");
        assert_eq!(transition.to_screen_id, target);
        assert_eq!(transition.kind, "direct");
        assert!(audit.render_txt().contains(step.visible_origin));
    }
}
