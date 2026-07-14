//! Flow 6 validation, score, concurrency, and tool evidence views.
//!
//! Owned contract assertions:
//! VAL-V3-F6-027, VAL-V3-F6-028, VAL-V3-F6-029, VAL-V3-F6-030, VAL-V3-F6-031,
//! VAL-V3-F6-032, VAL-V3-F6-033, VAL-V3-F6-034, VAL-V3-F6-035.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const FIX_TASK: &str = "Repair the safe publish path checks";

fn assert_no_internals_leak(text: &str) {
    for phrase in [
        "L0",
        "L1",
        "L2",
        "L3",
        "L4",
        "verification ladder",
        "intent scorer",
        "concurrency metrics",
        "tool envelope",
        "tool-envelope",
        "wave schedule",
        "Throughput ·",
        "quarantine",
        "scorecard",
        "payload:",
        "\"api_key\"",
        "sk-",
        "Bearer ",
        "score /",
    ] {
        assert!(
            !text.to_lowercase().contains(&phrase.to_lowercase()),
            "default mission surface leaked evidence internal `{phrase}`:\n{text}"
        );
    }
}

fn default_surface_fixtures() -> [(&'static str, &'static str); 6] {
    [
        ("workflow-launch", "gate.mission.workflow-launch"),
        ("workflow-monitor", "gate.mission.workflow-monitor"),
        (
            "workflow-monitor-failure",
            "gate.mission.workflow-monitor-failure",
        ),
        ("reviewer-output", "gate.mission.reviewer-output"),
        ("reviewer-approved", "gate.mission.reviewer-approved"),
        ("team-board", "orchestration.team-board"),
    ]
}

#[test]
fn validation_scores_runtime_and_tool_evidence_absent_from_defaults() {
    for (screen, fixture) in default_surface_fixtures() {
        let audit = audit_screen(screen, fixture).expect("default surface renders");
        let text = audit.render_txt();
        assert_no_internals_leak(&text);
        assert!(
            !text.contains("Validation stages"),
            "{screen} opened validation ladder by default:\n{text}"
        );
        assert!(
            !text.contains("Quality report · pass")
                && !text.contains("Quality report · needs repair")
                && !text.contains("Intent fidelity · full"),
            "{screen} opened quality report detail by default:\n{text}"
        );
        assert!(
            !text.contains("Runtime detail")
                && !text.contains("Held work ·")
                && !text.contains("Quarantine ·"),
            "{screen} opened concurrency detail by default:\n{text}"
        );
        assert!(
            !text.contains("Tool evidence")
                && !text.contains("Execution error ·")
                && !text.contains("did not run"),
            "{screen} opened tool envelope by default:\n{text}"
        );
    }
}

#[test]
fn verification_ladder_opens_on_request_with_origin_return() {
    let audit = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("verification ladder must render");
    audit
        .validate()
        .expect("verification ladder audit must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_ORIGIN) || text.contains(MISSION_GOAL));
    assert!(
        text.contains("Validation stages") || text.contains("validation stages"),
        "ladder must identify itself as validation stages:\n{text}"
    );
    // Decision-relevant stages with readable purposes, not raw L0-L4 labels.
    assert!(
        !text.contains("L0") && !text.contains("L1") && !text.contains("L2"),
        "ladder must not lead with L0-L4 machine labels:\n{text}"
    );
    assert!(
        text.contains("Deterministic checks")
            || text.contains("Agent-visible tests")
            || text.contains("Held-out checks")
            || text.contains("Intent audit")
            || text.contains("Process check"),
        "stages need readable purpose labels:\n{text}"
    );
    assert!(
        text.contains("pass")
            || text.contains("Pass")
            || text.contains("fail")
            || text.contains("Fail")
            || text.contains("pending")
            || text.contains("Pending"),
        "stage outcomes must be explicit:\n{text}"
    );
    assert!(
        text.contains("[esc] Return") || text.contains("Return to"),
        "ladder needs a direct return:\n{text}"
    );

    let open = replay_case("mission-open-verification-ladder", 1)
        .expect("open verification ladder case");
    assert_eq!(open.before.screen_id, "workflow-monitor-failure");
    assert_eq!(open.after.screen_id, "verification-ladder");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.view-validation-stages"
    );
    assert_eq!(open.before.user_task, open.after.user_task);
    assert_eq!(open.before.session_id, open.after.session_id);
    assert_eq!(open.before.visible_origin, open.after.visible_origin);

    let back = replay_case("mission-verification-ladder-return", 1)
        .expect("return from verification ladder");
    assert_eq!(back.before.screen_id, "verification-ladder");
    assert_eq!(back.after.screen_id, "workflow-monitor-failure");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.verification-ladder-return"
    );
}

#[test]
fn verification_ladder_highlights_current_failing_or_pending_stage() {
    let audit = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("ladder stages");
    let text = audit.render_txt();
    // Failing or pending stage is textually distinct.
    assert!(
        text.contains("fail")
            || text.contains("Fail")
            || text.contains("needs repair")
            || text.contains("Needs repair")
            || text.contains("pending")
            || text.contains("Pending"),
        "current failure/pending stage must be named in plain language:\n{text}"
    );
    // Glyphs distinguish status without relying on color alone.
    assert!(
        text.contains("✓") || text.contains("✗") || text.contains("●") || text.contains("○"),
        "stage status needs non-color glyphs:\n{text}"
    );
    // Further evidence is a nested disclosure, not the only content.
    assert!(
        text.contains("Open detailed evidence")
            || text.contains("View stage evidence")
            || text.contains("detailed evidence"),
        "nested evidence disclosure should remain one step further:\n{text}"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "validation-stage"
                || event.identity.family == "verification-stage"
                || event.identity.detail_level == "focused"
                || event.identity.detail_level == "evidence")
    );
}

#[test]
fn intent_scorer_pass_opens_only_on_request_and_stays_advisory() {
    let audit = audit_screen("intent-scorer-pass", "orchestration.intent-scorer-pass")
        .expect("pass scorer must render");
    audit.validate().expect("pass scorer audit");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_ORIGIN) || text.contains(MISSION_GOAL));
    assert!(
        text.contains("Pass") || text.contains("passed") || text.contains("passing"),
        "pass scorer must state the pass result:\n{text}"
    );
    assert!(
        text.contains("dimension")
            || text.contains("Intent fidelity")
            || text.contains("Execution evidence")
            || text.contains("Scope"),
        "supporting dimensions must remain visible:\n{text}"
    );
    // Advisory only — no lifecycle control.
    assert!(
        !text.to_lowercase().contains("approve completion")
            && !text.to_lowercase().contains("complete mission")
            && !text.to_lowercase().contains("start mission")
            && !text.to_lowercase().contains("stop mission"),
        "scores must not control lifecycle:\n{text}"
    );
    assert!(
        text.contains("advisory")
            || text.contains("never completes")
            || text.contains("still needs")
            || text.contains("human completion"),
        "pass score should explain that human validation still applies:\n{text}"
    );
    assert!(text.contains("[esc] Return") || text.contains("Return to"));

    let open = replay_case("mission-open-intent-scorer-pass", 1).expect("open pass score");
    assert_eq!(open.before.screen_id, "reviewer-approved");
    assert_eq!(open.after.screen_id, "intent-scorer-pass");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.view-score-detail"
    );
    assert_eq!(open.before.user_task, open.after.user_task);

    let back = replay_case("mission-intent-scorer-pass-return", 1).expect("return pass score");
    assert_eq!(back.before.screen_id, "intent-scorer-pass");
    assert_eq!(back.after.screen_id, "reviewer-approved");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.intent-scorer-return"
    );

    // No score-to-complete edge from the detail view itself.
    assert!(
        !audit.transitions.iter().any(|t| {
            t.action_id.contains("approve-completion")
                || (t.to_screen_id == "chat-turn" && t.kind == "direct" && t.path.contains("score"))
        }),
        "score detail must not provide a complete edge: {:?}",
        audit.transitions
    );
}

#[test]
fn intent_scorer_fail_names_dimension_and_routes_to_repair() {
    let audit = audit_screen("intent-scorer-fail", "orchestration.intent-scorer-fail")
        .expect("fail scorer must render");
    audit.validate().expect("fail scorer audit");
    let text = audit.render_txt();
    assert!(
        text.contains("Fail") || text.contains("failed") || text.contains("Needs repair"),
        "fail scorer must state the failure:\n{text}"
    );
    assert!(
        text.contains("Scope")
            || text.contains("Intent")
            || text.contains("dimension")
            || text.contains("honesty")
            || text.contains("fideli"),
        "failed dimension must be named in plain English:\n{text}"
    );
    assert!(
        text.contains("Create a fix task")
            || text.contains("Update the fix task")
            || text.contains("fix task"),
        "fail score must offer a repair route:\n{text}"
    );
    assert!(
        !text.to_lowercase().contains("approve completion"),
        "failing score must not offer completion:\n{text}"
    );
    assert!(text.contains(FIX_TASK) || text.contains("publish path") || text.contains("safe"));

    let open = replay_case("mission-open-intent-scorer-fail", 1).expect("open fail score");
    assert_eq!(open.before.screen_id, "reviewer-output");
    assert_eq!(open.after.screen_id, "intent-scorer-fail");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.view-score-detail"
    );

    let repair = replay_case("mission-intent-scorer-fail-create-fix", 1)
        .expect("fail score routes to fix");
    assert_eq!(repair.before.screen_id, "intent-scorer-fail");
    assert_eq!(repair.after.screen_id, "workflow-monitor");
    assert!(
        repair.actions[0].action_id == "action.mission.review-create-fix"
            || repair.actions[0].action_id == "action.mission.create-fix-task"
            || repair.actions[0].action_id == "action.mission.scorer-create-fix"
    );

    let back = replay_case("mission-intent-scorer-fail-return", 1).expect("return fail score");
    assert_eq!(back.before.screen_id, "intent-scorer-fail");
    assert_eq!(back.after.screen_id, "reviewer-output");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.intent-scorer-return"
    );
}

#[test]
fn concurrency_metrics_open_on_request_and_exception_first() {
    let audit = audit_screen("concurrency-metrics", "orchestration.concurrency-metrics")
        .expect("concurrency metrics must render");
    audit.validate().expect("concurrency audit");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_ORIGIN) || text.contains(MISSION_GOAL));
    assert!(
        text.contains("Runtime detail")
            || text.contains("Concurrency")
            || text.contains("Held work")
            || text.contains("Conflict"),
        "concurrency detail needs a readable title:\n{text}"
    );
    // Exception-first: conflict/hold/quarantine before healthy throughput.
    let exception_pos = text
        .lines()
        .position(|line| {
            let lower = line.to_lowercase();
            lower.contains("conflict")
                || lower.contains("held")
                || lower.contains("quarantine")
                || lower.contains("hold")
                || lower.contains("blocked")
        })
        .unwrap_or_else(|| panic!("exceptions must appear:\n{text}"));
    let healthy_pos = text.lines().position(|line| {
        let lower = line.to_lowercase();
        lower.contains("throughput")
            || lower.contains("healthy")
            || lower.contains("workers ready")
            || lower.contains("clear waves")
            || lower.contains("running cleanly")
    });
    if let Some(h) = healthy_pos {
        assert!(
            exception_pos < h,
            "exceptions must precede healthy throughput:\n{text}"
        );
    }
    // Consequence for the user, not raw wave dumps as headline.
    assert!(
        text.contains("pause")
            || text.contains("replan")
            || text.contains("retry")
            || text.contains("decision")
            || text.contains("waits")
            || text.contains("blocked"),
        "each exception needs a plain-English consequence:\n{text}"
    );
    assert!(text.contains("[esc] Return") || text.contains("Return to mission"));

    let open = replay_case("mission-open-concurrency-metrics", 1).expect("open concurrency");
    assert_eq!(open.before.screen_id, "workflow-monitor");
    assert_eq!(open.after.screen_id, "concurrency-metrics");
    assert_eq!(
        open.actions[0].action_id,
        "action.mission.view-runtime-detail"
    );
    // Opening metrics does not change scheduling; identity preserved.
    assert_eq!(open.before.user_task, open.after.user_task);
    assert_eq!(open.before.session_id, open.after.session_id);

    let back = replay_case("mission-concurrency-metrics-return", 1).expect("return concurrency");
    assert_eq!(back.before.screen_id, "concurrency-metrics");
    assert_eq!(back.after.screen_id, "workflow-monitor");
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.concurrency-return"
    );
}

#[test]
fn tool_envelope_distinguishes_outcomes_without_secrets() {
    let audit = audit_screen("tool-envelope", "orchestration.tool-envelope")
        .expect("tool envelope must render");
    audit.validate().expect("tool envelope audit");
    let text = audit.render_txt();
    assert!(text.contains(MISSION_ORIGIN) || text.contains(MISSION_GOAL));
    assert!(
        text.contains("Tool evidence") || text.contains("tool evidence"),
        "tool detail needs a human title:\n{text}"
    );
    // Three distinct outcomes.
    assert!(
        text.contains("Success") || text.contains("succeeded") || text.contains("✓"),
        "success outcome missing:\n{text}"
    );
    assert!(
        text.contains("Execution error")
            || text.contains("error")
            || text.contains("failed")
            || text.contains("✗"),
        "execution error outcome missing:\n{text}"
    );
    assert!(
        text.contains("Refusal")
            || text.contains("refused")
            || text.contains("did not run")
            || text.contains("○"),
        "refusal outcome missing:\n{text}"
    );
    assert!(
        text.contains("did not run") || text.contains("never ran") || text.contains("not run"),
        "refusal must explicitly mean the tool did not run:\n{text}"
    );
    // Redaction / truncation markers without secrets.
    assert!(
        text.contains("truncated")
            || text.contains("redacted")
            || text.contains("[redacted]")
            || text.contains("…"),
        "payload detail should show redaction or truncation:\n{text}"
    );
    for secret in [
        "sk-ant-",
        "sk-proj-",
        "Bearer ey",
        "api_key\": \"",
        "password\": \"",
        "SECRET_TOKEN",
    ] {
        assert!(
            !text.contains(secret),
            "tool evidence leaked secret pattern `{secret}`:\n{text}"
        );
    }
    // Linked to originating validation or task.
    assert!(
        text.contains("Safe publish")
            || text.contains("publish path")
            || text.contains("validation")
            || text.contains(FIX_TASK)
            || text.contains("origin"),
        "evidence must bind to an origin task/validation:\n{text}"
    );
    assert!(text.contains("[esc] Return") || text.contains("Return to"));

    let open = replay_case("mission-open-tool-envelope", 1).expect("open tool envelope");
    assert!(
        open.before.screen_id == "verification-ladder"
            || open.before.screen_id == "workflow-monitor-failure"
            || open.before.screen_id == "workflow-monitor"
    );
    assert_eq!(open.after.screen_id, "tool-envelope");
    assert!(
        open.actions[0].action_id == "action.mission.view-tool-evidence"
            || open.actions[0].action_id == "action.mission.open-tool-envelope"
    );

    let back = replay_case("mission-tool-envelope-return", 1).expect("return tool envelope");
    assert_eq!(back.before.screen_id, "tool-envelope");
    assert!(
        back.after.screen_id == "verification-ladder"
            || back.after.screen_id == "workflow-monitor-failure"
    );
    assert_eq!(
        back.actions[0].action_id,
        "action.mission.tool-envelope-return"
    );
}
