//! Flow 6 team board and bounded progress disclosure.
//!
//! Owned contract assertions:
//! VAL-V3-F6-013, VAL-V3-F6-014, VAL-V3-F6-015, VAL-V3-F6-020,
//! VAL-V3-F6-021, VAL-V3-F6-022, VAL-V3-F6-056.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const TASK_1: &str = "Review the release notes for open risks";
const TASK_2: &str = "Update the next-step note after fixes";
const TASK_3: &str = "Check the safe publish path";
const FIX_TASK: &str = "Repair the safe publish path checks";

fn forbidden_first_progress_internals(text: &str) {
    for phrase in [
        "Wave ·",
        "Worktree ·",
        "Branch ·",
        "Contract ·",
        "Owner ·",
        "score",
        "ladder",
        "tool envelope",
        "tool-envelope",
        "acceptance-contract",
        "features.json",
        "concurrency",
        "orchestrat",
        "/tmp/",
        "worker-",
        "Worker ·",
        "assigned to",
    ] {
        assert!(
            !text.to_lowercase().contains(&phrase.to_lowercase()),
            "first progress disclosure leaked internal detail `{phrase}`:\n{text}"
        );
    }
}

fn assert_group_order(text: &str) {
    let needs = text
        .lines()
        .position(|line| line.contains("Needs attention") && !line.contains(":"));
    let working = text
        .lines()
        .position(|line| line.trim_start().starts_with("Working"));
    let completed = text
        .lines()
        .position(|line| line.contains("Recently completed"));

    if let (Some(n), Some(w)) = (needs, working) {
        assert!(n < w, "Needs attention must precede Working:\n{text}");
    }
    if let (Some(w), Some(c)) = (working, completed) {
        assert!(w < c, "Working must precede Recently completed:\n{text}");
    }
    if let (Some(n), Some(c)) = (needs, completed) {
        assert!(
            n < c,
            "Needs attention must precede Recently completed:\n{text}"
        );
    }
}

#[test]
fn expanded_progress_orders_groups_exception_first() {
    let audit = audit_screen("workflow-monitor", "workflow-monitor.progress.attention")
        .expect("exception-ordered progress must render");
    audit
        .validate()
        .expect("progress audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Needs attention"));
    assert!(text.contains("Working"));
    assert!(text.contains("Recently completed"));
    assert!(text.contains(TASK_3) || text.contains(FIX_TASK));
    assert_group_order(&text);
    // Empty groups are not forced — healthy progress still omits placeholders when unused.
    let healthy = audit_screen("workflow-monitor", "workflow-monitor.progress")
        .expect("healthy progress must render");
    let healthy_text = healthy.render_txt();
    assert!(!healthy_text.lines().any(|line| {
        line.contains("Needs attention")
            && !line.contains("Needs attention:")
            && line.trim() == "Needs attention"
    }));
    assert!(healthy_text.contains("Working"));
    assert!(healthy_text.contains("Recently completed"));
}

#[test]
fn first_progress_disclosure_is_human_tasks_without_internals() {
    let audit = audit_screen("workflow-monitor", "workflow-monitor.progress")
        .expect("first progress disclosure must render");
    let text = audit.render_txt();
    assert!(text.contains(TASK_2));
    assert!(text.contains(TASK_1));
    assert!(text.contains("Collapse progress"));
    forbidden_first_progress_internals(&text);
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "progress-group"
                || event.identity.family == "progress-stage"
                || event.identity.detail_level == "summary")
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.collapse-progress"
            && transition.to_screen_id == "workflow-monitor"
    }));
}

#[test]
fn workers_remain_hidden_until_explicit_detail() {
    let compact = audit_screen("workflow-monitor", "gate.mission.workflow-monitor")
        .expect("compact monitor");
    let compact_text = compact.render_txt();
    assert!(!compact_text.to_lowercase().contains("worker"));
    assert!(!compact_text.contains("Owner ·"));

    let progress = audit_screen("workflow-monitor", "workflow-monitor.progress")
        .expect("progress summary");
    let progress_text = progress.render_txt();
    assert!(!progress_text.to_lowercase().contains("worker ·"));
    assert!(!progress_text.contains("Owner ·"));
    assert!(
        progress_text.contains("View workers") || progress_text.contains("Open team board"),
        "progress must offer an explicit route into worker/team detail:\n{progress_text}"
    );

    let workers = audit_screen("team-board", "team-board.workers")
        .expect("worker detail must render");
    workers.validate().expect("worker detail audit");
    let workers_text = workers.render_txt();
    assert!(workers_text.contains("Workers") || workers_text.contains("worker"));
    assert!(
        workers_text.contains("Release reviewer")
            || workers_text.contains("Docs lead")
            || workers_text.contains("Notes specialist")
    );
    assert!(workers_text.contains("Return to"));
    assert!(workers.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.workers-return"
            || transition.action_id == "action.mission.team-board-return"
            || transition.path.contains("return")
    }));
}

#[test]
fn team_board_opens_explicitly_as_temporary_detail() {
    let board = audit_screen("team-board", "orchestration.team-board")
        .expect("team board must render");
    board.validate().expect("team board audit must stay bijective");
    let text = board.render_txt();
    assert!((text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL));
    assert!(text.contains("Team board") || text.contains("ownership and assignments"));
    assert!(
        text.contains("ownership") || text.contains("assignments") || text.contains("Owner ·"),
        "board must declare ownership/assignment purpose:\n{text}"
    );
    assert!(text.contains("Return to mission monitor") || text.contains("Return to mission"));
    assert!(board.transitions.iter().any(|transition| {
        (transition.action_id == "action.mission.team-board-return"
            || transition.action_id == "action.mission.team-board-open")
            && (transition.to_screen_id == "workflow-monitor"
                || transition.from_screen_id == "workflow-monitor"
                || transition.to_screen_id == "team-board")
    }));

    let open = replay_case("mission-open-team-board", 1).expect("open team board case");
    assert_eq!(open.before.screen_id, "workflow-monitor");
    assert_eq!(open.after.screen_id, "team-board");
    assert_eq!(open.actions[0].action_id, "action.mission.open-team-board");
    assert_eq!(open.before.user_task, open.after.user_task);
    assert_eq!(open.before.visible_origin, open.after.visible_origin);
    assert_eq!(open.before.session_id, open.after.session_id);

    let back = replay_case("mission-team-board-return", 1).expect("return from team board");
    assert_eq!(back.before.screen_id, "team-board");
    assert_eq!(back.after.screen_id, "workflow-monitor");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.team-board-return"
    );
}

#[test]
fn team_board_tasks_are_human_readable_with_blocked_first() {
    let audit = audit_screen("team-board", "orchestration.team-board")
        .expect("readable board must render");
    let text = audit.render_txt();
    assert!(text.contains(TASK_3) || text.contains("safe publish"));
    assert!(text.contains("Depends on") || text.contains("Depends ·"));
    assert!(text.contains("Owner ·"));
    assert!(
        text.contains("Needs attention")
            || text.contains("Blocked")
            || text.contains("blocked"),
        "blocked work must remain visible:\n{text}"
    );
    // Internal scheduling must stay out of the first board level.
    assert!(!text.contains("Wave ·"));
    assert!(!text.contains("Worktree ·"));
    assert!(!text.contains("acceptance-contract"));
    assert!(!text.contains("/tmp/"));

    let needs_row = text
        .lines()
        .position(|line| line.contains("Needs attention") || line.contains(TASK_3))
        .expect("needs-attention content");
    let completed_row = text
        .lines()
        .position(|line| line.contains("Recently completed") || line.contains(TASK_1))
        .expect("completed content");
    assert!(
        needs_row < completed_row,
        "blocked ownership rows must appear before completed work:\n{text}"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "board-task")
    );
}

#[test]
fn board_internals_require_second_disclosure() {
    let board = audit_screen("team-board", "orchestration.team-board").expect("initial board");
    let board_text = board.render_txt();
    assert!(
        board_text.contains("View advanced") || board_text.contains("advanced detail"),
        "board must expose an advanced disclosure action:\n{board_text}"
    );
    assert!(!board_text.contains("Wave ·"));
    assert!(!board_text.contains("Worktree ·"));
    assert!(!board_text.contains("Branch · release"));

    let advanced = audit_screen("team-board", "team-board.advanced")
        .expect("advanced board detail must render");
    advanced.validate().expect("advanced board audit");
    let advanced_text = advanced.render_txt();
    assert!(advanced_text.contains("Wave ·") || advanced_text.contains("wave"));
    assert!(advanced_text.contains("Branch ·") || advanced_text.contains("branch"));
    assert!(advanced_text.contains("Worktree ·") || advanced_text.contains("worktree"));
    assert!(
        advanced_text.contains("Contract ·") || advanced_text.contains("contract"),
        "advanced detail should expose contract metadata:\n{advanced_text}"
    );
    assert!(
        advanced_text.contains("Advanced") || advanced_text.contains("detail"),
        "advanced state must be labeled as detail:\n{advanced_text}"
    );
    assert!(advanced_text.contains("Return to team board"));
    assert!(advanced.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.team-board-advanced-return"
            && transition.to_screen_id == "team-board"
    }));

    let open_adv = replay_case("mission-team-board-advanced", 1).expect("open advanced");
    assert_eq!(open_adv.before.screen_id, "team-board");
    assert_eq!(open_adv.after.screen_id, "team-board");
    assert_eq!(
        open_adv.actions[0].action_id,
        "action.mission.team-board-advanced"
    );
    assert_eq!(open_adv.before.user_task, open_adv.after.user_task);

    let close_adv = replay_case("mission-team-board-advanced-return", 1).expect("close advanced");
    assert_eq!(close_adv.before.screen_id, "team-board");
    assert_eq!(close_adv.after.screen_id, "team-board");
    assert_eq!(
        close_adv.actions[0].action_id,
        "action.mission.team-board-advanced-return"
    );
}

#[test]
fn progress_detail_is_bounded_with_exact_omission_and_one_group_expand() {
    let bounded = audit_screen("workflow-monitor", "workflow-monitor.progress.bounded")
        .expect("oversized progress must render");
    bounded.validate().expect("bounded progress audit");
    let text = bounded.render_txt();
    assert!(text.contains("Needs attention"));
    assert!(text.contains("Working"));
    assert!(text.contains("Recently completed"));
    // Exact omitted counts reconstruct the full set.
    assert!(
        text.contains("3 more") || text.contains("… 3 more") || text.contains("and 3 more"),
        "bounded progress must state exact omitted counts:\n{text}"
    );
    // Still no worker/advanced evidence on expand of the list itself.
    assert!(!text.contains("Owner ·"));
    assert!(!text.contains("Wave ·"));
    assert!(!text.contains("Worktree ·"));
    assert!(
        text.contains("Show all") || text.contains("Expand"),
        "bounded progress must offer local group expansion:\n{text}"
    );
    assert!(
        bounded
            .events
            .iter()
            .any(|event| event.identity.family == "progress-bound"
                || event.identity.subject_id.contains("omitted")
                || event.identity.family == "progress-group")
    );

    let expanded = audit_screen(
        "workflow-monitor",
        "workflow-monitor.progress.expand-working",
    )
    .expect("one-group expansion must render");
    let expand_text = expanded.render_txt();
    assert!(expand_text.contains("Working"));
    // Expanded group lists more than the default bound without opening workers.
    let working_tasks = expand_text
        .lines()
        .filter(|line| {
            line.contains("●")
                || line.contains("task ")
                || line.to_lowercase().contains("working item")
                || line.contains("Update")
                || line.contains("Prepare")
                || line.contains("Document")
                || line.contains("Align")
                || line.contains("Draft")
        })
        .count();
    assert!(
        working_tasks >= 5,
        "expanded Working group should reveal the full local set:\n{expand_text}"
    );
    assert!(!expand_text.contains("Owner ·"));
    assert!(!expand_text.contains("Wave ·"));
    assert!(
        expand_text.contains("Back") || expand_text.contains("Collapse group"),
        "group expansion must restore via Back:\n{expand_text}"
    );

    let open_group = replay_case("mission-progress-expand-working", 1).expect("expand group");
    assert_eq!(open_group.before.screen_id, "workflow-monitor");
    assert_eq!(open_group.after.screen_id, "workflow-monitor");
    assert_eq!(
        open_group.actions[0].action_id,
        "action.mission.progress-expand-group"
    );
    assert_eq!(open_group.before.session_id, open_group.after.session_id);
    assert_eq!(open_group.before.user_task, open_group.after.user_task);

    let collapse_group =
        replay_case("mission-progress-collapse-group", 1).expect("collapse group");
    assert_eq!(collapse_group.before.screen_id, "workflow-monitor");
    assert_eq!(collapse_group.after.screen_id, "workflow-monitor");
    assert_eq!(
        collapse_group.actions[0].action_id,
        "action.mission.progress-collapse-group"
    );
    assert!(
        collapse_group.before.selection.contains("working")
            || collapse_group.after.selection.contains("working")
            || collapse_group.after.focus.contains("progress")
    );
}

#[test]
fn view_workers_from_progress_reaches_worker_detail() {
    let replay = replay_case("mission-view-workers", 1).expect("view workers");
    assert_eq!(replay.before.screen_id, "workflow-monitor");
    assert_eq!(replay.after.screen_id, "team-board");
    assert_eq!(replay.actions[0].action_id, "action.mission.view-workers");
    assert_eq!(replay.before.user_task, replay.after.user_task);
    assert_eq!(replay.before.session_id, replay.after.session_id);
}
