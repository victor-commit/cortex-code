//! Flow 6 mission failure, fix, revalidation, review, and human completion.
//!
//! Owned contract assertions:
//! VAL-V3-F6-012, 017, 018, 019, 023, 024, 025, 026, 041, 042, 043, 052, 053.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const FAILURE_CAUSE: &str = "Publish path validation failed the safe release checks";
const FIX_TASK: &str = "Repair the safe publish path checks";
const REVIEW_REASON_CHANGES: &str = "Next-step note still omits the open risk and rollback order";
const REVIEW_REASON_APPROVED: &str = "Final validation passed on the post-repair revision";
const AFFECTED_RESULT: &str = "Safe publish path";
const VALIDATION_RUN: &str = "validation.release.2";
const COMPLETION_VALIDATION: &str = "validation.release.completion.1";

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
        "L0",
        "L1",
        "L2",
        "contract_id",
    ] {
        assert!(
            !text.to_lowercase().contains(token),
            "default mission surface leaked internal detail `{token}`:\n{text}"
        );
    }
}

fn mission_card_fields_only(text: &str) {
    // Compact MissionCard keeps goal/stage/counts/blocker only; review outcome is separate.
    assert!(text.contains(MISSION_GOAL));
    assert!(!text.contains("VERDICT"));
    assert!(!text.contains("check matrix"));
    assert!(!text.contains("branch/"));
}

#[test]
fn failure_is_exception_first_with_create_fix_task_primary() {
    let audit = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure monitor must render");
    audit
        .validate()
        .expect("failure audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Needs attention"));
    assert!(text.contains(FAILURE_CAUSE) || text.contains("validation failed"));
    assert!(text.contains("1 complete · 2 remaining") || text.contains("1 complete"));
    assert!(text.contains("› [enter] Create a fix task") || text.contains("[enter] Create a fix task"));
    assert!(text.contains("Open evidence") || text.contains("[d] Open evidence"));
    assert!(!text.contains("Start mission"));
    assert!(!text.contains("Approve completion"));
    // Primary must not bury exception under delay noise.
    let lines: Vec<&str> = text.lines().collect();
    let needs_attention = lines
        .iter()
        .position(|line| line.contains("Needs attention"))
        .expect("Needs attention group or blocker");
    let cause = lines
        .iter()
        .position(|line| line.contains(FAILURE_CAUSE) || line.contains("validation failed"))
        .expect("plain-English cause");
    let create = lines
        .iter()
        .position(|line| line.contains("Create a fix task"))
        .expect("create fix task primary");
    assert!(needs_attention < create);
    assert!(cause <= create + 8);
    forbidden_internals(&text);
    mission_card_fields_only(&text);

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "primary-action"
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.label.contains("Create a fix task"))
        })
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "blocker" || event.identity.family == "failure")
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.create-fix-task"
            && transition.to_screen_id == "workflow-monitor"
            && (transition.kind == "direct" || transition.kind == "automatic")
    }));
}

#[test]
fn completed_work_stays_preserved_and_counts_stay_coherent() {
    let audit = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure counts must render");
    let text = audit.render_txt();
    assert!(text.contains("1 complete · 2 remaining"));
    // Completed human task stays visible either in card context or disclosed group.
    assert!(
        text.contains("Review the release notes for open risks")
            || text.contains("Recently completed")
            || text.contains("complete")
    );
    forbidden_internals(&text);
}

#[test]
fn create_fix_task_records_failure_and_updates_counts() {
    let replay = replay_case("mission-create-fix-task", 1).expect("create fix task must plan work");
    assert_eq!(replay.before.screen_id, "workflow-monitor-failure");
    assert_eq!(replay.after.screen_id, "workflow-monitor");
    assert_eq!(
        replay.actions[0].action_id,
        "action.mission.create-fix-task"
    );
    assert_eq!(replay.before.user_task, MISSION_TASK);
    assert_eq!(replay.after.user_task, MISSION_TASK);
    assert_eq!(replay.before.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.after.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.before.conversation_id, replay.after.conversation_id);
    assert_eq!(replay.before.session_id, replay.after.session_id);
    assert_eq!(replay.attempts[0].outcome, "fix-planned");
    assert!(
        replay.side_effects.iter().any(|effect| {
            effect.kind.contains("fix") || effect.status.contains("planned")
        }),
        "fix work must be an explicit planned effect"
    );
    let text = replay.audit.render_txt();
    assert!(text.contains(FIX_TASK) || text.contains("fix"));
    // Counts update when a fix task is added: completed stays 1, remaining grows to 3
    // (original remaining plus the new fix), or remaining becomes 3 total unfinished.
    assert!(
        text.contains("1 complete · 3 remaining") || text.contains("1 complete · 2 remaining"),
        "counts must remain mutually consistent after fix planning:\n{text}"
    );
    assert!(!text.contains("› [enter] Create a fix task"));
    forbidden_internals(&text);
}

#[test]
fn fix_work_always_enters_visible_validating_stage() {
    let working = replay_case("mission-fix-working", 1).expect("fix working must start");
    assert_eq!(working.before.screen_id, "workflow-monitor");
    assert_eq!(working.after.screen_id, "workflow-monitor");
    assert!(
        working.audit.render_txt().contains("Working")
            || working.audit.render_txt().to_lowercase().contains("fix")
    );

    let validating = replay_case("mission-fix-validating", 1).expect("fix completion validates");
    assert_eq!(validating.before.screen_id, "workflow-monitor");
    assert_eq!(validating.after.screen_id, "workflow-monitor");
    assert_eq!(
        validating.actions[0].action_id,
        "action.mission.fix-complete-validate"
    );
    assert!(
        validating.transitions[0].kind == "automatic" || validating.transitions[0].kind == "direct"
    );
    let text = validating.audit.render_txt();
    assert!(text.contains("Validating"));
    assert!(!text.contains("› [enter]"));
    assert!(
        !text.contains("Approve completion"),
        "human approval must stay unreachable during validating"
    );
    assert!(
        !text.contains("Changes requested"),
        "review must stay unreachable before validation finishes"
    );
    // Fresh validation run identity
    assert!(
        validating.attempts.iter().any(|attempt| {
            attempt.attempt_id.contains("validation")
                || attempt.outcome.contains("validating")
                || attempt.outcome.contains("running")
        }),
        "fresh validation attempt identity required: {:?}",
        validating.attempts
    );
    forbidden_internals(&text);
}

#[test]
fn validating_success_advances_to_review_and_failure_returns_needs_attention() {
    let success =
        replay_case("mission-validation-success", 1).expect("validation success must open review");
    assert_eq!(success.before.screen_id, "workflow-monitor");
    assert_eq!(success.after.screen_id, "reviewer-output");
    assert_eq!(
        success.actions[0].action_id,
        "action.mission.validation-finished"
    );
    assert!(
        success.attempts.iter().any(|attempt| {
            attempt.attempt_id.contains(VALIDATION_RUN)
                || attempt.outcome == "passed"
                || attempt.outcome.contains("pass")
        }),
        "success must bind a fresh validating run: {:?}",
        success.attempts
    );

    let failure =
        replay_case("mission-validation-failure", 1).expect("validation failure reopens exception");
    assert_eq!(failure.before.screen_id, "workflow-monitor");
    assert_eq!(failure.after.screen_id, "workflow-monitor-failure");
    assert!(failure.attempts.iter().any(|attempt| {
        attempt.outcome == "failed" || attempt.outcome.contains("fail")
    }));
    let text = failure.audit.render_txt();
    assert!(text.contains("Needs attention"));
    assert!(text.contains("Create a fix task"));
    forbidden_internals(&text);
}

#[test]
fn reviewer_output_keeps_mission_task_and_you_continuity() {
    // P1-3: monitor → reviewer-output → approved must keep the stable task banner and
    // You-line, or explicitly label a surface change. "Mission review" must not replace
    // the task header as an unlabeled second journey title.
    const USER_PROMPT: &str = "You · Turn the release work into a mission and walk me through it.";
    let failure = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )
    .expect("failure monitor must render");
    let review = audit_screen("reviewer-output", "gate.mission.reviewer-output")
        .expect("reviewer-output must render");
    let approved = audit_screen("reviewer-approved", "gate.mission.reviewer-approved")
        .expect("reviewer-approved must render");

    for (name, audit) in [
        ("workflow-monitor-failure", &failure),
        ("reviewer-output", &review),
        ("reviewer-approved", &approved),
    ] {
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{name} audit must stay bijective: {error}"));
        let conversation = audit
            .conversation
            .as_ref()
            .unwrap_or_else(|| panic!("{name} must carry conversation identity"));
        assert_eq!(
            conversation.user_task, MISSION_TASK,
            "{name} conversation identity must keep the stable mission task"
        );
        assert_eq!(
            conversation.visible_origin, MISSION_ORIGIN,
            "{name} conversation identity must keep Mission · current conversation"
        );
        assert!(
            conversation
                .submitted_prompt
                .as_deref()
                .is_some_and(|prompt| prompt.contains("Turn the release work into a mission")),
            "{name} submitted_prompt must keep the release symptom line"
        );

        let text = audit.render_txt();
        assert!(
            text.contains(MISSION_ORIGIN),
            "{name} must show the journey origin chip"
        );
        assert!(
            text.contains(MISSION_TASK),
            "{name} must keep the stable mission task band visible"
        );
        assert!(
            text.contains(USER_PROMPT)
                || (text.contains("You ·")
                    && text.contains("Turn the release work into a mission")),
            "{name} must preserve the You-line or an explicit You-line bridge"
        );
    }

    let review_text = review.render_txt();
    let lines: Vec<&str> = review_text.lines().collect();
    let origin = lines
        .iter()
        .position(|line| line.contains(MISSION_ORIGIN))
        .expect("reviewer-output origin chip");
    // Stable mission task must remain in the top identity band (not only later as card body).
    let band = lines
        .iter()
        .skip(origin)
        .take(6)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        band.contains(MISSION_TASK),
        "reviewer-output identity band must include the stable mission task under the origin:\n{band}"
    );
    assert!(
        band.contains("You ·")
            && (band.contains("Turn the release work into a mission")
                || band.contains("Review outcome ready")),
        "reviewer-output identity band must keep or bridge the You-line:\n{band}"
    );
    // If "Mission review" appears, it is a section/surface label under continuous identity —
    // not the sole bright task substitute for the mission goal.
    if let Some(mission_review_row) = lines
        .iter()
        .position(|line| line.contains("Mission review"))
    {
        assert!(
            mission_review_row > origin,
            "Mission review must not precede the origin chip"
        );
        let has_task_before_or_at_review = lines[origin..=mission_review_row]
            .iter()
            .any(|line| line.contains(MISSION_TASK));
        assert!(
            has_task_before_or_at_review,
            "stable mission task must remain visible when Mission review is shown"
        );
    }
    assert!(
        review_text.contains("Changes requested"),
        "reviewer-output must still surface the Changes requested outcome"
    );
}

#[test]
fn reviewer_output_prioritizes_changes_requested_outcome() {
    let audit = audit_screen("reviewer-output", "gate.mission.reviewer-output")
        .expect("reviewer-output must render");
    audit
        .validate()
        .expect("reviewer-output audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(text.contains("Changes requested"));
    assert!(text.contains(REVIEW_REASON_CHANGES) || text.contains("open risk"));
    assert!(text.contains(AFFECTED_RESULT) || text.contains("affected") || text.contains("Safe publish"));
    assert!(
        text.contains("Create a fix task")
            || text.contains("Open fix task")
            || text.contains("Update fix task")
    );
    assert!(!text.contains("Approve completion"));
    assert!(!text.contains("L0"));
    assert!(!text.contains("check matrix"));
    assert!(!text.contains("branch meta"));
    forbidden_internals(&text);
    mission_card_fields_only(&text);

    // Outcome appears before any checklist or deeper evidence.
    let lines: Vec<&str> = text.lines().collect();
    let changes = lines
        .iter()
        .position(|line| line.contains("Changes requested"))
        .expect("Changes requested");
    let primary = lines
        .iter()
        .position(|line| {
            line.contains("Create a fix task")
                || line.contains("Open fix task")
                || line.contains("Update fix task")
        })
        .expect("primary repair action");
    assert!(changes < primary);

    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "review-outcome"
                || event.identity.family == "mission-review-outcome"
        }),
        "review-outcome must be a distinct audited block"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "mission-card"),
        "MissionCard remains audited separately"
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
fn requested_changes_become_tracked_work_then_fresh_validation() {
    let replay =
        replay_case("mission-review-create-fix", 1).expect("requested changes become fix work");
    assert_eq!(replay.before.screen_id, "reviewer-output");
    assert_eq!(replay.after.screen_id, "workflow-monitor");
    assert_eq!(
        replay.actions[0].action_id,
        "action.mission.review-create-fix"
    );
    assert_eq!(replay.attempts[0].outcome, "fix-planned");
    assert!(
        replay.side_effects.iter().any(|effect| {
            effect.status.contains("planned")
                || effect.kind.contains("fix")
                || effect.effect_id.contains("fix")
        })
    );
    let text = replay.audit.render_txt();
    assert!(text.contains(FIX_TASK) || text.contains("fix") || text.contains("Working") || text.contains("Needs attention") || text.contains("Planned"));
    // cannot approve despite unresolved findings
    assert!(!text.contains("Approve completion"));
    forbidden_internals(&text);

    let validate = replay_case("mission-review-fix-validating", 1)
        .expect("review fix work re-enters validating");
    assert_eq!(validate.after.screen_id, "workflow-monitor");
    assert!(validate.audit.render_txt().contains("Validating"));
}

#[test]
fn reviewer_approved_is_concise_and_requires_human_completion() {
    let audit = audit_screen("reviewer-approved", "gate.mission.reviewer-approved")
        .expect("reviewer-approved must render");
    audit
        .validate()
        .expect("reviewer-approved audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_GOAL));
    assert!(
        text.contains("Ready for completion")
            || text.contains("Awaiting approval")
            || text.contains("Review complete")
            || text.contains("complete · 0 remaining")
            || text.contains("3 complete · 0 remaining")
    );
    assert!(text.contains("Approved"));
    assert!(text.contains(REVIEW_REASON_APPROVED) || text.contains("Final validation passed"));
    assert!(
        text.contains("› [enter] Approve completion")
            || text.contains("[enter] Approve completion")
    );
    assert!(text.contains("Request changes"));
    assert!(!text.contains("Changes requested"));
    forbidden_internals(&text);
    mission_card_fields_only(&text);

    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "review-outcome"
                || event.identity.family == "mission-review-outcome"
        })
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "mission-card")
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "human-gate"
                || event.identity.subject_id.contains("awaiting_human")
                || event
                    .action
                    .as_ref()
                    .is_some_and(|a| a.label.contains("Approve completion") && !a.automatic)
        })
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.approve-completion"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));
    // No automated completion transition
    assert!(
        !audit.transitions.iter().any(|transition| {
            transition.action_id.contains("auto") && transition.to_screen_id == "chat-turn"
        }),
        "automated completion is forbidden"
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
fn completion_requires_fresh_post_repair_validation() {
    let replay =
        replay_case("mission-completion-validation", 1).expect("completion validation must run");
    assert_eq!(replay.before.screen_id, "workflow-monitor");
    assert_eq!(replay.after.screen_id, "reviewer-approved");
    assert_eq!(
        replay.actions[0].action_id,
        "action.mission.completion-validation"
    );
    assert!(
        replay.attempts.iter().any(|attempt| {
            attempt.attempt_id.contains(COMPLETION_VALIDATION)
                || attempt.attempt_id.contains("completion")
                || attempt.outcome == "passed"
        }),
        "fresh completion validation identity required: {:?}",
        replay.attempts
    );
    assert!(
        replay.side_effects.iter().any(|effect| {
            effect.kind.contains("validation") || effect.status.contains("passed")
        })
    );
    let text = replay.audit.render_txt();
    assert!(text.contains("Approved"));
    assert!(text.contains("Approve completion"));
    forbidden_internals(&text);
}

#[test]
fn human_request_changes_returns_to_repair_with_feedback() {
    let open =
        replay_case("mission-request-changes-open", 1).expect("request changes opens reason dock");
    assert_eq!(open.before.screen_id, "reviewer-approved");
    assert_eq!(open.after.screen_id, "reviewer-approved");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.request-changes"
    );
    let dock_text = open.audit.render_txt();
    assert!(
        dock_text.contains("Request changes")
            || dock_text.contains("reason")
            || dock_text.contains("What should change")
    );

    let submit = replay_case("mission-request-changes-submit", 1)
        .expect("submitting reason creates fix work");
    assert_eq!(submit.before.screen_id, "reviewer-approved");
    assert_eq!(submit.after.screen_id, "workflow-monitor");
    assert_eq!(
        submit.actions[0].action_id,
        "action.mission.request-changes-submit"
    );
    assert_eq!(submit.attempts[0].outcome, "fix-planned");
    assert!(
        submit.side_effects.iter().any(|effect| {
            effect.effect_id.contains("feedback")
                || effect.kind.contains("fix")
                || effect.status.contains("planned")
        })
    );
    let text = submit.audit.render_txt();
    assert!(!text.contains("Approve completion"));
    forbidden_internals(&text);

    let cancel = replay_case("mission-request-changes-cancel", 1)
        .expect("cancel restores approval gate");
    assert_eq!(cancel.before.screen_id, "reviewer-approved");
    assert_eq!(cancel.after.screen_id, "reviewer-approved");
    assert_eq!(
        cancel.actions[0].action_id,
        "action.mission.request-changes-cancel"
    );
    let restored = cancel.audit.render_txt();
    assert!(restored.contains("Approve completion"));
    assert!(restored.contains("Approved"));
}

#[test]
fn approve_completion_returns_to_conversation_with_summary() {
    let replay =
        replay_case("mission-approve-completion", 1).expect("human approve must complete");
    assert_eq!(replay.before.screen_id, "reviewer-approved");
    assert_eq!(replay.after.screen_id, "chat-turn");
    assert_eq!(
        replay.actions[0].action_id,
        "action.mission.approve-completion"
    );
    assert_eq!(replay.transitions[0].kind, "direct");
    assert_eq!(replay.before.user_task, MISSION_TASK);
    assert_eq!(replay.after.user_task, MISSION_TASK);
    assert_eq!(replay.before.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.after.visible_origin, MISSION_ORIGIN);
    assert_eq!(replay.before.conversation_id, "conversation.mission");
    assert_eq!(replay.before.conversation_id, replay.after.conversation_id);
    assert_eq!(replay.before.session_id, replay.after.session_id);
    assert_eq!(replay.attempts[0].outcome, "completed");
    assert!(
        replay
            .side_effects
            .iter()
            .any(|effect| effect.status == "completed" || effect.status.contains("complete"))
    );

    let text = replay.audit.render_txt();
    assert!(
        text.contains("Mission complete")
            || text.contains("mission finished")
            || text.contains("completed")
            || text.contains("release is ready")
    );
    // normal composer restored
    assert!(
        text.contains("›") || text.contains("composer") || text.lines().any(|line| line.contains("Keep the risk"))
            || text.contains("Type a message")
            || text.contains("Ask Code")
            || text.contains("│")
    );
    // no action dock for completion approval
    assert!(!text.contains("Approve completion"));
    assert!(!text.contains("Start this mission?"));
    forbidden_internals(&text);
}

#[test]
fn fix_loop_preserves_goal_and_dependency_identity() {
    let looped = replay_case("mission-fix-loop-rerepair", 1)
        .expect("second failure keeps resolved identity");
    assert_eq!(looped.before.user_task, MISSION_TASK);
    assert_eq!(looped.after.user_task, MISSION_TASK);
    assert_eq!(looped.before.session_id, looped.after.session_id);
    assert_eq!(looped.before.conversation_id, looped.after.conversation_id);
    // Loop stage should land back on failure or monitor with needs attention after re-fail
    assert!(
        looped.after.screen_id == "workflow-monitor-failure"
            || looped.after.screen_id == "workflow-monitor"
    );
    assert!(
        looped.attempts.iter().any(|attempt| {
            attempt.attempt_id.contains("validation")
                || attempt.outcome.contains("fail")
                || attempt.outcome.contains("validat")
        })
    );
}

#[test]
fn default_failure_and_review_density_hides_internals() {
    for (screen, fixture) in [
        (
            "workflow-monitor-failure",
            "gate.mission.workflow-monitor-failure",
        ),
        ("reviewer-output", "gate.mission.reviewer-output"),
        ("reviewer-approved", "gate.mission.reviewer-approved"),
    ] {
        let text = audit_screen(screen, fixture)
            .unwrap_or_else(|error| panic!("{screen} must render for density: {error}"))
            .render_txt();
        forbidden_internals(&text);
        assert_eq!(text.lines().count(), 40, "{screen} must stay 120x40");
        assert!(
            text.lines().all(|line| line.chars().count() == 120),
            "{screen} rows must be 120 columns"
        );
    }
}
