//! Flow 6 interventions, replan, retry, stop, and durable resume.
//!
//! Owned contract assertions:
//! VAL-V3-F6-016, 037, 038, 039, 040, 054, 055, 057.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const ELIGIBLE_TASK: &str = "Check the safe publish path";
const MISSING_INPUT: &str = "Release checklist note is missing";

fn forbidden_internals(text: &str) {
    for token in [
        "worker",
        "wave",
        "worktree",
        "score",
        "ladder",
        "tool envelope",
        "acceptance-contract",
        "features.json",
        "N=3",
        "concurrency",
        "orchestrat",
        "contract_id",
    ] {
        assert!(
            !text.to_lowercase().contains(token),
            "mission surface leaked internal detail `{token}`:\n{text}"
        );
    }
}

fn primary_action_count(audit: &mockup_harness_v3::audit::AuditDocument) -> usize {
    audit
        .events
        .iter()
        .filter(|event| event.identity.family == "primary-action")
        .count()
}

fn preserved(replay: &mockup_harness_v3::scenario::CaseReplay, field: &str) -> (String, String) {
    let hash = replay
        .preserved_field_hashes
        .get(field)
        .unwrap_or_else(|| panic!("missing preserved field `{field}`"));
    (hash.before.clone(), hash.after.clone())
}

#[test]
fn healthy_monitor_keeps_view_progress_primary_and_intervenes_causes_no_toolbar() {
    let audit = audit_screen("workflow-monitor", "gate.mission.workflow-monitor")
        .expect("healthy monitor must render");
    audit.validate().expect("healthy monitor audit bijection");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("View progress"));
    assert!(text.contains("Intervene") || text.contains("[i] Intervene"));
    // Interventions are not equal primaries on the compact card/toolbar.
    assert!(!text.contains("[enter] Pause"));
    assert!(!text.contains("[enter] Replan"));
    assert!(!text.contains("[enter] Stop"));
    assert_eq!(primary_action_count(&audit), 1);
    forbidden_internals(&text);

    let open = replay_case("mission-open-intervene", 1).expect("open intervene");
    assert_eq!(open.before.screen_id, "workflow-monitor");
    assert_eq!(open.after.screen_id, "workflow-monitor");
    assert_eq!(open.actions[0].action_id, "action.mission.open-intervene");
    let dock = open.audit.render_txt();
    assert!(dock.contains("Pause"));
    assert!(dock.contains("Replan"));
    assert!(dock.contains("Stop"));
    assert!(dock.contains("›") || dock.contains("[enter]"));
    assert_eq!(primary_action_count(&open.audit), 1);
    forbidden_internals(&dock);
}

#[test]
fn pause_explains_safe_boundary_and_resume_runs_exactly_once() {
    let confirm = replay_case("mission-pause-open", 1).expect("pause open");
    assert_eq!(confirm.after.screen_id, "workflow-monitor");
    let confirm_txt = confirm.audit.render_txt();
    assert!(
        confirm_txt.contains("safe boundary")
            || confirm_txt.contains("Safe boundary")
            || confirm_txt.contains("finish this task")
            || confirm_txt.contains("finish the current task")
    );
    assert!(confirm_txt.contains("› [enter] Pause mission") || confirm_txt.contains("Pause mission"));
    assert!(confirm_txt.contains("Cancel"));
    assert_eq!(primary_action_count(&confirm.audit), 1);

    let paused = replay_case("mission-pause-confirm", 1).expect("pause confirm");
    assert_eq!(paused.after.screen_id, "workflow-monitor");
    let paused_txt = paused.audit.render_txt();
    assert!(paused_txt.contains("Paused"));
    assert!(paused_txt.contains("› [enter] Resume") || paused_txt.contains("[enter] Resume"));
    assert!(paused_txt.contains("Replan"));
    assert!(paused_txt.contains("Stop"));
    assert_eq!(primary_action_count(&paused.audit), 1);
    let (boundary_before, boundary_after) = preserved(&paused, "safe_boundary");
    assert_eq!(boundary_before, boundary_after);
    assert_eq!(boundary_after.len(), 64);
    let (work_before, work_after) = preserved(&paused, "completed_work");
    assert_eq!(work_before, work_after);
    let (inv_before, inv_after) = preserved(&paused, "invocation_identity");
    assert_eq!(inv_before, inv_after);

    let resume = replay_case("mission-resume", 1).expect("resume once");
    assert_eq!(resume.before.screen_id, "workflow-monitor");
    assert_eq!(resume.after.screen_id, "workflow-monitor");
    assert_eq!(resume.actions[0].action_id, "action.mission.resume");
    assert_eq!(resume.attempts[0].count, 1);
    assert_eq!(resume.attempts[0].outcome, "resumed-once");
    let resume_txt = resume.audit.render_txt();
    assert!(resume_txt.contains("Running"));
    assert!(!resume_txt.contains("Paused"));
    // completed work preserved and no abandon/rerun of finished work
    let (work_b, work_a) = preserved(&resume, "completed_work");
    assert_eq!(work_b, work_a);
    let (bound_b, bound_a) = preserved(&resume, "safe_boundary");
    assert_eq!(bound_b, bound_a);
    let (inv_b, inv_a) = preserved(&resume, "invocation_identity");
    assert_eq!(inv_b, inv_a);
    assert!(!resume_txt.to_lowercase().contains("rerun finished"));
    forbidden_internals(&resume_txt);
}

#[test]
fn replan_reports_exact_count_deltas_and_runs_fresh_preflight() {
    let open = replay_case("mission-replan-open", 1).expect("replan open");
    assert_eq!(open.after.screen_id, "workflow-launch");
    let proposal = open.audit.render_txt();
    assert!(proposal.contains("Mission proposal") || proposal.contains("Revised plan"));
    assert!(
        proposal.contains("Added")
            || proposal.contains("added")
            || proposal.contains("+1")
            || proposal.contains("1 added")
    );
    assert!(
        proposal.contains("Removed")
            || proposal.contains("removed")
            || proposal.contains("1 removed")
    );
    assert!(
        proposal.contains("Replaced")
            || proposal.contains("replaced")
            || proposal.contains("1 replaced")
    );
    assert!(
        proposal.contains("Deferred")
            || proposal.contains("deferred")
            || proposal.contains("1 deferred")
    );
    // Completed tasks stay outside mutation counts.
    assert!(
        !proposal.to_lowercase().contains("completed tasks removed")
            && !proposal.to_lowercase().contains("completed · removed")
    );
    assert_eq!(primary_action_count(&open.audit), 1);
    let (added_b, added_a) = preserved(&open, "replan_added");
    let (_removed_b, removed_a) = preserved(&open, "replan_removed");
    let (_replaced_b, replaced_a) = preserved(&open, "replan_replaced");
    let (_deferred_b, deferred_a) = preserved(&open, "replan_deferred");
    // after hashes are the reconciled counts for the proposal
    assert_eq!(added_a.len(), 64);
    assert_eq!(removed_a.len(), 64);
    assert_eq!(replaced_a.len(), 64);
    assert_eq!(deferred_a.len(), 64);
    assert_ne!(added_b, added_a); // before empty / after known
    let (completed_b, completed_a) = preserved(&open, "completed_work");
    assert_eq!(completed_b, completed_a);

    let accept = replay_case("mission-replan-accept", 1).expect("replan accept");
    assert_eq!(accept.before.screen_id, "workflow-launch");
    assert_eq!(accept.after.screen_id, "workflow-launch");
    assert_eq!(accept.transitions[0].kind, "automatic");
    let preflight = accept.audit.render_txt();
    assert!(preflight.contains("Preflight") || preflight.contains("preflight"));
    assert!(!preflight.contains("› [enter] Start mission"));
    assert!(!preflight.contains("[enter] Start mission"));
    forbidden_internals(&proposal);
    forbidden_internals(&preflight);
}

#[test]
fn retry_only_for_eligible_failed_work_preserves_completed() {
    let ineligible = audit_screen(
        "workflow-monitor-failure",
        "workflow-monitor-failure.missing-input",
    )
    .expect("missing-input failure");
    let ineligible_txt = ineligible.render_txt();
    assert!(ineligible_txt.contains(MISSING_INPUT) || ineligible_txt.contains("missing"));
    assert!(!ineligible_txt.contains("[enter] Retry"));
    assert!(!ineligible_txt.contains("› [enter] Retry"));
    forbidden_internals(&ineligible_txt);

    let eligible = audit_screen(
        "workflow-monitor-failure",
        "workflow-monitor-failure.retry-eligible",
    )
    .expect("retry eligible failure");
    let eligible_txt = eligible.render_txt();
    assert!(eligible_txt.contains("Retry") || eligible_txt.contains("retry"));
    assert!(eligible_txt.contains(ELIGIBLE_TASK) || eligible_txt.contains("safe publish path"));
    assert!(
        eligible_txt.contains("validation")
            || eligible_txt.contains("Validation")
            || eligible_txt.contains("rerun")
    );
    assert_eq!(primary_action_count(&eligible), 1);

    let retry = replay_case("mission-retry-eligible", 1).expect("scoped retry");
    assert_eq!(retry.after.screen_id, "workflow-monitor");
    assert_eq!(retry.actions[0].action_id, "action.mission.retry-task");
    let (work_b, work_a) = preserved(&retry, "completed_work");
    assert_eq!(work_b, work_a);
    let after_txt = retry.audit.render_txt();
    assert!(after_txt.contains("Validating") || after_txt.contains("Running"));
    assert!(!after_txt.to_lowercase().contains("rerun the whole mission"));
    forbidden_internals(&after_txt);
}

#[test]
fn stop_names_consequences_and_stopped_stays_inspectable_without_resume() {
    let open = replay_case("mission-stop-open", 1).expect("stop open");
    let open_txt = open.audit.render_txt();
    assert!(open_txt.contains("preserved") || open_txt.contains("Preserved"));
    assert!(open_txt.contains("abandon") || open_txt.contains("not run") || open_txt.contains("stopped"));
    assert!(open_txt.contains("› [enter] Stop mission") || open_txt.contains("Stop mission"));
    assert!(open_txt.contains("Cancel"));
    assert_eq!(primary_action_count(&open.audit), 1);

    let confirm = replay_case("mission-stop-confirm", 1).expect("stop confirm");
    assert_eq!(confirm.after.screen_id, "workflow-monitor");
    let stopped = confirm.audit.render_txt();
    assert!(stopped.contains("Stopped"));
    assert!(stopped.contains("not run") || stopped.contains("Not run") || stopped.contains("abandoned"));
    assert!(!stopped.contains("[enter] Resume"));
    assert!(!stopped.contains("› [enter] Resume"));
    assert!(
        stopped.contains("Open detail")
            || stopped.contains("Inspect")
            || stopped.contains("Return to conversation")
            || stopped.contains("Replan")
    );
    let (work_b, work_a) = preserved(&confirm, "completed_work");
    assert_eq!(work_b, work_a);
    let (inv_b, inv_a) = preserved(&confirm, "invocation_identity");
    assert_eq!(inv_b, inv_a);

    // Restart leaves stopped inspectable; no automatic resume.
    let restart = replay_case("mission-restart-stopped", 1).expect("restart stopped");
    assert_eq!(restart.after.screen_id, "workflow-monitor");
    let restart_txt = restart.audit.render_txt();
    assert!(restart_txt.contains("Stopped"));
    assert!(!restart_txt.contains("[enter] Resume"));
    assert_eq!(restart.attempts[0].outcome, "restored-stopped");
    assert_eq!(restart.attempts[0].count, 0);
    let (stage_b, stage_a) = preserved(&restart, "mission_stage");
    assert_eq!(stage_b, stage_a);
    forbidden_internals(&stopped);
    forbidden_internals(&restart_txt);
}

#[test]
fn resume_and_restart_preserve_stage_identity_without_duplicates() {
    for (case_id, expected_stage_marker) in [
        ("mission-restart-running", "Running"),
        ("mission-restart-paused", "Paused"),
        ("mission-restart-blocked", "Needs attention"),
        ("mission-restart-validating", "Validating"),
        ("mission-restart-reviewing", "Review"),
        ("mission-restart-awaiting-human", "Ready for completion"),
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|err| {
            panic!("{case_id} must restore mission stage: {err}");
        });
        assert_eq!(
            replay.before.session_id, replay.after.session_id,
            "{case_id} session identity"
        );
        assert_eq!(
            replay.before.user_task, MISSION_TASK,
            "{case_id} task continuity"
        );
        assert_eq!(replay.after.visible_origin, MISSION_ORIGIN);
        let (stage_b, stage_a) = preserved(&replay, "mission_stage");
        assert_eq!(stage_b, stage_a, "{case_id} stage hash");
        let (inv_b, inv_a) = preserved(&replay, "invocation_identity");
        assert_eq!(inv_b, inv_a, "{case_id} invocation");
        let (work_b, work_a) = preserved(&replay, "completed_work");
        assert_eq!(work_b, work_a, "{case_id} completed work");
        // No auto-advance except explicit resume case: restart leaves stage as-is.
        assert_eq!(
            replay.attempts[0].count, 0,
            "{case_id} must not invent work"
        );
        let text = replay.audit.render_txt();
        assert!(
            text.contains(expected_stage_marker),
            "{case_id} missing stage marker {expected_stage_marker}:\n{text}"
        );
        assert!(!text.to_lowercase().contains("duplicate mission"));
        // Do not silently rerun finished work on restore.
        assert!(!text.to_lowercase().contains("rerunning finished"));
        forbidden_internals(&text);
    }

    // Explicit resume remains exact-once and only from paused.
    let resume = replay_case("mission-resume", 1).expect("resume still once");
    assert_eq!(resume.attempts[0].count, 1);
    assert_eq!(resume.attempts[0].outcome, "resumed-once");
    let (entity_b, entity_a) = preserved(&resume, "mission_entity_count");
    assert_eq!(entity_b, entity_a);
}

#[test]
fn replan_identity_sets_reconcile_without_double_counting_completed() {
    let open = replay_case("mission-replan-open", 1).expect("reconcile replan");
    let (before_set, after_set) = preserved(&open, "task_identity_set");
    assert_eq!(before_set.len(), 64);
    assert_eq!(after_set.len(), 64);
    assert_ne!(before_set, after_set);
    let (completed_b, completed_a) = preserved(&open, "completed_task_ids");
    assert_eq!(completed_b, completed_a);
    // Topics used as labels in semantic events for reconciliation.
    assert!(open.audit.events.iter().any(|event| {
        event.identity.family == "replan-counts" || event.identity.family == "task-plan"
    }));
    let txt = open.audit.render_txt();
    // Exact delta wording in board copy (1 each for this fixture).
    let lower = txt.to_lowercase();
    assert!(lower.contains("1 added") || lower.contains("added · 1") || lower.contains("+1 added"));
    assert!(
        lower.contains("1 removed") || lower.contains("removed · 1") || lower.contains("-1 removed")
    );
    assert!(
        lower.contains("1 replaced")
            || lower.contains("replaced · 1")
            || lower.contains("~1 replaced")
    );
    assert!(
        lower.contains("1 deferred")
            || lower.contains("deferred · 1")
            || lower.contains("… 1 deferred")
    );
}

#[test]
fn cancel_paths_restore_origin_without_side_effects() {
    for case_id in [
        "mission-pause-cancel",
        "mission-stop-cancel",
        "mission-intervene-cancel",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|err| panic!("{case_id}: {err}"));
        assert_eq!(replay.after.screen_id, "workflow-monitor");
        assert_eq!(replay.transitions[0].kind, "cancel");
        assert_eq!(replay.attempts[0].count, 0);
        let text = replay.audit.render_txt();
        assert!(text.contains("View progress") || text.contains("Running") || text.contains("Paused") || text.contains(MISSION_GOAL));
        forbidden_internals(&text);
    }
}

#[test]
fn stopped_return_and_replan_do_not_auto_execute() {
    let ret = replay_case("mission-stopped-return", 1).expect("return from stopped");
    assert_eq!(ret.after.screen_id, "chat-turn");
    assert_eq!(ret.actions[0].action_id, "action.mission.stopped-return");
    assert_eq!(ret.attempts[0].count, 0);

    let replan = replay_case("mission-stopped-replan", 1).expect("replan from stopped");
    assert_eq!(replan.after.screen_id, "workflow-launch");
    assert_eq!(replan.actions[0].action_id, "action.mission.replan");
    // replan from stopped creates a proposal; no auto execution
    assert!(replan.audit.render_txt().contains("plan") || replan.audit.render_txt().contains("Plan"));
    assert!(!replan.audit.render_txt().contains("Start mission"));
}
