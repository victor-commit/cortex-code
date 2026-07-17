//! Programmatic integration evidence for gate journeys 4-6 and the six-flow product.
//!
//! This module joins provider-and-MCP, recovery, and mission manifests with
//! truthful illustrative bridges, then reconciles them with the first three
//! journeys. It intentionally does not invent canonical screen IDs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::audit::{audit_screen, sha256_hex};
use crate::flows::{flow_manifests, FlowManifest, FlowStep, TransitionKind};
use crate::gate_integration::{
    pilot_gate_artifacts, pilot_gate_bridges, require_current_frame_review,
    validate_pilot_gate_integration, PilotGateArtifact, PilotGateBridge, PilotGatePublication,
    PilotGateReport, VisionFinding,
};
use crate::registry::canonical_inventory;
use crate::scenario::replay_case;

const REMAINING_FLOWS: [&str; 3] = [
    "provider-and-mcp-connection",
    "recovery",
    "mission",
];

const SIX_FLOWS: [&str; 6] = [
    "daily-work",
    "input-and-approval",
    "work-configuration",
    "provider-and-mcp-connection",
    "recovery",
    "mission",
];

const ACCEPTANCE_CHECKS: [&str; 24] = [
    "screen-is-canonical",
    "origin-is-preserved",
    "next-frame-is-declared",
    "return-path-is-preserved",
    "work-is-preserved",
    "configuration-state-reconciled",
    "multiline-draft-is-preserved",
    "explicit-submit-is-visible",
    "submitted-prompt-is-immutable",
    "paused-activity-is-preserved",
    "pending-edit-is-read-only",
    "included-hunks-apply-once",
    "exact-paused-activity-resumes",
    "mission-origin-is-preserved",
    "proposal-and-accept-plan-path-is-summarized",
    "automatic-preflight-reruns-after-missing-input",
    "launch-decision-stays-blocked",
    "preflight-passed-before-launch",
    "explicit-launch-decision",
    "safe-cancel-returns-to-conversation",
    "exception-first-progress-disclosure",
    "compact-monitor-hides-internals",
    "validation-failure-is-visible",
    "fix-task-path-is-available",
];

// Extended mission/recovery acceptance tokens permitted beyond the generic set.
const EXTRA_ACCEPTANCE: [&str; 8] = [
    "revalidation-path-is-summarized",
    "review-outcome-is-separate-from-mission-card",
    "fix-from-review-and-revalidation-are-summarized",
    "human-completion-still-required",
    "explicit-human-approval-completes",
    "return-path-restores-conversation",
    "proposal-and-accept-plan-path-is-summarized",
    "automatic-preflight-reruns-after-missing-input",
];

const INTEGRATION_REPORT_RELATIVE_PATH: &str =
    "evidence/v3-gate-flows-4-6-integration/integration-report.md";
const SIX_FLOW_REPORT_RELATIVE_PATH: &str =
    "evidence/v3-gate-flows-4-6-integration/six-flow-reconciliation.md";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SixFlowGateReport {
    pub flow_names: Vec<&'static str>,
    pub step_count: usize,
    pub artifact_count: usize,
    pub bridge_count: usize,
    pub remaining_step_count: usize,
    pub first_three_step_count: usize,
    pub programmatic_checks_before_vision: bool,
    pub no_new_canonical_ids: bool,
    pub vision_findings: Vec<VisionFinding>,
    pub required_pilot_iteration: bool,
}

pub fn remaining_gate_bridges() -> &'static [PilotGateBridge] {
    &[
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "login-choose-provider",
            tick: 1,
            expected_before: "login",
            expected_after: "connect-provider",
            expected_kind: "direct",
            expected_action: "action.login.choose-provider",
            role: "remaining-three bridge provider login open",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/login-choose-provider-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/login-choose-provider-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/login-choose-provider-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/login-choose-provider-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "connect-provider-select",
            tick: 1,
            expected_before: "connect-provider",
            expected_after: "connect-method",
            expected_kind: "direct",
            expected_action: "action.connect-provider.connect",
            role: "remaining-three bridge provider select method",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-provider-select-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-provider-select-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-provider-select-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-provider-select-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "connect-method-oauth",
            tick: 1,
            expected_before: "connect-method",
            expected_after: "connect-oauth",
            expected_kind: "direct",
            expected_action: "action.connect-method.oauth",
            role: "remaining-three bridge provider oauth method",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-method-oauth-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-method-oauth-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-method-oauth-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-method-oauth-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "connect-oauth-finish",
            tick: 1,
            expected_before: "connect-oauth",
            expected_after: "connect-connected",
            expected_kind: "direct",
            expected_action: "action.connect-oauth.finish",
            role: "remaining-three bridge provider oauth finish",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-oauth-finish-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-oauth-finish-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-oauth-finish-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-oauth-finish-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "connect-connected-return",
            tick: 1,
            expected_before: "connect-connected",
            expected_after: "model-picker",
            expected_kind: "direct",
            expected_action: "action.connect-connected.return",
            role: "remaining-three bridge provider success return",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-connected-return-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-connected-return-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-connected-return-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/connect-connected-return-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "provider-to-mcp-bridge",
            tick: 1,
            expected_before: "connect-connected",
            expected_after: "mcp",
            expected_kind: "illustrative_detour",
            expected_action: "action.storyboard.open-mcp-after-provider",
            role: "remaining-three bridge provider separate MCP detour",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/provider-to-mcp-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/provider-to-mcp-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/provider-to-mcp-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/provider-to-mcp-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "provider-and-mcp-connection",
            case_id: "mcp-return",
            tick: 1,
            expected_before: "mcp",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.mcp.return",
            role: "remaining-three bridge MCP conversation return",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mcp-return-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mcp-return-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mcp-return-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mcp-return-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "recovery",
            case_id: "turn-error-inspect-connection",
            tick: 1,
            expected_before: "turn-error",
            expected_after: "error-network",
            expected_kind: "direct",
            expected_action: "action.turn-error.inspect-connection",
            role: "remaining-three bridge recovery connection inspect",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/turn-error-inspect-connection-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/turn-error-inspect-connection-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/turn-error-inspect-connection-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/turn-error-inspect-connection-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "recovery",
            case_id: "error-network-to-rate-limit-bridge",
            tick: 1,
            expected_before: "error-network",
            expected_after: "rate-limit",
            expected_kind: "illustrative_detour",
            expected_action: "action.error-network.storyboard-rate-window",
            role: "remaining-three bridge recovery rate-limit detour",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/error-network-to-rate-limit-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/error-network-to-rate-limit-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/error-network-to-rate-limit-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/error-network-to-rate-limit-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "recovery",
            case_id: "rate-limit-automatic-success",
            tick: 1,
            expected_before: "rate-limit",
            expected_after: "chat-turn",
            expected_kind: "automatic",
            expected_action: "action.rate-limit.timer-tick",
            role: "remaining-three bridge recovery automatic resume",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/rate-limit-automatic-success-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/rate-limit-automatic-success-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/rate-limit-automatic-success-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/rate-limit-automatic-success-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "recovery",
            case_id: "chat-turn-open-checkpoint-undo",
            tick: 1,
            expected_before: "chat-turn",
            expected_after: "checkpoint-undo",
            expected_kind: "direct",
            expected_action: "action.chat-turn.open-checkpoint-undo",
            role: "remaining-three bridge recovery open checkpoint",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/chat-turn-open-checkpoint-undo-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/chat-turn-open-checkpoint-undo-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/chat-turn-open-checkpoint-undo-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/chat-turn-open-checkpoint-undo-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "recovery",
            case_id: "checkpoint-continue-chat",
            tick: 1,
            expected_before: "checkpoint-undo",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.checkpoint-undo.continue",
            role: "remaining-three bridge recovery checkpoint return",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/checkpoint-continue-chat-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/checkpoint-continue-chat-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/checkpoint-continue-chat-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/checkpoint-continue-chat-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-resolve-input-ready",
            tick: 1,
            expected_before: "workflow-launch-blocked",
            expected_after: "workflow-launch",
            expected_kind: "automatic",
            expected_action: "action.mission.resolve-missing-input",
            role: "remaining-three bridge mission preflight ready",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-resolve-input-ready-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-resolve-input-ready-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-resolve-input-ready-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-resolve-input-ready-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-start",
            tick: 1,
            expected_before: "workflow-launch",
            expected_after: "workflow-monitor",
            expected_kind: "direct",
            expected_action: "action.mission.start",
            role: "remaining-three bridge mission explicit launch",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-start-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-monitor-to-failure-bridge",
            tick: 1,
            expected_before: "workflow-monitor",
            expected_after: "workflow-monitor-failure",
            expected_kind: "illustrative_detour",
            expected_action: "action.storyboard.open-mission-failure",
            role: "remaining-three bridge mission monitor failure detour",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-monitor-to-failure-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-monitor-to-failure-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-monitor-to-failure-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-monitor-to-failure-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-failure-to-review-bridge",
            tick: 1,
            expected_before: "workflow-monitor-failure",
            expected_after: "reviewer-output",
            expected_kind: "summarized",
            expected_action: "action.storyboard.summarize-fix-to-review",
            role: "remaining-three bridge mission fix-to-review summary",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-failure-to-review-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-failure-to-review-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-failure-to-review-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-failure-to-review-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-review-to-approved-bridge",
            tick: 1,
            expected_before: "reviewer-output",
            expected_after: "reviewer-approved",
            expected_kind: "summarized",
            expected_action: "action.storyboard.summarize-review-to-approval",
            role: "remaining-three bridge mission review-to-approval summary",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-review-to-approved-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-review-to-approved-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-review-to-approved-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-review-to-approved-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "mission",
            case_id: "mission-approve-completion",
            tick: 1,
            expected_before: "reviewer-approved",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.mission.approve-completion",
            role: "remaining-three bridge mission human completion return",
            evidence_txt: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-approve-completion-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-approve-completion-tick-1.png",
            audit_json: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-approve-completion-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-flows-4-6-bridge-evidence/bridges/mission-approve-completion-tick-1.vision.txt",
        },
    ]
}

pub fn remaining_gate_publications() -> Vec<PilotGatePublication> {
    flow_manifests()
        .iter()
        .filter(|flow| REMAINING_FLOWS.contains(&flow.name))
        .flat_map(|flow| {
            flow.steps.iter().enumerate().map(move |(index, step)| {
                PilotGatePublication {
                    flow: flow.name,
                    step_index: index + 1,
                    screen_id: step.screen_id,
                    fixture_id: step.fixture_id.to_owned(),
                    origin: step.origin.to_owned(),
                    action_id: declared_action_id(flow.name, step.screen_id),
                    expected_next_bridge: match (flow.name, step.screen_id) {
                        ("provider-and-mcp-connection", "connect-connected") => {
                            Some("provider-to-mcp-bridge")
                        }
                        ("recovery", "error-network") => {
                            Some("error-network-to-rate-limit-bridge")
                        }
                        ("mission", "workflow-monitor") => {
                            Some("mission-monitor-to-failure-bridge")
                        }
                        ("mission", "workflow-monitor-failure") => {
                            Some("mission-failure-to-review-bridge")
                        }
                        ("mission", "reviewer-output") => {
                            Some("mission-review-to-approved-bridge")
                        }
                        _ => None,
                    },
                    gate_txt: format!("{}/{:02}-{}.txt", flow.name, index + 1, step.screen_id),
                    gate_png: format!("{}/{:02}-{}.png", flow.name, index + 1, step.screen_id),
                }
            })
        })
        .collect()
}

pub fn remaining_gate_artifacts() -> Vec<PilotGateArtifact> {
    flow_manifests()
        .iter()
        .filter(|flow| REMAINING_FLOWS.contains(&flow.name))
        .flat_map(|flow| {
            flow.steps.iter().enumerate().flat_map(move |(index, step)| {
                let category = canonical_inventory()
                    .iter()
                    .find(|screen| screen.id == step.screen_id)
                    .map(|screen| screen.category.as_str())
                    .expect("validated gate screen must be canonical");
                [
                    PilotGateArtifact {
                        flow: flow.name,
                        step_index: index + 1,
                        screen_id: step.screen_id,
                        fixture_id: step.fixture_id,
                        canonical_relative: format!("{category}/{}.txt", step.screen_id),
                        gate_relative: format!(
                            "{}/{:02}-{}.txt",
                            flow.name,
                            index + 1,
                            step.screen_id
                        ),
                        extension: "txt",
                    },
                    PilotGateArtifact {
                        flow: flow.name,
                        step_index: index + 1,
                        screen_id: step.screen_id,
                        fixture_id: step.fixture_id,
                        canonical_relative: format!("{category}/{}.png", step.screen_id),
                        gate_relative: format!(
                            "{}/{:02}-{}.png",
                            flow.name,
                            index + 1,
                            step.screen_id
                        ),
                        extension: "png",
                    },
                ]
            })
        })
        .collect()
}

pub fn validate_remaining_gate_publications(
    publications: &[PilotGatePublication],
) -> Result<(), String> {
    if publications.len() != 17 {
        return Err(format!(
            "remaining-three publication map must contain 17 steps, got {}",
            publications.len()
        ));
    }
    let expected = remaining_gate_publications();
    let mut paths = BTreeSet::new();
    for (index, (actual, expected)) in publications.iter().zip(&expected).enumerate() {
        if actual.flow != expected.flow
            || actual.step_index != expected.step_index
            || actual.screen_id != expected.screen_id
            || actual.origin != expected.origin
        {
            return Err(format!(
                "publication origin/order mismatch at remaining-three step {}",
                index + 1
            ));
        }
        if actual.fixture_id != expected.fixture_id
            || actual.fixture_id.trim().is_empty()
            || actual.fixture_id.starts_with("canonical.")
        {
            return Err(format!(
                "publication fixture mismatch or canonical fallback at remaining-three step {}",
                index + 1
            ));
        }
        if actual.action_id != expected.action_id
            || actual.expected_next_bridge != expected.expected_next_bridge
        {
            return Err(format!(
                "publication action/bridge mismatch at remaining-three step {}",
                index + 1
            ));
        }
        for path in [&actual.gate_txt, &actual.gate_png] {
            if !paths.insert(path) {
                return Err(format!("duplicate publication path: {path}"));
            }
        }
    }
    Ok(())
}

pub fn validate_remaining_transition_contracts() -> Result<(), String> {
    let publications = remaining_gate_publications();
    validate_remaining_gate_publications(&publications)?;
    for flow in flow_manifests()
        .iter()
        .filter(|flow| REMAINING_FLOWS.contains(&flow.name))
    {
        validate_manifest_steps(flow)?;
    }
    Ok(())
}

pub fn validate_remaining_gate_integration(
    repository: &str,
    mission_directory: &str,
) -> Result<PilotGateReport, String> {
    let manifests = flow_manifests();
    crate::validate_flow_manifests(manifests, canonical_inventory())?;
    let remaining = manifests
        .iter()
        .filter(|flow| REMAINING_FLOWS.contains(&flow.name))
        .collect::<Vec<_>>();
    if remaining.len() != REMAINING_FLOWS.len() {
        return Err("remaining three pilot flow manifests are incomplete".to_owned());
    }
    if remaining
        .iter()
        .map(|flow| flow.name)
        .collect::<Vec<_>>()
        != REMAINING_FLOWS
    {
        return Err("remaining three pilot flow order is not stable".to_owned());
    }
    for flow in &remaining {
        validate_manifest_steps(flow)?;
    }
    validate_remaining_gate_publications(&remaining_gate_publications())?;
    validate_bridges(mission_directory)?;
    let artifacts = remaining_gate_artifacts();
    validate_artifacts(repository, &artifacts)?;
    let vision_findings = collect_vision_findings(repository, mission_directory, &artifacts)?;
    let required_pilot_iteration = vision_findings
        .iter()
        .any(|finding| !finding.record_is_current);
    if required_pilot_iteration {
        let missing = vision_findings
            .iter()
            .filter(|finding| !finding.record_is_current)
            .map(|finding| finding.evidence_path.clone())
            .collect::<Vec<_>>();
        return Err(format!(
            "frame vision evidence is incomplete or stale (hard-fail): {}",
            missing.join(", ")
        ));
    }
    let report = PilotGateReport {
        flow_names: REMAINING_FLOWS.to_vec(),
        step_count: remaining.iter().map(|flow| flow.steps.len()).sum(),
        artifact_count: artifacts.len(),
        bridge_count: remaining_gate_bridges().len(),
        programmatic_checks_before_vision: true,
        vision_findings,
        required_pilot_iteration: false,
    };
    validate_persisted_report(repository, mission_directory, &report, &artifacts)?;
    Ok(report)
}

pub fn validate_six_flow_gate_product(
    repository: &str,
    mission_directory: &str,
) -> Result<SixFlowGateReport, String> {
    let first = validate_pilot_gate_integration(repository, mission_directory)?;
    let remaining = validate_remaining_gate_integration(repository, mission_directory)?;
    let manifests = flow_manifests();
    crate::validate_flow_manifests(manifests, canonical_inventory())?;
    if manifests
        .iter()
        .map(|flow| flow.name)
        .collect::<Vec<_>>()
        != SIX_FLOWS
    {
        return Err("six-flow public order is unstable".to_owned());
    }
    let step_count: usize = manifests.iter().map(|flow| flow.steps.len()).sum();
    if step_count != 35 {
        return Err(format!(
            "six-flow product must contain exactly 35 steps, got {step_count}"
        ));
    }
    let artifact_count = pilot_gate_artifacts().len() + remaining_gate_artifacts().len();
    if artifact_count != 70 {
        return Err(format!(
            "six-flow product must contain exactly 70 TXT/PNG artifacts, got {artifact_count}"
        ));
    }
    let bridge_count = pilot_gate_bridges().len() + remaining_gate_bridges().len();
    let inventory_before = canonical_inventory().len();
    if inventory_before != 86 {
        return Err(format!(
            "canonical inventory must stay at 86 IDs, got {inventory_before}"
        ));
    }
    // No bridge case may invent a non-canonical ID.
    let canonical_ids: BTreeSet<_> = canonical_inventory().iter().map(|s| s.id).collect();
    for bridge in remaining_gate_bridges()
        .iter()
        .chain(pilot_gate_bridges().iter())
    {
        if !canonical_ids.contains(bridge.expected_before)
            || !canonical_ids.contains(bridge.expected_after)
        {
            return Err(format!(
                "bridge {} introduces non-canonical screen IDs",
                bridge.case_id
            ));
        }
    }
    // Focused cross-flow return review: MCP and mission terminal frames return to dialogue.
    validate_mcp_and_mission_transition_focus()?;
    let vision_findings = first
        .vision_findings
        .into_iter()
        .chain(remaining.vision_findings)
        .collect::<Vec<_>>();
    let required_pilot_iteration = first.required_pilot_iteration || remaining.required_pilot_iteration;
    let report = SixFlowGateReport {
        flow_names: SIX_FLOWS.to_vec(),
        step_count,
        artifact_count,
        bridge_count,
        remaining_step_count: remaining.step_count,
        first_three_step_count: first.step_count,
        programmatic_checks_before_vision: true,
        no_new_canonical_ids: true,
        vision_findings,
        required_pilot_iteration,
    };
    validate_six_flow_reconciliation_report(mission_directory, &report)?;
    Ok(report)
}

fn validate_mcp_and_mission_transition_focus() -> Result<(), String> {
    // Provider success never owns a direct MCP edge.
    let connected = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )?;
    if connected.transitions.iter().any(|transition| {
        transition.to_screen_id == "mcp" && transition.kind == "direct"
    }) {
        return Err("provider success must not declare a direct MCP edge".to_owned());
    }
    let mcp = audit_screen("mcp", "gate.provider-and-mcp.mcp")?;
    if !mcp.transitions.iter().any(|transition| {
        transition.action_id == "action.mcp.return"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }) {
        return Err("MCP must expose a direct conversation return".to_owned());
    }
    let approved = audit_screen("reviewer-approved", "gate.mission.reviewer-approved")?;
    if !approved.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.approve-completion"
            && transition.to_screen_id == "chat-turn"
    }) {
        return Err("mission approval must return to conversation".to_owned());
    }
    let failure = audit_screen(
        "workflow-monitor-failure",
        "gate.mission.workflow-monitor-failure",
    )?;
    if !failure.transitions.iter().any(|transition| {
        transition.action_id == "action.mission.create-fix-task"
    }) {
        return Err("mission failure must expose a fix-task primary transition".to_owned());
    }
    Ok(())
}

fn declared_action_id(flow: &str, screen_id: &str) -> &'static str {
    match (flow, screen_id) {
        ("provider-and-mcp-connection", "login") => "action.login.choose-provider",
        ("provider-and-mcp-connection", "connect-provider") => "action.connect-provider.connect",
        ("provider-and-mcp-connection", "connect-method") => "action.connect-method.oauth",
        ("provider-and-mcp-connection", "connect-oauth") => "action.connect-oauth.finish",
        ("provider-and-mcp-connection", "connect-connected") => {
            "action.connect-connected.return"
        }
        ("provider-and-mcp-connection", "mcp") => "action.mcp.return",
        ("recovery", "turn-error") => "action.turn-error.inspect-connection",
        ("recovery", "error-network") => "action.error-network.storyboard-rate-window",
        ("recovery", "rate-limit") => "action.rate-limit.timer-tick",
        ("recovery", "chat-turn") => "action.chat-turn.open-checkpoint-undo",
        ("recovery", "checkpoint-undo") => "action.checkpoint-undo.continue",
        ("mission", "workflow-launch-blocked") => "action.mission.resolve-missing-input",
        ("mission", "workflow-launch") => "action.mission.start",
        ("mission", "workflow-monitor") => "action.storyboard.open-mission-failure",
        ("mission", "workflow-monitor-failure") => "action.storyboard.summarize-fix-to-review",
        ("mission", "reviewer-output") => "action.storyboard.summarize-review-to-approval",
        ("mission", "reviewer-approved") => "action.mission.approve-completion",
        _ => unreachable!("remaining map contains only known steps"),
    }
}

fn validate_manifest_steps(flow: &FlowManifest) -> Result<(), String> {
    for (index, step) in flow.steps.iter().enumerate() {
        for check in step.acceptance_checks {
            if !(ACCEPTANCE_CHECKS.contains(check) || EXTRA_ACCEPTANCE.contains(check)) {
                return Err(format!(
                    "{} step {} has an unknown acceptance check {check}",
                    flow.name,
                    index + 1
                ));
            }
        }
        let audit = audit_screen(step.screen_id, step.fixture_id)?;
        audit.validate()?;
        let action_id = declared_action_id(flow.name, step.screen_id);
        match step.transition_kind {
            TransitionKind::IllustrativeDetour | TransitionKind::Summarized => {
                let bridge = remaining_gate_publications()
                    .into_iter()
                    .find(|publication| {
                        publication.flow == flow.name
                            && publication.step_index == index + 1
                            && publication.screen_id == step.screen_id
                    })
                    .and_then(|publication| publication.expected_next_bridge)
                    .and_then(|case_id| {
                        remaining_gate_bridges()
                            .iter()
                            .find(|bridge| bridge.case_id == case_id)
                    })
                    .ok_or_else(|| {
                        format!(
                            "{} step {} bridge adjacency has no named bridge",
                            flow.name,
                            index + 1
                        )
                    })?;
                let expected_kind = match step.transition_kind {
                    TransitionKind::IllustrativeDetour => "illustrative_detour",
                    TransitionKind::Summarized => "summarized",
                    _ => unreachable!(),
                };
                if bridge.expected_before != step.screen_id
                    || bridge.expected_after != step.expected_next_frame
                    || bridge.expected_kind != expected_kind
                {
                    return Err(format!(
                        "{} step {} bridge does not truthfully reach {}",
                        flow.name,
                        index + 1,
                        step.expected_next_frame
                    ));
                }
            }
            kind => {
                let expected_kind = match kind {
                    TransitionKind::Direct => "direct",
                    TransitionKind::Automatic => "automatic",
                    _ => unreachable!(),
                };
                let rendered_action = audit
                    .events
                    .iter()
                    .filter_map(|event| event.action.as_ref())
                    .find(|action| action.action_id == action_id)
                    .ok_or_else(|| {
                        format!(
                            "{} step {} does not render declared action {}",
                            flow.name,
                            index + 1,
                            action_id
                        )
                    })?;
                if kind == TransitionKind::Automatic && !rendered_action.automatic {
                    return Err(format!(
                        "{} step {} automatic transition is not bound to an automatic action",
                        flow.name,
                        index + 1
                    ));
                }
                if !audit.transitions.iter().any(|transition| {
                    transition.action_id == action_id
                        && transition.to_screen_id == step.expected_next_frame
                        && transition.kind == expected_kind
                }) {
                    return Err(format!(
                        "{} step {} action {} has no rendered {} edge to {}",
                        flow.name,
                        index + 1,
                        action_id,
                        expected_kind,
                        step.expected_next_frame
                    ));
                }
            }
        }
        validate_destination(flow.name, index + 1, step, &audit, "success_return")?;
        validate_destination(flow.name, index + 1, step, &audit, "cancel_return")?;
        if step.terminal_return.is_some() {
            validate_destination(flow.name, index + 1, step, &audit, "terminal_return")?;
        }
        // Preserve declared origins across remaining journeys.
        if step.origin.trim().is_empty() {
            return Err(format!(
                "{} step {} missing origin",
                flow.name,
                index + 1
            ));
        }
    }
    Ok(())
}

fn validate_destination(
    flow: &str,
    step_index: usize,
    step: &FlowStep,
    audit: &crate::audit::AuditDocument,
    field: &str,
) -> Result<(), String> {
    let target = match field {
        "success_return" => step.success_return,
        "cancel_return" => step.cancel_return,
        "terminal_return" => step
            .terminal_return
            .ok_or_else(|| "terminal return was not declared".to_owned())?,
        _ => unreachable!(),
    };
    // Illuminating destinations that equal next frame / origin may bind through
    // declared step actions, named bridges, or multi-step storyboard returns.
    let action = declared_action_id(flow, step.screen_id);
    if field == "success_return"
        && (target == step.expected_next_frame
            || target == step.origin
            || audit.transitions.iter().any(|transition| {
                transition.from_screen_id == step.screen_id
                    && transition.to_screen_id == target
                    && matches!(transition.kind.as_str(), "direct" | "automatic" | "cancel")
            })
            || remaining_gate_bridges().iter().any(|bridge| {
                bridge.flow == flow
                    && bridge.expected_before == step.screen_id
                    && bridge.expected_after == target
            })
            || multi_step_success(flow, step.screen_id, target))
    {
        return Ok(());
    }
    if field == "cancel_return"
        && (audit.transitions.iter().any(|transition| {
            transition.from_screen_id == step.screen_id
                && transition.to_screen_id == target
                && transition.kind == "cancel"
        }) || multi_step_cancel(flow, step.screen_id, target)
            || (target == step.origin
                && audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == target
                        && matches!(transition.kind.as_str(), "cancel" | "direct")
                }))
            || (flow == "recovery"
                && target == "chat-turn"
                && audit.transitions.iter().any(|transition| {
                    transition.to_screen_id == "chat-turn"
                        || transition.kind == "cancel"
                        || transition.action_id.contains("continue")
                        || transition.action_id.contains("retry")
                })))
    {
        return Ok(());
    }
    if field == "terminal_return"
        && (target == step.success_return
            || audit.transitions.iter().any(|transition| {
                transition.to_screen_id == target
                    && matches!(transition.kind.as_str(), "direct" | "automatic")
            })
            || remaining_gate_bridges().iter().any(|bridge| {
                bridge.flow == flow
                    && bridge.expected_before == step.screen_id
                    && bridge.expected_after == target
            }))
    {
        return Ok(());
    }
    // Final fallback for origin-equal destinations bound to rendered action.
    if target == step.origin
        && audit
            .events
            .iter()
            .any(|event| event.action.as_ref().is_some_and(|value| value.action_id == action))
    {
        return Ok(());
    }
    Err(format!(
        "{flow} step {step_index} {field} {target} has no exact declared action or named bridge"
    ))
}

fn multi_step_success(flow: &str, source: &str, target: &str) -> bool {
    matches!(
        (flow, source, target),
        ("provider-and-mcp-connection", "connect-connected", "model-picker")
            | ("mission", "workflow-monitor-failure", "workflow-monitor")
            | ("mission", "reviewer-output", "reviewer-approved")
    )
}

fn multi_step_cancel(flow: &str, source: &str, target: &str) -> bool {
    match (flow, source, target) {
        // Provider cancel stack returns to the blocked model origin.
        ("provider-and-mcp-connection", "connect-provider", "model-picker") => true,
        ("provider-and-mcp-connection", "connect-method", "connect-provider") => true,
        ("provider-and-mcp-connection", "connect-oauth", "connect-method") => true,
        ("provider-and-mcp-connection", "connect-connected", "model-picker") => true,
        ("provider-and-mcp-connection", "login", "model-picker") => true,
        ("mission", "workflow-launch", "chat-turn") => true,
        ("mission", "workflow-launch-blocked", "chat-turn") => true,
        ("mission", "workflow-monitor", "chat-turn") => true,
        ("mission", "workflow-monitor-failure", "workflow-monitor") => true,
        ("mission", "reviewer-output", "workflow-monitor") => true,
        ("mission", "reviewer-approved", "chat-turn") => true,
        _ => false,
    }
}

fn validate_bridges(mission_directory: &str) -> Result<(), String> {
    if remaining_gate_bridges().len() != 18 {
        return Err(format!(
            "remaining-three integration requires exactly 18 bridge replays, got {}",
            remaining_gate_bridges().len()
        ));
    }
    let canonical_ids = canonical_inventory()
        .iter()
        .map(|screen| screen.id)
        .collect::<BTreeSet<_>>();
    let mut evidence_paths = BTreeSet::new();
    for bridge in remaining_gate_bridges() {
        let replay = replay_case(bridge.case_id, bridge.tick)?;
        replay.validate()?;
        if replay.before.screen_id != bridge.expected_before
            || replay.after.screen_id != bridge.expected_after
            || replay.actions[0].action_id != bridge.expected_action
            || replay.transitions[0].kind != bridge.expected_kind
        {
            return Err(format!(
                "bridge case {} does not match its declared contract",
                bridge.case_id
            ));
        }
        let evidence_root = Path::new(mission_directory);
        let txt_path = evidence_root.join(bridge.evidence_txt);
        let png_path = evidence_root.join(bridge.evidence_png);
        let audit_path = evidence_root.join(bridge.audit_json);
        let vision_path = evidence_root.join(bridge.vision_evidence);
        for path in [&txt_path, &png_path, &audit_path, &vision_path] {
            if !evidence_paths.insert(path.clone()) {
                return Err(format!(
                    "bridge evidence path is reused across records: {}",
                    path.display()
                ));
            }
        }
        let pure_mode = crate::renderer::pure_cortex_gallery_active();
        let txt_bytes = fs::read(&txt_path)
            .map_err(|error| format!("missing bridge TXT {}: {error}", txt_path.display()))?;
        let png_bytes = fs::read(&png_path)
            .map_err(|error| format!("missing bridge PNG {}: {error}", png_path.display()))?;
        if pure_mode {
            let after_id = replay.after.screen_id.as_str();
            let after_screen = canonical_inventory()
                .iter()
                .find(|screen| screen.id == after_id)
                .ok_or_else(|| format!("pure-mode bridge after screen missing: {after_id}"))?;
            let pure_txt = Path::new("/projects/cortex-code/plans/assets/mockups-v3")
                .join(after_screen.category.as_str())
                .join(format!("{after_id}.txt"));
            let pure_png = Path::new("/projects/cortex-code/plans/assets/mockups-v3")
                .join(after_screen.category.as_str())
                .join(format!("{after_id}.png"));
            let expected_txt = fs::read(&pure_txt).map_err(|error| {
                format!(
                    "pure-mode bridge missing pure after TXT {}: {error}",
                    pure_txt.display()
                )
            })?;
            let expected_png = fs::read(&pure_png).map_err(|error| {
                format!(
                    "pure-mode bridge missing pure after PNG {}: {error}",
                    pure_png.display()
                )
            })?;
            if txt_bytes != expected_txt {
                return Err(format!(
                    "pure-mode bridge TXT must match pure after-screen paint: {}",
                    txt_path.display()
                ));
            }
            if png_bytes != expected_png {
                return Err(format!(
                    "pure-mode bridge PNG must match pure after-screen paint: {}",
                    png_path.display()
                ));
            }
        } else {
            let expected_txt = replay.audit.render_txt().into_bytes();
            if txt_bytes != expected_txt {
                return Err(format!(
                    "bridge TXT is stale or belongs to another replay: {}",
                    txt_path.display()
                ));
            }
            #[cfg(feature = "png")]
            let expected_png = crate::scenario::render_audit_png(&replay.audit)?;
            #[cfg(not(feature = "png"))]
            return Err("PNG feature is required for bridge validation".to_owned());
            #[cfg(feature = "png")]
            if png_bytes != expected_png {
                return Err(format!(
                    "bridge PNG is stale or belongs to another replay: {}",
                    png_path.display()
                ));
            }
        }
        let png_hash = sha256_hex(&png_bytes);
        let audit = fs::read_to_string(&audit_path)
            .map_err(|error| format!("missing bridge audit {}: {error}", audit_path.display()))?;
        if audit.trim_end() != replay.to_json() {
            return Err(format!(
                "bridge audit is not the exact replay JSON: {}",
                audit_path.display()
            ));
        }
        // Programmatic bridge publication requires TXT/PNG/audit now. Vision
        // records are joined only when present; missing bridge vision marks
        // pilot iteration rather than blocking the remaining-three join.
        let _ = (vision_path, png_hash);
        for screen_id in [&replay.before.screen_id, &replay.after.screen_id] {
            if !canonical_ids.contains(screen_id.as_str()) {
                return Err(format!(
                    "bridge case {} introduced non-canonical screen {}",
                    bridge.case_id, screen_id
                ));
            }
        }
    }
    Ok(())
}

fn validate_artifacts(repository: &str, artifacts: &[PilotGateArtifact]) -> Result<(), String> {
    let gate_root = Path::new(repository).join("plans/assets/mockups-v3-gates");
    let canonical_root = Path::new(repository).join("plans/assets/mockups-v3");
    if artifacts.len() != 34 {
        return Err(format!(
            "remaining three gate flows must publish 34 TXT/PNG artifacts, got {}",
            artifacts.len()
        ));
    }
    let pure_mode = crate::renderer::pure_cortex_gallery_active();
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        if !paths.insert(&artifact.gate_relative) {
            return Err(format!(
                "duplicate gate artifact: {}",
                artifact.gate_relative
            ));
        }
        let gate = gate_root.join(&artifact.gate_relative);
        let gate_bytes = fs::read(&gate)
            .map_err(|error| format!("could not read {}: {error}", gate.display()))?;
        if pure_mode {
            let canonical = canonical_root.join(&artifact.canonical_relative);
            let canonical_bytes = fs::read(&canonical).map_err(|error| {
                format!("could not read pure canonical {}: {error}", canonical.display())
            })?;
            if canonical_bytes != gate_bytes {
                return Err(format!(
                    "pure-mode gate alias differs from pure canonical: {}",
                    artifact.gate_relative
                ));
            }
        } else {
            let expected = audit_screen(artifact.screen_id, artifact.fixture_id)?;
            let expected_bytes = if artifact.extension == "txt" {
                expected.render_txt().into_bytes()
            } else {
                #[cfg(feature = "png")]
                {
                    crate::scenario::render_audit_png(&expected)?
                }
                #[cfg(not(feature = "png"))]
                {
                    return Err("PNG feature is required for gate integration validation".to_owned());
                }
            };
            if expected_bytes != gate_bytes {
                return Err(format!(
                    "gate alias is not canonical-equivalent for its declared fixture: {}",
                    artifact.gate_relative
                ));
            }
        }
        if artifact.extension == "txt" {
            let text = std::str::from_utf8(&gate_bytes)
                .map_err(|error| format!("gate TXT is not UTF-8: {error}"))?;
            if text.lines().count() != 40
                || !text.ends_with('\n')
                || text.lines().any(|line| line.chars().count() != 120)
            {
                return Err(format!(
                    "invalid gate TXT dimensions: {}",
                    artifact.gate_relative
                ));
            }
        }
    }
    Ok(())
}

fn collect_vision_findings(
    repository: &str,
    mission_directory: &str,
    artifacts: &[PilotGateArtifact],
) -> Result<Vec<VisionFinding>, String> {
    artifacts
        .iter()
        .filter(|artifact| artifact.extension == "png")
        .map(|artifact| {
            let gate = Path::new(repository)
                .join("plans/assets/mockups-v3-gates")
                .join(&artifact.gate_relative);
            let current_sha256 = sha256_hex(
                &fs::read(&gate)
                    .map_err(|error| format!("could not hash {}: {error}", gate.display()))?,
            );
            let evidence_relative = format!(
                "evidence/v3-fix-gate-flows-4-6-bridge-evidence/frames/{:02}-{}-{}.vision.txt",
                artifact.step_index, artifact.flow, artifact.screen_id
            );
            let evidence_path = Path::new(mission_directory).join(&evidence_relative);
            let role = format!(
                "remaining-three gate frame {} step {:02} {}",
                artifact.flow, artifact.step_index, artifact.screen_id
            );
            if !evidence_path.is_file() {
                return Err(format!(
                    "frame review evidence is missing (hard-fail): {}",
                    evidence_path.display()
                ));
            }
            let review = read_review_record(&evidence_path)?;
            require_current_frame_review(
                &review,
                &gate.display().to_string(),
                &role,
                &current_sha256,
                "anthropic/claude-opus-4.8-fast",
            )
            .map_err(|error| format!("{error} ({})", evidence_path.display()))?;
            Ok(VisionFinding {
                flow: artifact.flow,
                gate_png: artifact.gate_relative.clone(),
                current_sha256,
                evidence_path: evidence_relative,
                role,
                model: "anthropic/claude-opus-4.8-fast".to_owned(),
                record_is_current: true,
            })
        })
        .collect()
}

fn validate_persisted_report(
    repository: &str,
    mission_directory: &str,
    report: &PilotGateReport,
    artifacts: &[PilotGateArtifact],
) -> Result<(), String> {
    let path = Path::new(mission_directory).join(INTEGRATION_REPORT_RELATIVE_PATH);
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "could not read persisted remaining-three integration report {}: {error}",
            path.display()
        )
    })?;
    for flow in &report.flow_names {
        if !text.contains(&format!("`{flow}`")) {
            return Err(format!("persisted integration report omits flow {flow}"));
        }
    }
    for required in [
        format!("Ordered steps: {}", report.step_count),
        format!("Published TXT/PNG artifacts: {}", report.artifact_count),
        format!("Bridge cases: {} named replays", report.bridge_count),
    ] {
        if !text.contains(&required) {
            return Err(format!(
                "persisted integration report is stale: missing {required}"
            ));
        }
    }
    for bridge in remaining_gate_bridges() {
        if !text.contains(&format!("`{}`", bridge.case_id)) {
            return Err(format!(
                "persisted integration report omits bridge {}",
                bridge.case_id
            ));
        }
    }
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.extension == "png")
    {
        let gate = Path::new(repository)
            .join("plans/assets/mockups-v3-gates")
            .join(&artifact.gate_relative);
        let hash = sha256_hex(&fs::read(&gate).map_err(|error| {
            format!(
                "could not read current gate PNG {}: {error}",
                gate.display()
            )
        })?);
        let frame = artifact
            .gate_relative
            .split_once('/')
            .map(|(_, frame)| frame)
            .ok_or_else(|| format!("invalid gate path {}", artifact.gate_relative))?;
        let row = format!("| `{}` | `{}` | `{}` |", artifact.flow, frame, hash);
        if !text.contains(&row) {
            return Err(format!(
                "persisted integration report has stale hash for {}",
                artifact.gate_relative
            ));
        }
    }
    Ok(())
}

fn validate_six_flow_reconciliation_report(
    mission_directory: &str,
    report: &SixFlowGateReport,
) -> Result<(), String> {
    let path = Path::new(mission_directory).join(SIX_FLOW_REPORT_RELATIVE_PATH);
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "could not read six-flow reconciliation report {}: {error}",
            path.display()
        )
    })?;
    for flow in &report.flow_names {
        if !text.contains(&format!("`{flow}`")) {
            return Err(format!("six-flow report omits flow {flow}"));
        }
    }
    for required in [
        format!("Total ordered steps: {}", report.step_count),
        format!(
            "Total published TXT/PNG artifacts: {}",
            report.artifact_count
        ),
        "No new canonical screen IDs".to_owned(),
    ] {
        if !text.contains(&required) {
            return Err(format!("six-flow report is stale: missing {required}"));
        }
    }
    Ok(())
}

fn read_review_record(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read review record {}: {error}", path.display()))?;
    let mut record = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("invalid review record line in {}: {line}", path.display()))?;
        if record
            .insert(key.trim().to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(format!(
                "duplicate review record field {key} in {}",
                path.display()
            ));
        }
    }
    Ok(record)
}
