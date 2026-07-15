//! Flow 6 launch and monitor pilot coverage.
//!
//! Owned contract assertions:
//! VAL-V3-F6-001 through VAL-V3-F6-011.

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};

const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSING_INPUT: &str = "Release checklist note is missing";
const TASK_1: &str = "Review the release notes for open risks";
const TASK_2: &str = "Update the next-step note after fixes";
const TASK_3: &str = "Check the safe publish path";

fn mission_flow() -> &'static mockup_harness_v3::FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "mission")
        .expect("mission storyboard must exist")
}

fn forbidden_internals(text: &str) {
    for token in [
        "worker",
        "wave",
        "worktree",
        "branch",
        "score",
        "ladder",
        "tool envelope",
        "acceptance-contract",
        "features.json",
        "N=3",
        "concurrency",
        "orchestrat",
    ] {
        assert!(
            !text.to_lowercase().contains(token),
            "default mission surface leaked internal detail `{token}`:\n{text}"
        );
    }
}

#[test]
fn mission_proposal_is_conversational_with_accept_plan_only() {
    let audit = audit_screen("workflow-launch", "workflow-launch.proposal")
        .expect("proposal must render");
    audit
        .validate()
        .expect("proposal audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_ORIGIN));
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Mission proposal"));
    assert!(text.contains(TASK_1));
    assert!(text.contains(TASK_2));
    assert!(text.contains(TASK_3));
    assert!(text.contains("Depends on"));
    assert!(text.contains("› [enter] Accept plan"));
    assert!(text.contains("[esc] Keep discussing"));
    assert!(!text.contains("Start mission"));
    assert!(!text.contains("[enter] Start"));
    assert!(!text.to_lowercase().contains("launch the mission"));
    forbidden_internals(&text);
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "mission-proposal")
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.accept-plan"
            && transition.kind == "automatic"
            && transition.to_screen_id == "workflow-launch"
    }));
}

#[test]
fn human_task_plan_leads_with_goal_and_dependencies() {
    let audit = audit_screen("workflow-launch", "workflow-launch.proposal")
        .expect("task plan proposal must render");
    let text = audit.render_txt();
    let goal_row = text
        .lines()
        .position(|line| line.contains(MISSION_GOAL))
        .expect("goal");
    let task_row = text
        .lines()
        .position(|line| line.contains(TASK_1))
        .expect("task");
    let dep_row = text
        .lines()
        .position(|line| line.contains("Depends on"))
        .expect("dependency");
    assert!(goal_row < task_row, "goal must precede task list");
    assert!(task_row < dep_row);
    forbidden_internals(&text);
    assert!(audit.events.iter().any(|event| event.identity.family == "task-plan"));
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "task-dependency")
    );
}

#[test]
fn accept_plan_starts_automatic_preflight_without_launch() {
    let replay =
        replay_case("mission-proposal-accept", 1).expect("accept plan must start preflight");
    assert_eq!(replay.before.screen_id, "workflow-launch");
    assert_eq!(replay.after.screen_id, "workflow-launch");
    assert_eq!(replay.actions[0].action_id, "action.mission.accept-plan");
    assert_eq!(replay.transitions[0].kind, "automatic");
    assert_eq!(replay.before.user_task, MISSION_TASK);
    assert_eq!(replay.after.user_task, MISSION_TASK);
    assert_eq!(replay.before.session_id, replay.after.session_id);
    let text = replay.audit.render_txt();
    assert!(text.contains("Preflight"));
    assert!(text.contains("automatically") || text.contains("Automatic preflight"));
    assert!(!text.contains("› [enter] Start mission"));
    assert!(!text.contains("Run preflight"));
    assert!(!text.contains("Start preflight"));
    forbidden_internals(&text);
}

#[test]
fn successful_preflight_launch_is_compact_with_collapsed_evidence() {
    let audit = audit_screen("workflow-launch", "gate.mission.workflow-launch")
        .expect("ready launch must render");
    audit
        .validate()
        .expect("ready launch audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Ready to launch"));
    assert!(text.contains("0 complete · 3 remaining"));
    assert!(text.contains("✓ Preflight passed"));
    assert!(text.contains("[d] View preflight evidence"));
    assert!(text.contains("› [enter] Start mission"));
    assert!(!text.contains("Needs attention"));
    assert!(!text.contains("architecture.md"));
    assert!(!text.contains("acceptance-contract"));
    assert!(!text.contains("features.json"));
    assert!(!text.contains("Step 0"));
    forbidden_internals(&text);
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "mission-card")
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "preflight-success")
    );
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
fn launch_card_shows_only_allowed_fields_and_pending_decision() {
    let audit = audit_screen("workflow-launch", "gate.mission.workflow-launch")
        .expect("launch essentials must render");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Ready to launch · 0 complete · 3 remaining"));
    assert!(text.contains("Start this mission?"));
    assert!(text.contains("Start runs the planned tasks"));
    assert!(text.contains("› [enter] Start mission"));
    assert!(text.contains("[esc] Cancel · return to conversation"));
    assert!(!text.contains("Needs attention:"));
    assert!(!text.contains("worker"));
    assert!(!text.contains("budget"));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.start"
            && transition.to_screen_id == "workflow-monitor"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.launch-cancel"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "cancel"
    }));
}

#[test]
fn blocked_launch_surfaces_one_missing_input_action() {
    let audit = audit_screen(
        "workflow-launch-blocked",
        "gate.mission.workflow-launch-blocked",
    )
    .expect("blocked launch must render");
    audit
        .validate()
        .expect("blocked launch audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Preflight · 0 complete · 3 remaining"));
    assert!(text.contains(MISSING_INPUT) || text.contains("Needs attention:"));
    assert!(text.contains("› [enter] Add release checklist"));
    assert!(text.contains("preflight reruns automatically") || text.contains("automatically"));
    assert!(!text.contains("› [enter] Start mission"));
    assert!(!text.contains("Start mission"));
    assert!(!text.contains("Run preflight"));
    assert!(!text.contains("Re-run preflight"));
    assert!(!text.contains("exit 1"));
    forbidden_internals(&text);
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "blocker")
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.resolve-missing-input"
            && transition.to_screen_id == "workflow-launch"
            && transition.kind == "automatic"
    }));
}

#[test]
fn resolving_missing_input_reruns_preflight_ready_or_blocked() {
    let ready =
        replay_case("mission-resolve-input-ready", 1).expect("resolve must reach launch-ready");
    assert_eq!(ready.before.screen_id, "workflow-launch-blocked");
    assert_eq!(ready.after.screen_id, "workflow-launch");
    assert_eq!(
        ready.actions[0].action_id,
        "action.mission.resolve-missing-input"
    );
    assert_eq!(ready.transitions[0].kind, "automatic");
    assert_eq!(ready.attempts[0].outcome, "launch-ready");
    let ready_text = ready.audit.render_txt();
    assert!(ready_text.contains("› [enter] Start mission"));
    assert!(!ready_text.contains("Launch remains omitted"));

    let blocked = replay_case("mission-resolve-input-still-blocked", 1)
        .expect("failed rerun must remain blocked");
    assert_eq!(blocked.before.screen_id, "workflow-launch-blocked");
    assert_eq!(blocked.after.screen_id, "workflow-launch-blocked");
    assert_eq!(blocked.attempts[0].outcome, "still-blocked");
    let blocked_text = blocked.audit.render_txt();
    assert!(!blocked_text.contains("› [enter] Start mission"));
    assert!(blocked_text.contains("Add release checklist"));
    assert!(!blocked_text.contains("Run preflight"));
}

#[test]
fn launch_decision_is_explicit_bottom_dock_and_not_modal() {
    let audit = audit_screen("workflow-launch", "gate.mission.workflow-launch")
        .expect("explicit launch dock must render");
    let text = audit.render_txt();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 40);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("╭") && line.contains("─")),
        "launch decision must use a bottom dock border"
    );
    let dock_row = lines
        .iter()
        .position(|line| line.contains("Start this mission?"))
        .expect("dock title");
    assert!(
        dock_row >= 28,
        "launch decision dock must replace the bottom composer area, not float mid-canvas"
    );
    assert!(!text.to_lowercase().contains("modal"));
    assert!(!text.to_lowercase().contains("popup"));
    assert!(text.contains("› [enter] Start mission"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
}

#[test]
fn start_mission_enters_monitor_with_stable_identity() {
    let replay = replay_case("mission-start", 1).expect("start must open monitor");
    assert_eq!(replay.before.screen_id, "workflow-launch");
    assert_eq!(replay.after.screen_id, "workflow-monitor");
    assert_eq!(replay.actions[0].action_id, "action.mission.start");
    assert_eq!(replay.before.user_task, MISSION_TASK);
    assert_eq!(replay.after.user_task, MISSION_TASK);
    assert_eq!(replay.before.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.after.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.before.conversation_id, "conversation.mission");
    assert_eq!(
        replay.before.conversation_id,
        replay.after.conversation_id
    );
    assert_eq!(replay.before.session_id, replay.after.session_id);
    assert_eq!(replay.before.draft, replay.after.draft);
    let text = replay.audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Running"));
    assert!(text.contains("1 complete · 2 remaining"));
    assert!(text.contains("View progress"));
    assert!(!text.contains("Start this mission?"));
    assert!(!text.contains("› [enter] Start mission"));
    forbidden_internals(&text);
}

#[test]
fn cancel_launch_returns_to_conversation_without_starting() {
    let cancel = replay_case("mission-launch-cancel", 1).expect("launch cancel must return");
    assert_eq!(cancel.before.screen_id, "workflow-launch");
    assert_eq!(cancel.after.screen_id, "chat-turn");
    assert_eq!(
        cancel.actions[0].action_id,
        "action.mission.launch-cancel"
    );
    assert_eq!(cancel.transitions[0].kind, "cancel");
    assert_eq!(cancel.attempts[0].outcome, "cancelled");
    assert_eq!(cancel.before.draft, cancel.after.draft);
    assert_eq!(cancel.before.user_task, cancel.after.user_task);
    assert_eq!(cancel.before.session_id, cancel.after.session_id);
    assert_eq!(cancel.side_effects[0].status, "no-start");

    let blocked_cancel =
        replay_case("mission-blocked-cancel", 1).expect("blocked cancel must return");
    assert_eq!(blocked_cancel.before.screen_id, "workflow-launch-blocked");
    assert_eq!(blocked_cancel.after.screen_id, "chat-turn");
    assert_eq!(blocked_cancel.side_effects[0].status, "no-start");
}

#[test]
fn healthy_monitor_is_compact_with_one_view_progress_action() {
    let audit = audit_screen("workflow-monitor", "gate.mission.workflow-monitor")
        .expect("healthy monitor must render");
    audit
        .validate()
        .expect("healthy monitor audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Running · 1 complete · 2 remaining"));
    assert!(
        text.contains("Enter to view progress") || text.contains("› View progress"),
        "healthy monitor must show the View progress primary without bracket chords"
    );
    assert!(!text.contains("[enter]"), "launch monitor must not use bracket chords");
    assert!(!text.contains("Needs attention"));
    assert!(!text.contains("Pause"));
    assert!(!text.contains("Replan"));
    assert!(!text.contains("Stop"));
    assert!(!text.contains("worker"));
    assert!(!text.contains("FEATURE"));
    assert!(!text.contains("VERDICT"));
    forbidden_internals(&text);
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "mission-card")
    );
    let progress = replay_case("mission-view-progress", 1).expect("view progress must stay local");
    assert_eq!(progress.before.screen_id, "workflow-monitor");
    assert_eq!(progress.after.screen_id, "workflow-monitor");
    assert_eq!(
        progress.actions[0].action_id,
        "action.mission.view-progress"
    );
}

/// VAL-V3-DS-009/010: healthy workflow-monitor must not declare conversation_shell
/// until it paints a true bottom composer + statusline anatomy. Prefer temporary
/// detail with explicit return when the compact mission card owns the canvas.
#[test]
fn healthy_monitor_surface_matches_temporary_detail_anatomy() {
    for fixture in [
        "gate.mission.workflow-monitor",
        "canonical.workflow-monitor",
        "public-audit",
    ] {
        let audit = audit_screen("workflow-monitor", fixture)
            .unwrap_or_else(|e| panic!("monitor/{fixture} must render: {e}"));
        audit
            .validate()
            .unwrap_or_else(|e| panic!("monitor/{fixture} audit: {e}"));
        let surface = audit
            .events
            .iter()
            .find(|event| event.identity.family == "surface")
            .map(|event| event.identity.subject_id.as_str())
            .unwrap_or("");
        assert_eq!(
            surface, "temporary-detail-view",
            "monitor/{fixture}: healthy monitor must not claim conversation-shell with mid-card composer: got {surface}"
        );
        let text = audit.render_txt();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 40, "monitor/{fixture}: 40-row canvas");
        // No fake mid-screen composer label that pretends shell anatomy.
        let composer_line = lines
            .iter()
            .position(|line| line.contains("› composer ready"));
        assert!(
            composer_line.is_none(),
            "monitor/{fixture}: mid-screen › composer ready contradicts temporary detail; got row {composer_line:?}"
        );
        // Compact mission identity + one primary remain (prose bylines, no brackets).
        assert!(
            text.contains("Enter to view progress") || text.contains("› View progress"),
            "monitor/{fixture}: View progress primary must stay"
        );
        assert!(
            text.contains("Intervene") || text.contains("intervene"),
            "monitor/{fixture}: Intervene secondary must stay 1:1"
        );
        // Shared MissionCard primary must not emit [enter]; secondary/return
        // screen-owned chords migrate in phase-A orchestration rewrite.
        assert!(
            !text.contains("[enter] View progress"),
            "monitor/{fixture}: View progress must not use a bracket key atom"
        );
        // Temporary-detail return path is visible (not a conversation shell).
        let lower = text.to_ascii_lowercase();
        assert!(
            lower.contains("return") || lower.contains("resume conversation") || lower.contains("esc to"),
            "monitor/{fixture}: temporary detail needs an obvious return affordance"
        );
        // Return must not dual-bind onto View progress / Intervene labels.
        let view_progress = audit.events.iter().filter(|e| {
            e.action
                .as_ref()
                .is_some_and(|a| a.action_id == "action.mission.view-progress")
        }).count();
        let intervene = audit.events.iter().filter(|e| {
            e.action
                .as_ref()
                .is_some_and(|a| a.action_id == "action.mission.open-intervene")
        }).count();
        let return_home = audit.events.iter().filter(|e| {
            e.action.as_ref().is_some_and(|a| {
                a.action_id == "action.mission.monitor-return"
                    || a.action_id.contains("return")
                    || a.label.to_ascii_lowercase().contains("return")
                    || a.label.to_ascii_lowercase().contains("resume conversation")
            })
        }).count();
        assert_eq!(view_progress, 1, "monitor/{fixture}: View progress 1:1");
        assert_eq!(intervene, 1, "monitor/{fixture}: Intervene 1:1");
        assert!(
            return_home >= 1,
            "monitor/{fixture}: dedicated return action must exist (got {return_home})"
        );
    }
}

#[test]
fn mission_storyboard_declares_launch_through_monitor_path() {
    let flow = mission_flow();
    let expected = [
        (
            "workflow-launch-blocked",
            "workflow-launch",
            TransitionKind::Automatic,
        ),
        (
            "workflow-launch",
            "workflow-monitor",
            TransitionKind::Direct,
        ),
        (
            "workflow-monitor",
            "workflow-monitor-failure",
            TransitionKind::IllustrativeDetour,
        ),
        (
            "workflow-monitor-failure",
            "reviewer-output",
            TransitionKind::Summarized,
        ),
        (
            "reviewer-output",
            "reviewer-approved",
            TransitionKind::Summarized,
        ),
        (
            "reviewer-approved",
            "chat-turn",
            TransitionKind::Direct,
        ),
    ];
    assert_eq!(flow.steps.len(), expected.len());
    for (step, (screen_id, next_frame, kind)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen_id);
        assert_eq!(step.expected_next_frame, next_frame);
        assert_eq!(step.transition_kind, kind);
        assert_eq!(step.fixture_id, format!("gate.mission.{screen_id}"));
        assert_eq!(step.conversation_id, "conversation.mission");
        assert_eq!(step.user_task, MISSION_TASK);
        assert_eq!(step.visible_origin, MISSION_ORIGIN);
    }
}
