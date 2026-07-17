//! Flow 5 recovery gate storyboard integrity.
//!
//! Owned contract assertion:
//! VAL-V3-F5-052 — Recovery flow storyboard is coherent.

use std::{path::Path, process::Command};

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
/// Audit continuity tag (visible_origin field) — may not paint as a primary title.
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
/// Phase-A painted subject title after journey-breadcrumb kill-switch.
const RECOVERY_PAINTED_ORIGIN: &str = "Recovery";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const CHECKPOINT_NAME: &str = "Before risk-note expansion";

fn recovery() -> &'static mockup_harness_v3::FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "recovery")
        .expect("recovery storyboard must be public")
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn recovery_storyboard_declares_ordered_canonical_aliases_and_truthful_edges() {
    let flow = recovery();
    let expected = [
        (
            "turn-error",
            "error-network",
            TransitionKind::Direct,
            "direct",
            "gate.recovery.turn-error",
        ),
        (
            "error-network",
            "rate-limit",
            TransitionKind::IllustrativeDetour,
            "illustrative",
            "gate.recovery.error-network",
        ),
        (
            "rate-limit",
            "chat-turn",
            TransitionKind::Automatic,
            "automatic",
            "gate.recovery.rate-limit",
        ),
        (
            "chat-turn",
            "checkpoint-undo",
            TransitionKind::Direct,
            "direct",
            "gate.recovery.chat-turn",
        ),
        (
            "checkpoint-undo",
            "chat-turn",
            TransitionKind::Direct,
            "direct",
            "gate.recovery.checkpoint-undo",
        ),
    ];
    assert_eq!(flow.steps.len(), expected.len());
    assert_eq!(
        flow.steps
            .iter()
            .map(|step| step.screen_id)
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(screen, _, _, _, _)| *screen)
            .collect::<Vec<_>>()
    );

    for (step, (screen, next, kind, audit_kind, fixture)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen);
        assert_eq!(step.expected_next_frame, next);
        assert_eq!(step.transition_kind, kind);
        assert_eq!(step.fixture_id, fixture);
        assert_eq!(step.conversation_id, "conversation.recovery");
        assert_eq!(step.transcript_id, "transcript.recovery");
        assert_eq!(step.session_id, "session.recovery.1");
        assert_eq!(step.turn_id, "turn.recovery.1");
        assert_eq!(step.user_task, RECOVERY_TASK);
        assert_eq!(step.visible_origin, RECOVERY_ORIGIN);
        assert!(!step.user_action.trim().is_empty());
        assert!(!step.transition_path.trim().is_empty());
        assert!(!step.origin.trim().is_empty());
        assert!(!step.success_return.trim().is_empty());
        assert!(!step.cancel_return.trim().is_empty());
        assert!(!step.acceptance_checks.is_empty());
        for field in [
            "conversation_id",
            "transcript_id",
            "session_id",
            "turn_id",
            "user_task",
            "visible_origin",
            "draft",
            "partial_response",
            "completed_work",
            "scroll_anchor",
        ] {
            assert!(
                step.preservation_fields.contains(&field),
                "{screen} must declare preservation of {field}"
            );
        }

        let audit = audit_screen(screen, fixture)
            .unwrap_or_else(|error| panic!("{screen} must render: {error}"));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen} must validate: {error}"));
        assert!(
            !audit.transitions.is_empty(),
            "{screen} must not be a dead end"
        );
        assert!(
            audit
                .transitions
                .iter()
                .all(|transition| transition.from_screen_id == screen),
            "{screen} transitions must keep their screen origin"
        );

        match kind {
            TransitionKind::Direct | TransitionKind::Automatic => {
                assert!(
                    audit.transitions.iter().any(|transition| {
                        transition.to_screen_id == next && transition.kind == audit_kind
                    }),
                    "{screen} must expose the declared {audit_kind} edge to {next}"
                );
            }
            TransitionKind::IllustrativeDetour | TransitionKind::Summarized => {
                assert!(
                    audit.transitions.iter().any(|transition| {
                        transition.kind == "cancel"
                            || transition.to_screen_id == step.success_return
                            || transition.to_screen_id == next
                    }),
                    "{screen} estimated bridge must expose return or declared next frame"
                );
                assert!(
                    !step
                        .transition_path
                        .to_lowercase()
                        .contains("directly navigates"),
                    "non-direct edges must not claim a silent direct jump"
                );
            }
        }
    }
}

#[test]
fn recovery_storyboard_tells_one_coherent_failure_to_return_to_undo_story() {
    let flow = recovery();
    let turn = audit_screen("turn-error", "gate.recovery.turn-error").expect("turn-error");
    let network =
        audit_screen("error-network", "gate.recovery.error-network").expect("error-network");
    let rate = audit_screen("rate-limit", "gate.recovery.rate-limit").expect("rate-limit");
    let chat = audit_screen("chat-turn", "gate.recovery.chat-turn").expect("chat-turn");
    let checkpoint =
        audit_screen("checkpoint-undo", "gate.recovery.checkpoint-undo").expect("checkpoint-undo");

    let turn_text = turn.render_txt();
    // Painted subject is "Recovery"; audit origin tag may still be journey-flagged.
    assert!(
        turn_text.contains(RECOVERY_PAINTED_ORIGIN) || turn_text.contains(RECOVERY_ORIGIN),
        "turn-error must paint Recovery subject (or residual journey origin)"
    );
    assert!(turn_text.contains(RECOVERY_TASK));
    assert!(turn_text.contains(USER_PROMPT));
    assert!(turn_text.contains(PARTIAL_REPLY));
    assert!(turn_text.contains("The reply stopped before it finished"));
    assert!(turn_text.contains("Inspect the connection problem"));
    assert!(turn.transitions.iter().any(|transition| {
        transition.action_id == "action.turn-error.inspect-connection"
            && transition.to_screen_id == "error-network"
            && transition.kind == "direct"
    }));

    let network_text = network.render_txt();
    assert!(
        network_text.contains(RECOVERY_PAINTED_ORIGIN) || network_text.contains(RECOVERY_ORIGIN),
        "error-network must paint Recovery subject (or residual journey origin)"
    );
    assert!(network_text.contains(USER_PROMPT));
    assert!(network_text.contains(PARTIAL_REPLY));
    assert!(
        network_text.contains("Trying again automatically")
            || network_text.contains("could not reach the provider")
    );
    assert!(
        network
            .events
            .iter()
            .any(|event| event.identity.family == "timer-owner")
            || network_text.contains("Retry request")
    );

    let rate_text = rate.render_txt();
    assert!(rate_text.contains("automatically") || rate_text.contains("on its own"));
    assert!(rate_text.contains("20s") || rate_text.contains("12s"));
    assert_eq!(
        rate.events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        0,
        "automatic rate-limit must own zero primary actions"
    );
    assert!(rate.transitions.iter().any(|transition| {
        transition.action_id == "action.rate-limit.timer-tick"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "automatic"
    }));

    let chat_text = chat.render_txt();
    assert!(chat_text.contains(USER_PROMPT) || chat_text.contains("release notes"));
    assert!(chat_text.contains(PARTIAL_REPLY) || chat_text.contains("retry window"));
    assert!(
        chat_text.contains("succeeded")
            || chat_text.contains("Draft, queue, and completed work")
            || chat_text.contains("Ready for your next")
    );
    assert!(chat.transitions.iter().any(|transition| {
        transition.action_id == "action.chat-turn.open-checkpoint-undo"
            && transition.to_screen_id == "checkpoint-undo"
            && transition.kind == "direct"
    }));
    assert!(
        chat_text.contains("Undo last checkpoint")
            || chat_text.contains("Undo the last checkpoint")
            || chat_text.contains("checkpoint")
    );

    let checkpoint_text = checkpoint.render_txt();
    assert!(checkpoint_text.contains(CHECKPOINT_NAME));
    assert!(checkpoint_text.contains("notes/release.md"));
    assert!(checkpoint_text.contains("notes/risks.md"));
    assert!(checkpoint_text.contains("Redo change") || checkpoint_text.contains("Continue"));
    assert!(checkpoint.transitions.iter().any(|transition| {
        transition.to_screen_id == "chat-turn"
            && (transition.kind == "direct" || transition.kind == "cancel")
    }));

    // Step metadata remains acceptanced and no recovery frame is a dead end.
    for step in &flow.steps {
        let audit = audit_screen(step.screen_id, step.fixture_id).expect("frame renders");
        assert!(
            audit.transitions.iter().any(|transition| {
                !transition.to_screen_id.is_empty()
                    && (transition.to_screen_id == step.success_return
                        || transition.to_screen_id == step.cancel_return
                        || transition.to_screen_id == step.expected_next_frame
                        || transition.to_screen_id == step.origin
                        || transition.kind == "cancel")
            }),
            "{} must expose at least one declared return or next-frame edge",
            step.screen_id
        );
    }
}

#[test]
fn recovery_bridges_preserve_origin_task_and_work_hashes() {
    let inspect =
        replay_case("turn-error-inspect-connection", 1).expect("inspect must open network");
    assert_eq!(inspect.before.screen_id, "turn-error");
    assert_eq!(inspect.after.screen_id, "error-network");
    assert_eq!(
        inspect.actions[0].action_id,
        "action.turn-error.inspect-connection"
    );
    assert_eq!(inspect.before.user_task, RECOVERY_TASK);
    assert_eq!(inspect.after.user_task, RECOVERY_TASK);
    assert_eq!(inspect.before.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(inspect.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(inspect.before.session_id, inspect.after.session_id);
    assert_eq!(inspect.before.draft, inspect.after.draft);
    assert!(!inspect.before.partial_response.is_empty());
    assert_eq!(
        inspect.before.partial_response,
        inspect.after.partial_response
    );
    for field in ["draft", "session_id", "transcript"] {
        let hashes = inspect
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be hashed across inspect"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive inspect");
    }

    let automatic =
        replay_case("rate-limit-automatic-success", 1).expect("automatic wait must resume chat");
    assert_eq!(automatic.before.screen_id, "rate-limit");
    assert_eq!(automatic.after.screen_id, "chat-turn");
    assert!(automatic.actions[0].automatic);
    assert_eq!(
        automatic.actions[0].action_id,
        "action.rate-limit.timer-tick"
    );
    assert_eq!(automatic.before.user_task, RECOVERY_TASK);
    assert_eq!(automatic.after.user_task, RECOVERY_TASK);
    assert_eq!(automatic.before.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(automatic.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(
        automatic.preserved_field_hashes["draft"].before,
        automatic.preserved_field_hashes["draft"].after
    );
    assert_eq!(
        automatic.preserved_field_hashes["session_id"].before,
        automatic.preserved_field_hashes["session_id"].after
    );

    let open = replay_case("chat-turn-open-checkpoint-undo", 1)
        .expect("recovered chat must open reversible undo");
    assert_eq!(open.before.screen_id, "chat-turn");
    assert_eq!(open.after.screen_id, "checkpoint-undo");
    assert_eq!(
        open.actions[0].action_id,
        "action.chat-turn.open-checkpoint-undo"
    );
    assert_eq!(open.before.user_task, RECOVERY_TASK);
    assert_eq!(open.after.user_task, RECOVERY_TASK);
    assert_eq!(open.before.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(open.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(
        open.preserved_field_hashes["draft"].before,
        open.preserved_field_hashes["draft"].after
    );
    assert_eq!(
        open.preserved_field_hashes["session_id"].before,
        open.preserved_field_hashes["session_id"].after
    );

    let cont = replay_case("checkpoint-continue-chat", 1)
        .expect("continue must restore the conversation shell");
    assert_eq!(cont.before.screen_id, "checkpoint-undo");
    assert_eq!(cont.after.screen_id, "chat-turn");
    assert_eq!(
        cont.actions[0].action_id,
        "action.checkpoint-undo.continue"
    );
    assert_eq!(cont.after.focus, "composer");
    assert_eq!(cont.after.user_task, RECOVERY_TASK);
    assert_eq!(cont.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(
        cont.preserved_field_hashes["transcript"].before,
        cont.preserved_field_hashes["transcript"].after
    );
}

#[test]
fn recovery_gate_publication_emits_five_ordered_aliases() {
    let output = harness()
        .args(["--flow", "recovery"])
        .output()
        .expect("public recovery flow selector must run");
    if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid --flow recovery must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
        return;
    }
    if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid --flow recovery must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
        return;
    }
    assert!(
        output.status.success(),
        "recovery must publish only after all five frames preflight:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    }
    }
    }
    let manifest = String::from_utf8(output.stdout).expect("flow metadata must be UTF-8");
    assert!(manifest.contains("\"flow\":\"recovery\""));
    assert!(manifest.contains("\"screen_id\":\"turn-error\""));
    assert!(manifest.contains("\"screen_id\":\"error-network\""));
    assert!(manifest.contains("\"screen_id\":\"rate-limit\""));
    assert!(manifest.contains("\"screen_id\":\"chat-turn\""));
    assert!(manifest.contains("\"screen_id\":\"checkpoint-undo\""));
    assert!(manifest.contains("\"transition_kind\":\"automatic\""));
    assert!(manifest.contains("\"transition_kind\":\"illustrative_detour\""));

    for (index, screen) in [
        "turn-error",
        "error-network",
        "rate-limit",
        "chat-turn",
        "checkpoint-undo",
    ]
    .iter()
    .enumerate()
    {
        let number = index + 1;
        for extension in ["txt", "png"] {
            let gate_path = format!(
                "/projects/cortex-code/plans/assets/mockups-v3-gates/recovery/{number:02}-{screen}.{extension}"
            );
            assert!(
                Path::new(&gate_path).is_file(),
                "gate frame {number:02}-{screen}.{extension} must publish"
            );
        }

        // Canonical gallery still has the shared surface under its category.
        let category = if *screen == "turn-error" || *screen == "chat-turn" {
            "session"
        } else {
            "states"
        };
        let canonical_path = format!(
            "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.txt"
        );
        assert!(
            Path::new(&canonical_path).is_file(),
            "canonical {category}/{screen}.txt must exist; gate stays an alias"
        );
    }
}
