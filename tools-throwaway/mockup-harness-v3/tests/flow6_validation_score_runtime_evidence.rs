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

fn painted_actions(audit: &mockup_harness_v3::audit::AuditDocument) -> Vec<(String, String)> {
    audit
        .events
        .iter()
        .filter_map(|event| {
            event.action.as_ref().map(|action| {
                (action.action_id.clone(), action.label.clone())
            })
        })
        .collect()
}

fn assert_unique_action_labels(audit: &mockup_harness_v3::audit::AuditDocument) {
    let mut by_label: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (action_id, label) in painted_actions(audit) {
        by_label.entry(label).or_default().push(action_id);
    }
    for (label, action_ids) in by_label {
        assert_eq!(
            action_ids.len(),
            1,
            "visible label `{label}` dual-binds action_ids {action_ids:?}"
        );
    }
}

fn assert_action_painted(
    audit: &mockup_harness_v3::audit::AuditDocument,
    action_id: &str,
    expected_label: &str,
) {
    let text = audit.render_txt();
    assert!(
        text.contains(expected_label),
        "origin must paint `{expected_label}`:\n{text}"
    );
    let hit = audit.events.iter().find_map(|event| {
        event.action.as_ref().and_then(|action| {
            (action.action_id == action_id).then_some(action.label.as_str())
        })
    });
    assert_eq!(
        hit,
        Some(expected_label),
        "action `{action_id}` must bind painted label `{expected_label}`, got {hit:?}"
    );
}

#[test]
fn verification_ladder_opens_on_request_with_origin_return() {
    let audit = audit_screen("verification-ladder", "orchestration.verification-ladder")
        .expect("verification ladder must render");
    audit
        .validate()
        .expect("verification ladder audit must stay bijective");
    let text = audit.render_txt();
    assert!((text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL));
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
        text.contains("Return") || text.contains("Return to"),
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
fn score_and_runtime_origin_affordances_are_distinct_and_painted() {
    // Gate products may omit open paints; origin evidence uses disclosure-safe
    // non-gate fixtures so action_id ↔ label stays 1:1 (VAL-V3-F6-027/029/030/032).
    let pass_origin = audit_screen(
        "reviewer-approved",
        "orchestration.reviewer-approved.score-open",
    )
    .expect("pass origin with score open");
    pass_origin
        .validate()
        .expect("pass origin audit must stay bijective");
    assert_unique_action_labels(&pass_origin);
    assert_action_painted(
        &pass_origin,
        "action.mission.view-score-detail",
        "View quality report",
    );
    assert_action_painted(
        &pass_origin,
        "action.mission.stay-on-review",
        "Stay on this review",
    );
    let pass_text = pass_origin.render_txt();
    assert!(
        !pass_text.contains("View quality report")
            || !pass_text
                .lines()
                .any(|line| line.contains("Stay on this review")
                    && line.contains("View quality report")),
        "score open must not share Stay label:\n{pass_text}"
    );

    let fail_origin = audit_screen(
        "reviewer-output",
        "orchestration.reviewer-output.score-open",
    )
    .expect("fail origin with score open");
    fail_origin
        .validate()
        .expect("fail origin audit must stay bijective");
    assert_unique_action_labels(&fail_origin);
    assert_action_painted(
        &fail_origin,
        "action.mission.view-validation-stages",
        "View validation summary",
    );
    assert_action_painted(
        &fail_origin,
        "action.mission.view-score-detail",
        "View quality report",
    );
    let fail_text = fail_origin.render_txt();
    assert!(
        fail_text.contains("View validation summary"),
        "validation summary key must remain:\n{fail_text}"
    );
    assert!(
        fail_text.contains("View quality report"),
        "score detail needs its own painted open:\n{fail_text}"
    );

    let runtime_origin = audit_screen(
        "workflow-monitor",
        "orchestration.workflow-monitor.runtime-open",
    )
    .expect("runtime origin");
    runtime_origin
        .validate()
        .expect("runtime origin audit must stay bijective");
    assert_unique_action_labels(&runtime_origin);
    assert_action_painted(
        &runtime_origin,
        "action.mission.view-runtime-detail",
        "View advanced detail",
    );
    assert_action_painted(
        &runtime_origin,
        "action.mission.view-progress",
        "View progress",
    );
    assert_action_painted(
        &runtime_origin,
        "action.mission.open-intervene",
        "Intervene",
    );
    let runtime_text = runtime_origin.render_txt();
    assert!(
        runtime_text.contains("View advanced detail"),
        "compact disclosure-safe monitor must paint advanced runtime open:\n{runtime_text}"
    );

    // Gate frames must not dual-bind residual score/runtime opens onto Stay or summary.
    for (screen, fixture) in [
        ("reviewer-approved", "gate.mission.reviewer-approved"),
        ("reviewer-output", "gate.mission.reviewer-output"),
        ("workflow-monitor", "gate.mission.workflow-monitor"),
    ] {
        let gate = audit_screen(screen, fixture).expect("gate frame renders");
        gate.validate().expect("gate audit bijective");
        let gate_actions = painted_actions(&gate);
        assert!(
            !gate_actions
                .iter()
                .any(|(id, label)| id == "action.mission.view-score-detail"
                    && label == "Stay on this review"),
            "{screen} must not dual-bind score onto Stay: {gate_actions:?}"
        );
        assert!(
            !gate_actions
                .iter()
                .any(|(id, label)| id == "action.mission.view-score-detail"
                    && label == "View validation summary"),
            "{screen} must not dual-bind score onto validation summary: {gate_actions:?}"
        );
        assert!(
            !gate_actions
                .iter()
                .any(|(id, label)| id == "action.mission.view-runtime-detail"
                    && (label == "View progress" || label == "Intervene")),
            "{screen} must not dual-bind runtime onto View progress/Intervene: {gate_actions:?}"
        );
        if screen == "reviewer-output" {
            // Gate may omit score open paint while keeping stages only.
            assert!(
                !gate_actions
                    .iter()
                    .any(|(id, _)| id == "action.mission.view-score-detail"),
                "gate reviewer-output should not invent dual score action: {gate_actions:?}"
            );
        }
        if screen == "reviewer-approved" {
            assert!(
                !gate_actions
                    .iter()
                    .any(|(id, _)| id == "action.mission.view-score-detail"),
                "gate reviewer-approved should not invent dual score action: {gate_actions:?}"
            );
        }
        if screen == "workflow-monitor" {
            assert!(
                !gate_actions
                    .iter()
                    .any(|(id, _)| id == "action.mission.view-runtime-detail"),
                "gate compact monitor omits advanced runtime paint: {gate_actions:?}"
            );
        }
    }
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
    assert!((text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL));
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
    assert!(text.contains("Return") || text.contains("Return to"));

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
    assert!((text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL));
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
    assert!(text.contains("Return") || text.contains("Return to mission"));

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
    assert!((text.contains("Mission") || text.contains(MISSION_ORIGIN)) || text.contains(MISSION_GOAL));
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
    assert!(text.contains("Return") || text.contains("Return to"));

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
