//! Flow 6 reversible mission evidence disclosure navigation.
//!
//! Owned contract assertions:
//! VAL-V3-F6-036, VAL-V3-F6-049.
//!
//! Compact summary -> focused human detail -> advanced evidence must always
//! unwind one rendered level with exact origin, counts, blocker, selection,
//! and focus preservation. Deeper views never become permanent dashboards.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const SESSION: &str = "session.mission.1";
const TASK_COUNTS_FAILURE: &str = "1 complete · 2 remaining";
const FAILURE_BLOCKER: &str = "Publish path validation failed the safe release checks";

fn assert_identity_preserved(
    user_task_a: &str,
    user_task_b: &str,
    session_a: &str,
    session_b: &str,
    origin_a: &str,
    origin_b: &str,
) {
    assert_eq!(user_task_a, user_task_b, "mission user task must stay stable");
    assert_eq!(session_a, session_b, "mission session identity must stay stable");
    assert_eq!(origin_a, origin_b, "visible origin must stay stable");
}

fn assert_has_return(text: &str, screen: &str) {
    assert!(
        text.contains("Return")
            || text.contains("Return to")
            || text.contains("Collapse progress")
            || text.contains("Back"),
        "{screen} needs an explicit return to its immediate origin:\n{text}"
    );
}

fn assert_not_permanent_dashboard(text: &str, screen: &str) {
    // Evidence details must remain temporary with a return, not a new home shell.
    assert!(
        !text.contains("› composer ready") || text.contains("Return") || text.contains("Collapse"),
        "{screen} must not turn evidence into a permanent composer dashboard:\n{text}"
    );
    // Opening one evidence surface must not dump every advanced category at once.
    let advanced_markers = [
        "Wave ·",
        "Worktree ·",
        "Throughput ·",
        "Payload ·",
        "Credential fields",
        "Intent fidelity ·",
        "Held-out checks ·",
    ];
    let advanced_hits = advanced_markers
        .iter()
        .filter(|marker| text.contains(**marker))
        .count();
    // A single advanced surface may include its own domain, but not the union of all.
    assert!(
        advanced_hits <= 4,
        "{screen} dumped unrelated advanced categories together ({advanced_hits}):\n{text}"
    );
}

#[test]
fn every_evidence_screen_returns_to_immediate_origin() {
    // VAL-V3-F6-036: named evidence view -> obvious return -> exact origin restore.
    let matrix = [
        (
            "team-board",
            "orchestration.team-board",
            "mission-team-board-return",
            "workflow-monitor",
            "action.mission.team-board-return",
        ),
        (
            "verification-ladder",
            "orchestration.verification-ladder",
            "mission-verification-ladder-return",
            "workflow-monitor-failure",
            "action.mission.verification-ladder-return",
        ),
        (
            "intent-scorer-pass",
            "orchestration.intent-scorer-pass",
            "mission-intent-scorer-pass-return",
            "reviewer-approved",
            "action.mission.intent-scorer-return",
        ),
        (
            "intent-scorer-fail",
            "orchestration.intent-scorer-fail",
            "mission-intent-scorer-fail-return",
            "reviewer-output",
            "action.mission.intent-scorer-return",
        ),
        (
            "concurrency-metrics",
            "orchestration.concurrency-metrics",
            "mission-concurrency-metrics-return",
            "workflow-monitor",
            "action.mission.concurrency-return",
        ),
        (
            "tool-envelope",
            "orchestration.tool-envelope",
            "mission-tool-envelope-return",
            "verification-ladder",
            "action.mission.tool-envelope-return",
        ),
    ];

    for (screen, fixture, case_id, origin_screen, action_id) in matrix {
        let audit = audit_screen(screen, fixture).expect("evidence screen must render");
        audit
            .validate()
            .expect("evidence audit must stay bijective");
        let text = audit.render_txt();
        assert!(
            (text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL),
            "{screen} must retain the mission identity:\n{text}"
        );
        assert_has_return(&text, screen);
        assert_not_permanent_dashboard(&text, screen);
        assert!(
            audit.transitions.iter().any(|transition| {
                transition.action_id == action_id
                    && transition.from_screen_id == screen
                    && transition.to_screen_id == origin_screen
            }),
            "{screen} must audit a single-level return to {origin_screen} via {action_id}: {:?}",
            audit.transitions
        );

        let back = replay_case(case_id, 1).expect("return case must exist");
        assert_eq!(back.before.screen_id, screen);
        assert_eq!(back.after.screen_id, origin_screen);
        assert_eq!(back.actions[0].action_id, action_id);
        assert_identity_preserved(
            &back.before.user_task,
            &back.after.user_task,
            &back.before.session_id,
            &back.after.session_id,
            &back.before.visible_origin,
            &back.after.visible_origin,
        );
        assert_eq!(back.before.session_id, SESSION);
        // Selection and focus restore to the origin surface, not the nested leaf.
        assert!(
            !back.after.focus.is_empty() && back.after.focus != back.before.focus
                || back.after.selection != back.before.selection
                || back.after.scroll_anchor != back.before.scroll_anchor,
            "{case_id} must restore origin focus/selection rather than leave the nested leaf active"
        );
    }
}

#[test]
fn nested_stage_evidence_is_distinct_and_unwinds_one_level() {
    // VAL-V3-F6-036 / 049: ladder focused detail admits one deeper stage-evidence
    // level that returns to the ladder, not to the failure monitor in one jump.
    let ladder = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("focused validation stages");
    let ladder_text = ladder.render_txt();
    assert!(
        ladder_text.contains("Open detailed evidence")
            || ladder_text.contains("View stage evidence")
            || ladder_text.contains("detailed evidence"),
        "focused ladder must offer one nested stage evidence action:\n{ladder_text}"
    );
    // Focused ladder must not already dump the deepest payload corpus.
    assert!(
        !ladder_text.contains("Assertion detail ·")
            && !ladder_text.contains("Hidden assertion body"),
        "focused ladder must not already fully expand stage-evidence payload:\n{ladder_text}"
    );

    let nested = audit_screen("verification-ladder", "verification-ladder.stage-evidence")
        .expect("nested stage evidence must render as a distinct fixture");
    nested
        .validate()
        .expect("nested stage evidence audit stays bijective");
    let nested_text = nested.render_txt();
    assert!(
        nested_text.contains("Held-out")
            || nested_text.contains("held-out")
            || nested_text.contains("Stage evidence"),
        "nested level must name the focused stage:\n{nested_text}"
    );
    assert!(
        nested_text.contains("Assertion")
            || nested_text.contains("evidence body")
            || nested_text.contains("Expected")
            || nested_text.contains("Observed"),
        "nested stage evidence must add distinct information:\n{nested_text}"
    );
    // Deeper evidence is not the full waterfall of every mission internal.
    assert!(!nested_text.contains("Wave ·"));
    assert!(!nested_text.contains("Worktree ·"));
    assert!(!nested_text.contains("Approve completion"));
    assert!(
        nested_text.contains("Return to validation stages")
            || nested_text.contains("Return to validation stages")
            || nested_text.contains("Return to Validation stages"),
        "nested stage evidence must return only one level to the ladder:\n{nested_text}"
    );
    assert!(nested.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.stage-evidence-return"
            && transition.to_screen_id == "verification-ladder"
            && transition.from_screen_id == "verification-ladder"
    }));
    // Nested disclosure must not also offer a skip-level jump as the primary return.
    assert!(
        !nested.transitions.iter().any(|transition| {
            transition.action_id == "action.mission.stage-evidence-return"
                && transition.to_screen_id == "workflow-monitor-failure"
        }),
        "stage evidence must not skip levels back to the failure monitor"
    );

    let open = replay_case("mission-open-stage-evidence", 1).expect("open stage evidence");
    assert_eq!(open.before.screen_id, "verification-ladder");
    assert_eq!(open.after.screen_id, "verification-ladder");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.view-stage-evidence"
    );
    assert_identity_preserved(
        &open.before.user_task,
        &open.after.user_task,
        &open.before.session_id,
        &open.after.session_id,
        &open.before.visible_origin,
        &open.after.visible_origin,
    );
    // Mission counts / blocker / selection identity ride on selection + focus + pending.
    assert_eq!(open.before.user_task, open.after.user_task);
    assert!(
        open.after.focus.contains("stage-evidence")
            || open.after.selection.contains("held-out")
            || open.after.scroll_anchor.contains("stage-evidence"),
        "opening nested evidence must mark the deeper focus: focus={} selection={} anchor={}",
        open.after.focus,
        open.after.selection,
        open.after.scroll_anchor
    );

    let back = replay_case("mission-stage-evidence-return", 1).expect("return stage evidence");
    assert_eq!(back.before.screen_id, "verification-ladder");
    assert_eq!(back.after.screen_id, "verification-ladder");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.stage-evidence-return"
    );
    // Restore the focused ladder selection (held-out), not the damage we opened with.
    assert!(
        back.after.selection.contains("held-out")
            || back.after.focus.contains("validation-stages")
            || back.after.scroll_anchor.contains("verification-ladder"),
        "stage evidence return must restore ladder selection/focus: selection={} focus={} anchor={}",
        back.after.selection,
        back.after.focus,
        back.after.scroll_anchor
    );
    assert_identity_preserved(
        &back.before.user_task,
        &back.after.user_task,
        &back.before.session_id,
        &back.after.session_id,
        &back.before.visible_origin,
        &back.after.visible_origin,
    );
}

#[test]
fn failure_ladder_tool_chain_is_compact_focused_advanced_and_reversible() {
    // VAL-V3-F6-049: compact failure summary -> focused validation stages ->
    // advanced tool envelope, each level adding distinct information, reverse one by one.
    let compact = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("compact failure summary");
    let compact_text = compact.render_txt();
    assert!(compact_text.contains(MISSION_GOAL));
    assert!(compact_text.contains(TASK_COUNTS_FAILURE));
    assert!(
        compact_text.contains(FAILURE_BLOCKER) || compact_text.contains("Validation failed"),
        "compact card must keep the blocker:\n{compact_text}"
    );
    assert!(
        compact_text.contains("Open evidence") || compact_text.contains("open evidence"),
        "compact summary must offer focused disclosure:\n{compact_text}"
    );
    // Compact must not already expand every advanced domain.
    assert!(!compact_text.contains("Payload ·"));
    assert!(!compact_text.contains("Deterministic checks ·"));
    assert!(!compact_text.contains("Throughput ·"));

    let open_ladder =
        replay_case("mission-open-verification-ladder", 1).expect("open ladder from failure");
    assert_eq!(open_ladder.before.screen_id, "workflow-monitor-failure");
    assert_eq!(open_ladder.after.screen_id, "verification-ladder");
    assert_identity_preserved(
        &open_ladder.before.user_task,
        &open_ladder.after.user_task,
        &open_ladder.before.session_id,
        &open_ladder.after.session_id,
        &open_ladder.before.visible_origin,
        &open_ladder.after.visible_origin,
    );

    let focused = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("focused ladder");
    let focused_text = focused.render_txt();
    assert!(
        focused_text.contains("Validation stages") || focused_text.contains("validation stages")
    );
    assert!(focused_text.contains("Held-out checks"));
    assert!(
        !focused_text.contains("Wave ·") && !focused_text.contains("Throughput ·"),
        "focused ladder must not dump unrelated advanced categories:\n{focused_text}"
    );

    let open_tools = replay_case("mission-open-tool-envelope", 1).expect("open tool envelope");
    assert_eq!(open_tools.before.screen_id, "verification-ladder");
    assert_eq!(open_tools.after.screen_id, "tool-envelope");
    assert_identity_preserved(
        &open_tools.before.user_task,
        &open_tools.after.user_task,
        &open_tools.before.session_id,
        &open_tools.after.session_id,
        &open_tools.before.visible_origin,
        &open_tools.after.visible_origin,
    );

    let advanced = audit_screen("tool-envelope", "orchestration.tool-envelope")
        .expect("advanced tool envelope");
    let advanced_text = advanced.render_txt();
    assert!(advanced_text.contains("Tool evidence") || advanced_text.contains("tool evidence"));
    assert!(advanced_text.contains("Payload") || advanced_text.contains("payload") || advanced_text.contains("truncated") || advanced_text.contains("redacted"));
    // Distinct from the ladder's stage list: tool outcomes, not validation stage names as only content.
    assert!(
        advanced_text.contains("Success")
            || advanced_text.contains("Execution error")
            || advanced_text.contains("Refusal"),
        "advanced tool envelope must add distinct tool outcomes:\n{advanced_text}"
    );
    assert!(!advanced_text.contains("Wave ·"));
    assert!(!advanced_text.contains("Approve completion"));

    // Unwind one level at a time: tools -> ladder, then ladder -> failure.
    let back_tools = replay_case("mission-tool-envelope-return", 1).expect("tools return ladder");
    assert_eq!(back_tools.after.screen_id, "verification-ladder");
    assert_eq!(
        back_tools.actions[0].action_id,
        "action.mission.tool-envelope-return"
    );
    assert!(
        back_tools.after.selection.contains("held-out")
            || back_tools.after.focus.contains("validation-stages"),
        "tool envelope return restores ladder selection/focus"
    );

    let back_ladder =
        replay_case("mission-verification-ladder-return", 1).expect("ladder return failure");
    assert_eq!(back_ladder.after.screen_id, "workflow-monitor-failure");
    assert_eq!(
        back_ladder.actions[0].action_id,
        "action.mission.verification-ladder-return"
    );

    // After both returns the compact failure still owns the same mission counts/blocker.
    let restored = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("restored failure");
    let restored_text = restored.render_txt();
    assert!(restored_text.contains(TASK_COUNTS_FAILURE));
    assert!(
        restored_text.contains(FAILURE_BLOCKER) || restored_text.contains("Validation failed")
    );
    assert!(restored_text.contains("Create a fix task"));
}

#[test]
fn monitor_progress_board_chain_unwinds_without_dumping_internals() {
    // Compact monitor -> focused progress/team board -> advanced board internals.
    let compact =
        audit_screen("workflow-monitor", "gate.mission.workflow-monitor").expect("compact monitor");
    let compact_text = compact.render_txt();
    assert!(compact_text.contains(MISSION_GOAL));
    assert!(compact_text.contains("1 complete · 2 remaining"));
    assert!(!compact_text.contains("Wave ·"));
    assert!(!compact_text.contains("Owner ·"));
    assert!(!compact_text.contains("Throughput ·"));

    let progress = replay_case("mission-view-progress", 1).expect("open progress");
    assert_eq!(progress.before.screen_id, "workflow-monitor");
    assert_eq!(progress.after.screen_id, "workflow-monitor");
    assert_eq!(
        progress.actions[0].action_id,
        "action.mission.view-progress"
    );
    assert_identity_preserved(
        &progress.before.user_task,
        &progress.after.user_task,
        &progress.before.session_id,
        &progress.after.session_id,
        &progress.before.visible_origin,
        &progress.after.visible_origin,
    );

    let focused =
        audit_screen("workflow-monitor", "workflow-monitor.progress").expect("focused progress");
    let focused_text = focused.render_txt();
    assert!(focused_text.contains("Working") || focused_text.contains("Recently completed"));
    assert!(!focused_text.contains("Wave ·"));
    assert!(!focused_text.contains("Worktree ·"));
    assert!(
        focused_text.contains("Open team board") || focused_text.contains("View workers"),
        "focused progress may open board/worker detail, not advanced internals en masse"
    );

    let open_board = replay_case("mission-open-team-board", 1).expect("open board");
    assert_eq!(open_board.after.screen_id, "team-board");
    let board =
        audit_screen("team-board", "orchestration.team-board").expect("focused ownership board");
    let board_text = board.render_txt();
    assert!(board_text.contains("Owner ·") || board_text.contains("ownership"));
    assert!(!board_text.contains("Wave ·"));
    assert!(!board_text.contains("Worktree ·"));

    let open_adv = replay_case("mission-team-board-advanced", 1).expect("open advanced board");
    assert_eq!(open_adv.after.screen_id, "team-board");
    let advanced =
        audit_screen("team-board", "team-board.advanced").expect("advanced board internals");
    let advanced_text = advanced.render_txt();
    assert!(advanced_text.contains("Wave ·") || advanced_text.contains("wave"));
    assert!(advanced_text.contains("Worktree ·") || advanced_text.contains("worktree"));
    // Advanced board must not also become a permanent dashboard + concurrency + tool dump.
    assert!(!advanced_text.contains("Payload ·"));
    assert!(!advanced_text.contains("Throughput ·"));
    assert!(!advanced_text.contains("Approve completion"));
    assert!(advanced_text.contains("Return to team board"));

    // One level at a time unwind.
    let close_adv =
        replay_case("mission-team-board-advanced-return", 1).expect("close advanced board");
    assert_eq!(close_adv.after.screen_id, "team-board");
    assert_eq!(
        close_adv.actions[0].action_id,
        "action.mission.team-board-advanced-return"
    );

    let close_board = replay_case("mission-team-board-return", 1).expect("close board");
    assert_eq!(close_board.after.screen_id, "workflow-monitor");
    assert_eq!(
        close_board.actions[0].action_id,
        "action.mission.team-board-return"
    );

    // Concurrently, advanced runtime detail remains its own reversible branch.
    let open_runtime =
        replay_case("mission-open-concurrency-metrics", 1).expect("open concurrency");
    assert_eq!(open_runtime.before.screen_id, "workflow-monitor");
    assert_eq!(open_runtime.after.screen_id, "concurrency-metrics");
    let runtime = audit_screen("concurrency-metrics", "orchestration.concurrency-metrics")
        .expect("runtime detail");
    let runtime_text = runtime.render_txt();
    assert!(runtime_text.contains("Runtime detail") || runtime_text.contains("Held work"));
    assert!(!runtime_text.contains("Wave · release"));
    assert!(!runtime_text.contains("Approve completion"));
    let close_runtime =
        replay_case("mission-concurrency-metrics-return", 1).expect("close concurrency");
    assert_eq!(close_runtime.after.screen_id, "workflow-monitor");
    assert_identity_preserved(
        &close_runtime.before.user_task,
        &close_runtime.after.user_task,
        &close_runtime.before.session_id,
        &close_runtime.after.session_id,
        &close_runtime.before.visible_origin,
        &close_runtime.after.visible_origin,
    );
}

#[test]
fn score_detail_chain_is_focused_advisory_and_returns_exactly_once() {
    // Compact review summary -> focused score detail, no permanent score dashboard.
    let open_fail = replay_case("mission-open-intent-scorer-fail", 1).expect("open fail score");
    assert_eq!(open_fail.before.screen_id, "reviewer-output");
    assert_eq!(open_fail.after.screen_id, "intent-scorer-fail");
    assert_identity_preserved(
        &open_fail.before.user_task,
        &open_fail.after.user_task,
        &open_fail.before.session_id,
        &open_fail.after.session_id,
        &open_fail.before.visible_origin,
        &open_fail.after.visible_origin,
    );

    let fail = audit_screen("intent-scorer-fail", "orchestration.intent-scorer-fail")
        .expect("fail score");
    let fail_text = fail.render_txt();
    assert!(fail_text.contains("Scope") || fail_text.contains("failed"));
    assert!(fail_text.contains("Return to review outcome") || fail_text.contains("Return"));
    assert!(!fail_text.contains("Wave ·"));
    assert!(!fail_text.contains("Throughput ·"));
    assert!(!fail_text.to_lowercase().contains("approve completion"));

    let back_fail =
        replay_case("mission-intent-scorer-fail-return", 1).expect("return fail score");
    assert_eq!(back_fail.after.screen_id, "reviewer-output");
    assert!(
        back_fail.after.selection.contains("create-fix")
            || back_fail.after.focus.contains("review"),
        "score return restores review selection/focus"
    );

    let open_pass = replay_case("mission-open-intent-scorer-pass", 1).expect("open pass score");
    assert_eq!(open_pass.before.screen_id, "reviewer-approved");
    assert_eq!(open_pass.after.screen_id, "intent-scorer-pass");
    let pass = audit_screen("intent-scorer-pass", "orchestration.intent-scorer-pass")
        .expect("pass score");
    let pass_text = pass.render_txt();
    assert!(pass_text.contains("advisory") || pass_text.contains("never completes") || pass_text.contains("still needs"));
    assert!(pass_text.contains("Return to human completion") || pass_text.contains("Return"));
    let back_pass =
        replay_case("mission-intent-scorer-pass-return", 1).expect("return pass score");
    assert_eq!(back_pass.after.screen_id, "reviewer-approved");
    assert!(
        back_pass.after.selection.contains("approve")
            || back_pass.after.focus.contains("human-completion")
            || back_pass.after.focus.contains("completion"),
        "pass score return restores human completion selection/focus"
    );
}

#[test]
fn opening_one_level_never_expands_unrelated_categories() {
    // VAL-V3-F6-049 negative shape: each first open leaves other advanced domains closed.
    let ladder = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("ladder focused")
        .render_txt();
    assert!(!ladder.to_lowercase().contains("wave ·"));
    assert!(!ladder.to_lowercase().contains("worktree"));
    assert!(!ladder.to_lowercase().contains("throughput"));
    assert!(!ladder.to_lowercase().contains("intent fidelity · full"));

    let board = audit_screen("team-board", "orchestration.team-board")
        .expect("board focused")
        .render_txt();
    assert!(!board.contains("Payload ·"));
    assert!(!board.contains("Held-out checks"));
    assert!(!board.contains("Throughput ·"));
    assert!(!board.contains("score /"));

    let progress = audit_screen("workflow-monitor", "workflow-monitor.progress")
        .expect("progress focused")
        .render_txt();
    assert!(!progress.contains("Wave ·"));
    assert!(!progress.contains("Worktree ·"));
    assert!(!progress.contains("Payload ·"));
    assert!(!progress.contains("Intent fidelity"));

    let concurrency = audit_screen("concurrency-metrics", "orchestration.concurrency-metrics")
        .expect("runtime focused")
        .render_txt();
    assert!(!concurrency.contains("Wave · release"));
    assert!(!concurrency.contains("Contract ·"));
    assert!(!concurrency.contains("Payload ·"));
    assert!(!concurrency.contains("Approve completion"));
}

#[test]
fn scenario_transitions_for_evidence_chain_have_no_dead_ends() {
    // Every open edge has a matching reverse / one-level return; no unexplained category jump.
    let opens = [
        (
            "mission-open-verification-ladder",
            "workflow-monitor-failure",
            "verification-ladder",
        ),
        (
            "mission-open-stage-evidence",
            "verification-ladder",
            "verification-ladder",
        ),
        (
            "mission-open-tool-envelope",
            "verification-ladder",
            "tool-envelope",
        ),
        (
            "mission-open-team-board",
            "workflow-monitor",
            "team-board",
        ),
        (
            "mission-team-board-advanced",
            "team-board",
            "team-board",
        ),
        (
            "mission-open-concurrency-metrics",
            "workflow-monitor",
            "concurrency-metrics",
        ),
        (
            "mission-open-intent-scorer-fail",
            "reviewer-output",
            "intent-scorer-fail",
        ),
        (
            "mission-open-intent-scorer-pass",
            "reviewer-approved",
            "intent-scorer-pass",
        ),
    ];
    let returns = [
        (
            "mission-verification-ladder-return",
            "verification-ladder",
            "workflow-monitor-failure",
        ),
        (
            "mission-stage-evidence-return",
            "verification-ladder",
            "verification-ladder",
        ),
        (
            "mission-tool-envelope-return",
            "tool-envelope",
            "verification-ladder",
        ),
        (
            "mission-team-board-return",
            "team-board",
            "workflow-monitor",
        ),
        (
            "mission-team-board-advanced-return",
            "team-board",
            "team-board",
        ),
        (
            "mission-concurrency-metrics-return",
            "concurrency-metrics",
            "workflow-monitor",
        ),
        (
            "mission-intent-scorer-fail-return",
            "intent-scorer-fail",
            "reviewer-output",
        ),
        (
            "mission-intent-scorer-pass-return",
            "intent-scorer-pass",
            "reviewer-approved",
        ),
    ];

    for (case_id, from, to) in opens.into_iter().chain(returns) {
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|err| panic!("{case_id} must be a public case: {err}"));
        assert_eq!(
            replay.before.screen_id, from,
            "{case_id} before screen"
        );
        assert_eq!(replay.after.screen_id, to, "{case_id} after screen");
        assert_eq!(
            replay.before.origin_stack, replay.after.origin_stack,
            "{case_id} must preserve origin stack"
        );
        assert_identity_preserved(
            &replay.before.user_task,
            &replay.after.user_task,
            &replay.before.session_id,
            &replay.after.session_id,
            &replay.before.visible_origin,
            &replay.after.visible_origin,
        );
        assert!(
            !replay.transitions.is_empty(),
            "{case_id} must declare an audited transition"
        );
        // No silent category jump to unrelated chat/auth/recovery loops.
        for transition in &replay.transitions {
            assert!(
                !matches!(
                    transition.to_screen_id.as_str(),
                    "login" | "connect-provider" | "home" | "onboarding"
                ),
                "{case_id} unexplained jump to {}",
                transition.to_screen_id
            );
        }
    }
}
