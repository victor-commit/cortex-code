//! Flow 6 mission/workflow gate storyboard integrity.
//!
//! Owned contract assertions:
//! VAL-V3-F6-044 — Preserve the canonical gate sequence
//! VAL-V3-F6-046 — Enforce compact default density

use std::{path::Path, process::Command};

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};

const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSING_INPUT: &str = "Release checklist note is missing";
const FAILURE_CAUSE: &str = "Publish path validation failed the safe release checks";

fn mission() -> &'static mockup_harness_v3::FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "mission")
        .expect("mission storyboard must be public")
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn forbidden_internals(text: &str, screen: &str) {
    for token in [
        "worker",
        "wave",
        "worktree",
        "score",
        "ladder",
        "tool envelope",
        "acceptance-contract",
        "features.json",
        "n=3",
        "concurrency",
        "orchestrat",
        "contract_id",
        "l0",
        "l1",
        "l2",
    ] {
        assert!(
            !text.to_lowercase().contains(token),
            "{screen} default frame must hide internals (`{token}`):\n{text}"
        );
    }
}

fn assert_compact_mission_card(text: &str, screen: &str) {
    assert!(
        text.contains(MISSION_GOAL),
        "{screen} must show the mission goal"
    );
    assert!(
        text.contains("complete") && text.contains("remaining"),
        "{screen} must show completed/remaining counts"
    );
    // Compact MissionCard never owns review-outcome fields.
    if screen != "reviewer-output" && screen != "reviewer-approved" {
        assert!(
            !text.contains("VERDICT"),
            "{screen} MissionCard must not own verdict tables"
        );
    }
    assert!(
        !text.contains("check matrix"),
        "{screen} must not show raw check matrices by default"
    );
    assert!(
        !text.contains("branch/"),
        "{screen} must not show branch paths in the default card"
    );
}

#[test]
fn mission_storyboard_declares_ordered_canonical_aliases_and_truthful_edges() {
    let flow = mission();
    let expected = [
        (
            "workflow-launch-blocked",
            "workflow-launch",
            TransitionKind::Automatic,
            "automatic",
            "gate.mission.workflow-launch-blocked",
        ),
        (
            "workflow-launch",
            "workflow-monitor",
            TransitionKind::Direct,
            "direct",
            "gate.mission.workflow-launch",
        ),
        (
            "workflow-monitor",
            "workflow-monitor-failure",
            TransitionKind::IllustrativeDetour,
            "illustrative",
            "gate.mission.workflow-monitor",
        ),
        (
            "workflow-monitor-failure",
            "reviewer-output",
            TransitionKind::Summarized,
            "summarized",
            "gate.mission.workflow-monitor-failure",
        ),
        (
            "reviewer-output",
            "reviewer-approved",
            TransitionKind::Summarized,
            "summarized",
            "gate.mission.reviewer-output",
        ),
        (
            "reviewer-approved",
            "chat-turn",
            TransitionKind::Direct,
            "direct",
            "gate.mission.reviewer-approved",
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
        assert_eq!(step.conversation_id, "conversation.mission");
        assert_eq!(step.transcript_id, "transcript.mission");
        assert_eq!(step.session_id, "session.mission.1");
        assert_eq!(step.turn_id, "turn.mission.1");
        assert_eq!(step.user_task, MISSION_TASK);
        assert_eq!(step.visible_origin, MISSION_ORIGIN);
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
                            || transition.to_screen_id == step.cancel_return
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
fn mission_metadata_names_noncanonical_paths_across_the_journey() {
    let flow = mission();
    let joined = flow
        .steps
        .iter()
        .flat_map(|step| {
            step.acceptance_checks
                .iter()
                .copied()
                .chain(std::iter::once(step.transition_path))
                .chain(std::iter::once(step.user_action))
        })
        .collect::<Vec<_>>()
        .join(" | ")
        .to_ascii_lowercase();

    for path in [
        "proposal",
        "preflight",
        "fix",
        "revalidat",
        "review",
        "approv",
        "return",
    ] {
        assert!(
            joined.contains(path),
            "mission storyboard metadata must name the `{path}` path:\n{joined}"
        );
    }

    // Concrete per-step acceptance anchors that the gate product reviews.
    let by_screen = flow
        .steps
        .iter()
        .map(|step| (step.screen_id, step.acceptance_checks))
        .collect::<Vec<_>>();
    for (screen, checks) in by_screen {
        let blob = checks.join(" ");
        match screen {
            "workflow-launch-blocked" => {
                assert!(blob.contains("preflight") || blob.contains("proposal"));
            }
            "workflow-launch" => {
                assert!(blob.contains("launch") || blob.contains("preflight"));
            }
            "workflow-monitor" => {
                assert!(blob.contains("monitor") || blob.contains("progress"));
            }
            "workflow-monitor-failure" => {
                assert!(blob.contains("fix") || blob.contains("validation"));
            }
            "reviewer-output" => {
                assert!(blob.contains("review") || blob.contains("fix"));
            }
            "reviewer-approved" => {
                assert!(blob.contains("approval") || blob.contains("return"));
            }
            other => panic!("unexpected mission screen {other}"),
        }
    }
}

#[test]
fn mission_storyboard_tells_one_goal_led_blocker_to_approval_story() {
    let blocked = audit_screen(
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    )
    .expect("blocked launch");
    let launch = audit_screen("workflow-launch", "gate.mission.workflow-launch").expect("launch");
    let monitor =
        audit_screen("workflow-monitor", "gate.mission.workflow-monitor").expect("monitor");
    let failure = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure");
    let review =
        audit_screen("reviewer-output", "gate.mission.reviewer-output").expect("review");
    let approved =
        audit_screen("reviewer-approved", "gate.mission.reviewer-approved").expect("approved");

    let blocked_text = blocked.render_txt();
    assert!(blocked_text.contains(MISSION_ORIGIN));
    assert!(blocked_text.contains(MISSION_TASK) || blocked_text.contains(MISSION_GOAL));
    assert!(blocked_text.contains(MISSING_INPUT));
    assert!(blocked_text.contains("Preflight") || blocked_text.contains("preflight"));
    assert!(
        blocked_text.contains("Add release checklist")
            || blocked_text.contains("Resolve the missing input")
    );
    assert!(!blocked_text.contains("Start mission"));
    forbidden_internals(&blocked_text, "workflow-launch-blocked");
    assert_compact_mission_card(&blocked_text, "workflow-launch-blocked");
    assert_eq!(
        blocked
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(blocked.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.resolve-missing-input"
            && transition.to_screen_id == "workflow-launch"
            && transition.kind == "automatic"
    }));

    let launch_text = launch.render_txt();
    assert!(launch_text.contains(MISSION_ORIGIN));
    assert!(launch_text.contains("Ready to launch") || launch_text.contains("Preflight passed"));
    assert!(launch_text.contains("Start mission"));
    assert!(launch_text.contains("Cancel"));
    forbidden_internals(&launch_text, "workflow-launch");
    assert_compact_mission_card(&launch_text, "workflow-launch");
    assert_eq!(
        launch
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(launch.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.start"
            && transition.to_screen_id == "workflow-monitor"
            && transition.kind == "direct"
    }));

    let monitor_text = monitor.render_txt();
    assert!(monitor_text.contains(MISSION_ORIGIN));
    assert!(monitor_text.contains("Running") || monitor_text.contains("Progress"));
    assert!(
        monitor_text.contains("View progress") || monitor_text.contains("composer ready")
    );
    forbidden_internals(&monitor_text, "workflow-monitor");
    assert_compact_mission_card(&monitor_text, "workflow-monitor");
    // Healthy monitor keeps card primary subordinate: at most one and no clever tables.
    let monitor_primary = monitor
        .events
        .iter()
        .filter(|event| event.identity.family == "primary-action")
        .count();
    assert!(monitor_primary <= 1);

    let failure_text = failure.render_txt();
    assert!(failure_text.contains(MISSION_ORIGIN));
    assert!(failure_text.contains("Needs attention"));
    assert!(
        failure_text.contains(FAILURE_CAUSE) || failure_text.contains("validation failed")
    );
    assert!(failure_text.contains("Create a fix task"));
    assert!(!failure_text.contains("Approve completion"));
    forbidden_internals(&failure_text, "workflow-monitor-failure");
    assert_compact_mission_card(&failure_text, "workflow-monitor-failure");
    assert_eq!(
        failure
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(failure.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.create-fix-task"
            && transition.to_screen_id == "workflow-monitor"
    }));

    let review_text = review.render_txt();
    assert!(review_text.contains(MISSION_ORIGIN));
    assert!(
        review_text.contains(MISSION_TASK),
        "reviewer-output must keep the stable mission task banner across monitor→review"
    );
    assert!(
        review_text.contains("You · Turn the release work into a mission and walk me through it.")
            || (review_text.contains("You ·")
                && review_text.contains("Turn the release work into a mission")),
        "reviewer-output must preserve You-line continuity or an explicit You-line bridge"
    );
    assert!(
        review_text.contains("Changes requested") || review_text.contains("Create a fix task")
    );
    assert!(
        review_text.contains("Safe publish path") || review_text.contains("validation")
    );
    // Review may add a separate concise outcome block; MissionCard stays compact.
    // "Mission review" is allowed only as a section label under continuous task/You identity.
    if review_text.contains("Mission review") {
        let origin = review_text
            .lines()
            .position(|line| line.contains(MISSION_ORIGIN))
            .expect("origin on review");
        let review_label = review_text
            .lines()
            .position(|line| line.contains("Mission review"))
            .expect("mission review label");
        let task_row = review_text
            .lines()
            .position(|line| line.contains(MISSION_TASK))
            .expect("stable task must appear");
        assert!(
            origin < task_row && task_row <= review_label + 2,
            "Mission review must not replace the mission task as the unlabeled journey title"
        );
    }
    assert_compact_mission_card(&review_text, "reviewer-output");
    forbidden_internals(&review_text, "reviewer-output");
    assert_eq!(
        review
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );

    let approved_text = approved.render_txt();
    assert!(approved_text.contains(MISSION_ORIGIN));
    assert!(
        approved_text.contains("Ready for completion") || approved_text.contains("Approved")
    );
    assert!(approved_text.contains("Approve completion"));
    assert!(
        approved_text.contains("Request changes") || approved_text.contains("[d] Request changes")
    );
    assert_compact_mission_card(&approved_text, "reviewer-approved");
    forbidden_internals(&approved_text, "reviewer-approved");
    assert_eq!(
        approved
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(approved.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.approve-completion"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));

    for step in &mission().steps {
        let audit = audit_screen(step.screen_id, step.fixture_id).expect("frame renders");
        assert!(
            audit.transitions.iter().any(|transition| {
                !transition.to_screen_id.is_empty()
                    && (transition.to_screen_id == step.success_return
                        || transition.to_screen_id == step.cancel_return
                        || transition.to_screen_id == step.expected_next_frame
                        || transition.to_screen_id == step.origin
                        || transition.kind == "cancel"
                        || transition.kind == "automatic"
                        || transition.kind == "direct")
            }),
            "{} must expose at least one declared return or next-frame edge",
            step.screen_id
        );
    }
}

#[test]
fn mission_default_frames_enforce_compact_mission_card_density() {
    for (screen, fixture) in [
        (
            "workflow-launch-blocked",
            "gate.mission.workflow-launch-blocked",
        ),
        ("workflow-launch", "gate.mission.workflow-launch"),
        ("workflow-monitor", "gate.mission.workflow-monitor"),
        (
            "workflow-monitor-failure",
            "gate.mission.workflow-monitor-failure",
        ),
        ("reviewer-output", "gate.mission.reviewer-output"),
        ("reviewer-approved", "gate.mission.reviewer-approved"),
    ] {
        let audit = audit_screen(screen, fixture).expect("mission gate frame must render");
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen}: {error}"));
        let text = audit.render_txt();
        assert_eq!(text.lines().count(), 40);
        assert!(text.lines().all(|line| line.chars().count() == 120));
        forbidden_internals(&text, screen);
        assert_compact_mission_card(&text, screen);

        // Mission card sermentics always present; review outcome is separate.
        assert!(
            audit
                .events
                .iter()
                .any(|event| event.identity.family == "mission-card"
                    || event.identity.family == "mission-goal"),
            "{screen} must audit a mission card / goal field"
        );
        if screen == "reviewer-output" || screen == "reviewer-approved" {
            assert!(
                audit
                    .events
                    .iter()
                    .any(|event| event.identity.family == "review-outcome"),
                "{screen} must keep the outcome block separate from MissionCard"
            );
        }

        // No raw orchestration tables in default density.
        for banned in [
            "worker #",
            "wave ",
            "score:",
            "contract",
            "ladder L",
            "tool-envelope",
        ] {
            assert!(
                !text.to_lowercase().contains(&banned.to_ascii_lowercase()),
                "{screen} leaked banned density token `{banned}`"
            );
        }

        // Exactly zero or one primary action; decision screens use one.
        let primary_count = audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count();
        assert!(
            primary_count <= 1,
            "{screen} must not present competing primary actions ({primary_count})"
        );
        if matches!(
            screen,
            "workflow-launch-blocked"
                | "workflow-launch"
                | "workflow-monitor-failure"
                | "reviewer-output"
                | "reviewer-approved"
        ) {
            assert_eq!(
                primary_count, 1,
                "{screen} decision/blocker/review state needs exactly one primary"
            );
        }
    }
}

#[test]
fn mission_bridges_preserve_origin_task_and_session_hashes() {
    let resolve = replay_case("mission-resolve-input-ready", 1)
        .expect("resolve missing input must open ready launch");
    assert_eq!(resolve.before.screen_id, "workflow-launch-blocked");
    assert_eq!(resolve.after.screen_id, "workflow-launch");
    assert_eq!(
        resolve.actions[0].action_id,
        "action.mission.resolve-missing-input"
    );
    assert_eq!(resolve.before.user_task, MISSION_TASK);
    assert_eq!(resolve.after.user_task, MISSION_TASK);
    assert_eq!(resolve.before.visible_origin, MISSION_ORIGIN);
    assert_eq!(resolve.after.visible_origin, MISSION_ORIGIN);
    assert_eq!(resolve.before.session_id, resolve.after.session_id);
    for field in ["session_id", "transcript"] {
        if let Some(hashes) = resolve.preserved_field_hashes.get(field) {
            assert_eq!(
                hashes.before, hashes.after,
                "{field} must survive resolve-input"
            );
        }
    }

    let start =
        replay_case("mission-start", 1).expect("start mission must enter compact monitoring");
    assert_eq!(start.before.screen_id, "workflow-launch");
    assert_eq!(start.after.screen_id, "workflow-monitor");
    assert_eq!(start.actions[0].action_id, "action.mission.start");
    assert_eq!(start.before.user_task, MISSION_TASK);
    assert_eq!(start.after.user_task, MISSION_TASK);
    assert_eq!(start.before.visible_origin, MISSION_ORIGIN);
    assert_eq!(start.after.visible_origin, MISSION_ORIGIN);

    let create_fix = replay_case("mission-create-fix-task", 1)
        .expect("failure create-fix must open the planned fix path");
    assert_eq!(create_fix.before.screen_id, "workflow-monitor-failure");
    assert_eq!(create_fix.after.screen_id, "workflow-monitor");
    assert_eq!(
        create_fix.actions[0].action_id,
        "action.mission.create-fix-task"
    );
    assert_eq!(create_fix.before.user_task, MISSION_TASK);
    assert_eq!(create_fix.after.user_task, MISSION_TASK);
    assert_eq!(create_fix.before.visible_origin, MISSION_ORIGIN);

    let approve = replay_case("mission-approve-completion", 1)
        .expect("approve must return to the conversation");
    assert_eq!(approve.before.screen_id, "reviewer-approved");
    assert_eq!(approve.after.screen_id, "chat-turn");
    assert_eq!(
        approve.actions[0].action_id,
        "action.mission.approve-completion"
    );
    assert_eq!(approve.after.user_task, MISSION_TASK);
    assert_eq!(approve.after.visible_origin, MISSION_ORIGIN);
}

#[test]
fn mission_gate_publication_emits_six_ordered_aliases() {
    let output = harness()
        .args(["--flow", "mission"])
        .output()
        .expect("public mission flow selector must run");
    assert!(
        output.status.success(),
        "mission must publish only after all six frames preflight:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = String::from_utf8(output.stdout).expect("flow metadata must be UTF-8");
    assert!(manifest.contains("\"flow\":\"mission\""));
    for screen in [
        "workflow-launch-blocked",
        "workflow-launch",
        "workflow-monitor",
        "workflow-monitor-failure",
        "reviewer-output",
        "reviewer-approved",
    ] {
        assert!(
            manifest.contains(&format!("\"screen_id\":\"{screen}\"")),
            "manifest must include {screen}"
        );
    }
    assert!(manifest.contains("\"transition_kind\":\"automatic\""));
    assert!(manifest.contains("\"transition_kind\":\"summarized\""));
    assert!(manifest.contains("\"transition_kind\":\"illustrative_detour\""));
    assert!(manifest.contains("\"transition_kind\":\"direct\""));
    // Non-canonical path names must appear in the public metadata surface.
    let lower = manifest.to_ascii_lowercase();
    for needle in [
        "preflight",
        "proposal",
        "fix",
        "review",
        "approv",
        "return",
    ] {
        assert!(
            lower.contains(needle),
            "public mission flow metadata must name `{needle}`"
        );
    }

    for (index, screen) in [
        "workflow-launch-blocked",
        "workflow-launch",
        "workflow-monitor",
        "workflow-monitor-failure",
        "reviewer-output",
        "reviewer-approved",
    ]
    .iter()
    .enumerate()
    {
        let number = index + 1;
        for extension in ["txt", "png"] {
            let gate_path = format!(
                "/projects/cortex-code/plans/assets/mockups-v3-gates/mission/{number:02}-{screen}.{extension}"
            );
            assert!(
                Path::new(&gate_path).is_file(),
                "gate frame {number:02}-{screen}.{extension} must publish"
            );
        }

        let canonical_path = format!(
            "/projects/cortex-code/plans/assets/mockups-v3/orchestration/{screen}.txt"
        );
        assert!(
            Path::new(&canonical_path).is_file(),
            "canonical orchestration/{screen}.txt must exist; gate stays an alias"
        );
    }
}
