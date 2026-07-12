//! Programmatic integration evidence for the first three human-gate journeys.
//!
//! This module joins existing canonical renderers, ordered flow metadata,
//! deterministic bridge replays, published aliases, and the pilot vision
//! records. It intentionally does not render or mutate any canonical screen.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::audit::{audit_screen, sha256_hex};
use crate::flows::{flow_manifests, FlowManifest, FlowStep, TransitionKind};
use crate::registry::canonical_inventory;
use crate::scenario::replay_case;

const PILOT_FLOWS: [&str; 3] = ["daily-work", "input-and-approval", "work-configuration"];

const ACCEPTANCE_CHECKS: [&str; 13] = [
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
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PilotGateArtifact {
    pub flow: &'static str,
    pub step_index: usize,
    pub screen_id: &'static str,
    pub fixture_id: &'static str,
    pub canonical_relative: String,
    pub gate_relative: String,
    pub extension: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PilotGatePublication {
    pub flow: &'static str,
    pub step_index: usize,
    pub screen_id: &'static str,
    pub fixture_id: String,
    pub origin: String,
    pub action_id: &'static str,
    pub expected_next_bridge: Option<&'static str>,
    pub gate_txt: String,
    pub gate_png: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PilotGateBridge {
    pub flow: &'static str,
    pub case_id: &'static str,
    pub tick: u32,
    pub expected_before: &'static str,
    pub expected_after: &'static str,
    pub expected_kind: &'static str,
    pub expected_action: &'static str,
    pub role: &'static str,
    pub evidence_txt: &'static str,
    pub evidence_png: &'static str,
    pub audit_json: &'static str,
    pub vision_evidence: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisionFinding {
    pub flow: &'static str,
    pub gate_png: String,
    pub current_sha256: String,
    pub evidence_path: String,
    pub role: String,
    pub model: String,
    pub record_is_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PilotGateReport {
    pub flow_names: Vec<&'static str>,
    pub step_count: usize,
    pub artifact_count: usize,
    pub bridge_count: usize,
    pub programmatic_checks_before_vision: bool,
    pub vision_findings: Vec<VisionFinding>,
    pub required_pilot_iteration: bool,
}

const INTEGRATION_REPORT_RELATIVE_PATH: &str =
    "evidence/v3-gate-flows-1-3-integration/integration-report.md";

pub fn validate_review_session_id(session_id: &str) -> Result<(), String> {
    let value = session_id.trim();
    if value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "unavailable-from-analyze-image-response"
                | "placeholder-review-session"
                | "unknown"
                | "n/a"
                | "none"
        )
    {
        return Err("review session ID is missing or a placeholder".to_owned());
    }
    let valid_shape = value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            character.is_ascii_hexdigit() || matches!(index, 8 | 13 | 18 | 23) && character == '-'
        });
    if !valid_shape {
        return Err("review session ID must be a UUID".to_owned());
    }
    Ok(())
}

pub fn pilot_gate_bridges() -> &'static [PilotGateBridge] {
    &[
        PilotGateBridge {
            flow: "daily-work",
            case_id: "tool-success-continue",
            tick: 1,
            expected_before: "tool-success",
            expected_after: "rich-response",
            expected_kind: "direct",
            expected_action: "action.tool-success.continue",
            role: "first-three bridge daily-work tool result continuation",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/tool-success-continue-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/tool-success-continue-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/tool-success-continue-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/tool-success-continue-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "daily-work",
            case_id: "streaming-interrupt",
            tick: 1,
            expected_before: "streaming",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.streaming.interrupt",
            role: "first-three bridge daily-work streaming interruption",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/streaming-interrupt-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/streaming-interrupt-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/streaming-interrupt-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/streaming-interrupt-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "flow2-submit-bridge",
            tick: 1,
            expected_before: "file-mention",
            expected_after: "permission-shell",
            expected_kind: "direct",
            expected_action: "action.file-mention.submit-prepared-prompt",
            role: "first-three bridge input-and-approval prompt submission",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/flow2-submit-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/flow2-submit-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/flow2-submit-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/flow2-submit-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "permission-allow-once",
            tick: 1,
            expected_before: "permission-shell",
            expected_after: "diff-edit",
            expected_kind: "direct",
            expected_action: "action.permission.allow-once",
            role: "first-three bridge input-and-approval permission approval",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/permission-allow-once-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/permission-allow-once-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/permission-allow-once-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/permission-allow-once-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "diff-review",
            tick: 1,
            expected_before: "diff-edit",
            expected_after: "diff-viewer-staging",
            expected_kind: "direct",
            expected_action: "action.diff-edit.review-changes",
            role: "first-three bridge input-and-approval diff review",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/diff-review-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/diff-review-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/diff-review-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/diff-review-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "staging-finish",
            tick: 1,
            expected_before: "diff-viewer-staging",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.staging.finish",
            role: "first-three bridge input-and-approval staging finish",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/staging-finish-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/staging-finish-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/staging-finish-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/staging-finish-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "model-to-work-style-bridge",
            tick: 1,
            expected_before: "model-picker",
            expected_after: "agent-picker",
            expected_kind: "illustrative_detour",
            expected_action: "action.storyboard.open-work-style-after-model",
            role: "first-three bridge work-configuration model return to work style",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/model-to-work-style-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/model-to-work-style-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/model-to-work-style-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/model-to-work-style-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-pending",
            tick: 1,
            expected_before: "agent-picker",
            expected_after: "mode-switch",
            expected_kind: "direct",
            expected_action: "action.agent-picker.review-access",
            role: "first-three bridge work-configuration access review",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-pending-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-pending-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-pending-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-pending-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-confirm",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.mode-switch.confirm",
            role: "first-three bridge work-configuration access confirmation",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-confirm-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-confirm-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-confirm-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/work-style-access-confirm-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "mode-switch-to-session-bridge",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "session-picker",
            expected_kind: "illustrative_detour",
            expected_action: "action.storyboard.open-sessions-after-work-style",
            role: "first-three bridge work-configuration work style return to sessions",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/mode-switch-to-session-bridge-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/mode-switch-to-session-bridge-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/mode-switch-to-session-bridge-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/mode-switch-to-session-bridge-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-timeline-back",
            tick: 1,
            expected_before: "timeline",
            expected_after: "session-picker",
            expected_kind: "cancel",
            expected_action: "action.timeline.back",
            role: "first-three bridge work-configuration timeline back",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-timeline-back-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-timeline-back-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-timeline-back-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-timeline-back-tick-1.vision.txt",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-resume-latest",
            tick: 1,
            expected_before: "session-picker",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.session-picker.resume",
            role: "first-three bridge work-configuration latest session resume",
            evidence_txt: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-resume-latest-tick-1.txt",
            evidence_png: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-resume-latest-tick-1.png",
            audit_json: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-resume-latest-tick-1.json",
            vision_evidence: "evidence/v3-fix-gate-publication-bridge-evidence/bridges/session-resume-latest-tick-1.vision.txt",
        },
    ]
}

pub fn pilot_gate_publications() -> Vec<PilotGatePublication> {
    flow_manifests()
        .iter()
        .filter(|flow| PILOT_FLOWS.contains(&flow.name))
        .flat_map(|flow| {
            flow.steps
                .iter()
                .enumerate()
                .map(move |(index, step)| PilotGatePublication {
                    flow: flow.name,
                    step_index: index + 1,
                    screen_id: step.screen_id,
                    fixture_id: step.fixture_id.to_owned(),
                    origin: step.origin.to_owned(),
                    action_id: declared_action_id(flow.name, step.screen_id),
                    expected_next_bridge: match (flow.name, step.screen_id) {
                        ("work-configuration", "model-picker") => {
                            Some("model-to-work-style-bridge")
                        }
                        ("work-configuration", "mode-switch") => {
                            Some("mode-switch-to-session-bridge")
                        }
                        _ => None,
                    },
                    gate_txt: format!("{}/{:02}-{}.txt", flow.name, index + 1, step.screen_id),
                    gate_png: format!("{}/{:02}-{}.png", flow.name, index + 1, step.screen_id),
                })
        })
        .collect()
}

pub fn validate_pilot_gate_publications(
    publications: &[PilotGatePublication],
) -> Result<(), String> {
    if publications.len() != 18 {
        return Err(format!(
            "first-three publication map must contain 18 steps, got {}",
            publications.len()
        ));
    }
    let expected = pilot_gate_publications();
    let mut paths = BTreeSet::new();
    for (index, (actual, expected)) in publications.iter().zip(&expected).enumerate() {
        if actual.flow != expected.flow
            || actual.step_index != expected.step_index
            || actual.screen_id != expected.screen_id
            || actual.origin != expected.origin
        {
            return Err(format!(
                "publication origin/order mismatch at first-three step {}",
                index + 1
            ));
        }
        if actual.fixture_id != expected.fixture_id
            || actual.fixture_id.trim().is_empty()
            || actual.fixture_id.starts_with("canonical.")
        {
            return Err(format!(
                "publication fixture mismatch or canonical fallback at first-three step {}",
                index + 1
            ));
        }
        if actual.action_id != expected.action_id
            || actual.expected_next_bridge != expected.expected_next_bridge
        {
            return Err(format!(
                "publication action/bridge mismatch at first-three step {}",
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

pub fn validate_pilot_transition_contracts() -> Result<(), String> {
    let publications = pilot_gate_publications();
    validate_pilot_gate_publications(&publications)?;
    for flow in flow_manifests()
        .iter()
        .filter(|flow| PILOT_FLOWS.contains(&flow.name))
    {
        validate_manifest_steps(flow)?;
    }
    Ok(())
}

fn declared_action_id(flow: &str, screen_id: &str) -> &'static str {
    match (flow, screen_id) {
        ("daily-work", "home") => "action.home.start-request",
        ("daily-work", "chat-turn") => "action.chat-turn.send-reply",
        ("daily-work", "streaming") => "action.streaming.start-requested-tool",
        ("daily-work", "tool-running") => "action.tool-running.complete",
        ("daily-work", "tool-success") => "action.tool-success.continue",
        ("daily-work", "rich-response") => "action.rich-response.continue",
        ("input-and-approval", "composer-multiline") => {
            "action.composer-multiline.open-file-mention"
        }
        ("input-and-approval", "file-mention") => "action.file-mention.submit-prepared-prompt",
        ("input-and-approval", "permission-shell") => "action.permission.allow-once",
        ("input-and-approval", "diff-edit") => "action.diff-edit.review-changes",
        ("input-and-approval", "diff-viewer-staging") => "action.staging.finish",
        ("input-and-approval", "chat-turn") => "action.chat-turn.resume-request",
        ("work-configuration", "onboarding") => "action.onboarding.review-models",
        ("work-configuration", "model-picker") => "action.model-picker.apply",
        ("work-configuration", "agent-picker") => "action.agent-picker.review-access",
        ("work-configuration", "mode-switch") => "action.mode-switch.confirm",
        ("work-configuration", "session-picker") => "action.session-picker.timeline",
        ("work-configuration", "timeline") => "action.timeline.resume",
        _ => unreachable!("the authoritative first-three map contains only known steps"),
    }
}

pub fn pilot_gate_artifacts() -> Vec<PilotGateArtifact> {
    flow_manifests()
        .iter()
        .filter(|flow| PILOT_FLOWS.contains(&flow.name))
        .flat_map(|flow| {
            flow.steps
                .iter()
                .enumerate()
                .flat_map(move |(index, step)| {
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

pub fn validate_pilot_gate_integration(
    repository: &str,
    mission_directory: &str,
) -> Result<PilotGateReport, String> {
    let manifests = flow_manifests();
    crate::validate_flow_manifests(manifests, canonical_inventory())?;
    let pilot_manifests = manifests
        .iter()
        .filter(|flow| PILOT_FLOWS.contains(&flow.name))
        .collect::<Vec<_>>();
    if pilot_manifests.len() != PILOT_FLOWS.len() {
        return Err("first three pilot flow manifests are incomplete".to_owned());
    }
    if pilot_manifests
        .iter()
        .map(|flow| flow.name)
        .collect::<Vec<_>>()
        != PILOT_FLOWS
    {
        return Err("first three pilot flow order is not stable".to_owned());
    }

    for flow in &pilot_manifests {
        validate_manifest_steps(flow)?;
    }
    validate_pilot_gate_publications(&pilot_gate_publications())?;
    validate_bridges(mission_directory)?;

    let artifacts = pilot_gate_artifacts();
    validate_artifacts(repository, &artifacts)?;

    // Only after all programmatic checks above do we inspect the existing,
    // read-only pilot vision records. A missing current hash is a finding that
    // requires pilot iteration, not a reason to silently reuse stale evidence.
    let vision_findings = collect_vision_findings(repository, mission_directory, &artifacts)?;
    let required_pilot_iteration = vision_findings
        .iter()
        .any(|finding| !finding.record_is_current);
    let report = PilotGateReport {
        flow_names: PILOT_FLOWS.to_vec(),
        step_count: pilot_manifests.iter().map(|flow| flow.steps.len()).sum(),
        artifact_count: artifacts.len(),
        bridge_count: pilot_gate_bridges().len(),
        programmatic_checks_before_vision: true,
        vision_findings,
        required_pilot_iteration,
    };
    validate_persisted_report(repository, mission_directory, &report, &artifacts)?;
    Ok(report)
}

fn validate_manifest_steps(flow: &FlowManifest) -> Result<(), String> {
    for (index, step) in flow.steps.iter().enumerate() {
        if step
            .acceptance_checks
            .iter()
            .any(|check| !ACCEPTANCE_CHECKS.contains(check))
        {
            return Err(format!(
                "{} step {} has an unknown acceptance check",
                flow.name,
                index + 1
            ));
        }
        let audit = audit_screen(step.screen_id, step.fixture_id)?;
        audit.validate()?;
        let action_id = declared_action_id(flow.name, step.screen_id);
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
        if step.transition_kind == TransitionKind::Automatic && !rendered_action.automatic {
            return Err(format!(
                "{} step {} automatic transition is not bound to an automatic action",
                flow.name,
                index + 1
            ));
        }
        if matches!(flow.name, "daily-work" | "input-and-approval") {
            let identity = audit.conversation.as_ref().ok_or_else(|| {
                format!(
                    "{} step {} lacks conversation identity",
                    flow.name,
                    index + 1
                )
            })?;
            let expected = crate::audit::ConversationIdentity {
                conversation_id: step.conversation_id.to_owned(),
                transcript_id: step.transcript_id.to_owned(),
                session_id: step.session_id.to_owned(),
                turn_id: step.turn_id.to_owned(),
                user_task: step.user_task.to_owned(),
                visible_origin: step.visible_origin.to_owned(),
                submitted_prompt: (flow.name == "input-and-approval").then(|| {
                    "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge."
                        .to_owned()
                }),
                paused_activity_id: (flow.name == "input-and-approval")
                    .then(|| "activity.retry-policy-edit.18".to_owned()),
                later_draft: (flow.name == "input-and-approval")
                    .then(|| "Add a short note about the remaining test hunk.".to_owned()),
            };
            crate::audit::validate_conversation_identity(identity, &expected, &audit.render_txt())
                .map_err(|error| {
                    format!(
                        "{} step {} ({}) identity validation failed: {error}",
                        flow.name,
                        index + 1,
                        step.screen_id
                    )
                })?;
        }
        match step.transition_kind {
            TransitionKind::IllustrativeDetour => {
                let bridge = pilot_gate_publications()
                    .into_iter()
                    .find(|publication| {
                        publication.flow == flow.name
                            && publication.step_index == index + 1
                            && publication.screen_id == step.screen_id
                    })
                    .and_then(|publication| publication.expected_next_bridge)
                    .and_then(|case_id| {
                        pilot_gate_bridges()
                            .iter()
                            .find(|bridge| bridge.case_id == case_id)
                    })
                    .ok_or_else(|| {
                        format!(
                            "{} step {} illustrative adjacency has no named bridge",
                            flow.name,
                            index + 1
                        )
                    })?;
                if bridge.expected_before != step.screen_id
                    || bridge.expected_after != step.expected_next_frame
                    || bridge.expected_kind != "illustrative_detour"
                {
                    return Err(format!(
                        "{} step {} illustrative bridge does not reach {}",
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
                    TransitionKind::Summarized => "summarized",
                    TransitionKind::IllustrativeDetour => unreachable!(),
                };
                let rendered_kind = if kind == TransitionKind::Summarized {
                    "summarized"
                } else {
                    expected_kind
                };
                if !audit.transitions.iter().any(|transition| {
                    transition.action_id == action_id
                        && transition.to_screen_id == step.expected_next_frame
                        && transition.kind == rendered_kind
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
        _ => unreachable!("destination validation accepts only declared return fields"),
    };
    let required_kind = match field {
        "success_return" => match (flow, step.screen_id) {
            ("daily-work", "streaming" | "tool-running") => "automatic",
            _ => "direct",
        },
        "cancel_return" => "cancel",
        "terminal_return" => "direct",
        _ => unreachable!(),
    };
    if target == step.origin && step.origin == step.cancel_return {
        return Ok(());
    }
    let declared_action = declared_destination_action(flow, step.screen_id, field);
    let exact_transition = audit.transitions.iter().any(|transition| {
        transition.from_screen_id == step.screen_id
            && transition.to_screen_id == target
            && transition.kind == required_kind
            && declared_action.is_some_and(|action| transition.action_id == action)
            && audit.events.iter().any(|event| {
                event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.action_id == transition.action_id)
            })
    });
    if exact_transition {
        return Ok(());
    }

    if exact_named_destination(flow, step.screen_id, target, field, declared_action) {
        return Ok(());
    }

    Err(format!(
        "{flow} step {step_index} {field} {target} has no exact declared action or named bridge"
    ))
}

fn exact_named_destination(
    flow: &str,
    source: &str,
    target: &str,
    field: &str,
    declared_action: Option<&str>,
) -> bool {
    let bridge_kind_is_valid = |kind: &str| {
        (field == "cancel_return" && kind == "cancel")
            || (field != "cancel_return"
                && matches!(
                    kind,
                    "direct" | "automatic" | "summarized" | "illustrative_detour"
                ))
    };
    let exact = |case_id: &str, action: &str, before: &str, after: &str| {
        pilot_gate_bridges().iter().any(|bridge| {
            bridge.case_id == case_id
                && bridge.flow == flow
                && bridge.expected_before == before
                && bridge.expected_after == after
                && bridge.expected_action == action
                && bridge_kind_is_valid(bridge.expected_kind)
        })
    };
    if source == "agent-picker"
        && target == "chat-turn"
        && field == "success_return"
        && declared_action == Some("action.agent-picker.review-access")
    {
        return exact(
            "work-style-access-pending",
            "action.agent-picker.review-access",
            "agent-picker",
            "mode-switch",
        ) && exact(
            "work-style-access-confirm",
            "action.mode-switch.confirm",
            "mode-switch",
            "chat-turn",
        );
    }
    if source == "mode-switch"
        && target == "chat-turn"
        && field == "success_return"
        && declared_action == Some("action.mode-switch.confirm")
    {
        return exact(
            "work-style-access-confirm",
            "action.mode-switch.confirm",
            "mode-switch",
            "chat-turn",
        );
    }
    false
}

fn declared_destination_action(flow: &str, screen: &str, field: &str) -> Option<&'static str> {
    match (flow, screen, field) {
        ("daily-work", "streaming", "cancel_return") => Some("action.streaming.interrupt"),
        ("daily-work", "tool-running", "cancel_return") => Some("action.tool-running.interrupt"),
        ("input-and-approval", "file-mention", "cancel_return") => {
            Some("action.file-mention.dismiss")
        }
        ("input-and-approval", "permission-shell", "cancel_return") => {
            Some("action.permission.cancel")
        }
        ("input-and-approval", "diff-edit", "cancel_return") => Some("action.diff-edit.cancel"),
        ("input-and-approval", "diff-viewer-staging", "cancel_return") => {
            Some("action.staging.cancel")
        }
        ("work-configuration", "model-picker", "cancel_return") => {
            Some("action.model-picker.cancel")
        }
        ("work-configuration", "agent-picker", "cancel_return") => {
            Some("action.agent-picker.cancel")
        }
        ("work-configuration", "mode-switch", "cancel_return") => Some("action.mode-switch.cancel"),
        ("work-configuration", "session-picker", "cancel_return") => {
            Some("action.session-picker.back")
        }
        ("work-configuration", "timeline", "cancel_return") => Some("action.timeline.back"),
        ("daily-work", "rich-response", "terminal_return") => Some("action.rich-response.continue"),
        ("input-and-approval", "diff-viewer-staging", "success_return") => {
            Some("action.staging.finish")
        }
        ("input-and-approval", "diff-viewer-staging", "terminal_return") => {
            Some("action.staging.finish")
        }
        ("work-configuration", "timeline", "success_return") => Some("action.timeline.resume"),
        ("work-configuration", "timeline", "terminal_return") => Some("action.timeline.resume"),
        ("work-configuration", "session-picker", "success_return") => {
            Some("action.session-picker.resume")
        }
        ("work-configuration", "session-picker", "terminal_return") => {
            Some("action.session-picker.resume")
        }
        _ => declared_action_id(flow, screen).into(),
    }
}

fn validate_bridges(mission_directory: &str) -> Result<(), String> {
    if pilot_gate_bridges().len() != 12 {
        return Err(format!(
            "first-three integration requires exactly 12 bridge replays, got {}",
            pilot_gate_bridges().len()
        ));
    }
    let canonical_ids = canonical_inventory()
        .iter()
        .map(|screen| screen.id)
        .collect::<BTreeSet<_>>();
    let mut evidence_paths = BTreeSet::new();
    for bridge in pilot_gate_bridges() {
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
        let txt_bytes = fs::read(&txt_path)
            .map_err(|error| format!("missing bridge TXT {}: {error}", txt_path.display()))?;
        let expected_txt = replay.audit.render_txt().into_bytes();
        if txt_bytes != expected_txt {
            return Err(format!(
                "bridge TXT is stale or belongs to another replay: {}",
                txt_path.display()
            ));
        }
        let png_bytes = fs::read(&png_path)
            .map_err(|error| format!("missing bridge PNG {}: {error}", png_path.display()))?;
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
        let png_hash = sha256_hex(&png_bytes);
        let audit = fs::read_to_string(&audit_path)
            .map_err(|error| format!("missing bridge audit {}: {error}", audit_path.display()))?;
        if audit.trim_end() != replay.to_json() {
            return Err(format!(
                "bridge audit is not the exact replay JSON: {}",
                audit_path.display()
            ));
        }
        let review = read_review_record(&vision_path)?;
        require_review_field(&review, "artifact_path", &png_path.display().to_string())?;
        require_review_field(&review, "role", bridge.role)?;
        require_review_field(&review, "sha256", &png_hash)?;
        require_review_field(&review, "model", "anthropic/claude-opus-4.8-fast")?;
        require_review_field(&review, "case_id", bridge.case_id)?;
        require_review_field(&review, "tick", &bridge.tick.to_string())?;
        require_review_field(&review, "verdict", "pass")?;
        if review
            .get("review_session_id")
            .ok_or_else(|| {
                format!(
                    "bridge review lacks review session binding: {}",
                    vision_path.display()
                )
            })
            .and_then(|value| validate_review_session_id(value))
            .is_err()
            || review
                .get("response_sha256")
                .is_none_or(|value| value.len() != 64)
        {
            return Err(format!(
                "bridge review lacks exact review session/response binding: {}",
                vision_path.display()
            ));
        }
        for screen_id in [&replay.before.screen_id, &replay.after.screen_id] {
            if !canonical_ids.contains(screen_id.as_str()) {
                return Err(format!(
                    "bridge case {} introduced non-canonical screen {}",
                    bridge.case_id, screen_id
                ));
            }
        }
    }
    validate_bridge_continuity()?;
    Ok(())
}

fn validate_bridge_continuity() -> Result<(), String> {
    let flow2 = pilot_gate_bridges()
        .iter()
        .filter(|bridge| bridge.flow == "input-and-approval")
        .map(|bridge| replay_case(bridge.case_id, bridge.tick))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_session = "session.retry-review.18";
    let expected_activity = "activity.retry-policy-edit.18";
    for replay in flow2 {
        if replay.before.session_id != expected_session
            || replay.after.session_id != expected_session
            || replay.before.paused_activity_id != expected_activity
            || replay.after.paused_activity_id != expected_activity
        {
            return Err(format!(
                "Flow 2 bridge {} breaks session/paused-activity continuity",
                replay.case_id
            ));
        }
    }
    Ok(())
}

fn read_review_record(path: &Path) -> Result<std::collections::BTreeMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read review record {}: {error}", path.display()))?;
    let mut record = std::collections::BTreeMap::new();
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

fn require_review_field(
    review: &std::collections::BTreeMap<String, String>,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    match review.get(field) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!(
            "review field {field} mismatch: expected {expected}, got {actual}"
        )),
        None => Err(format!("review record is missing {field}")),
    }
}

fn validate_artifacts(repository: &str, artifacts: &[PilotGateArtifact]) -> Result<(), String> {
    let canonical_root = Path::new(repository).join("plans/assets/mockups-v3");
    let gate_root = Path::new(repository).join("plans/assets/mockups-v3-gates");
    if artifacts.len() != 36 {
        return Err(format!(
            "first three gate flows must publish 36 TXT/PNG artifacts, got {}",
            artifacts.len()
        ));
    }
    let mut paths = BTreeSet::new();
    for artifact in artifacts {
        if !paths.insert(&artifact.gate_relative) {
            return Err(format!(
                "duplicate gate artifact: {}",
                artifact.gate_relative
            ));
        }
        let canonical = canonical_root.join(&artifact.canonical_relative);
        let gate = gate_root.join(&artifact.gate_relative);
        let canonical_bytes = fs::read(&canonical)
            .map_err(|error| format!("could not read {}: {error}", canonical.display()))?;
        let gate_bytes = fs::read(&gate)
            .map_err(|error| format!("could not read {}: {error}", gate.display()))?;
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
        let has_fixture_variants = flow_manifests()
            .iter()
            .take(3)
            .flat_map(|flow| flow.steps.iter())
            .filter(|step| step.screen_id == artifact.screen_id)
            .map(|step| step.fixture_id)
            .collect::<BTreeSet<_>>()
            .len()
            > 1;
        if canonical_bytes != gate_bytes && !has_fixture_variants {
            return Err(format!(
                "canonical and gate alias bytes differ: {}",
                artifact.gate_relative
            ));
        }
        if expected_bytes != gate_bytes {
            return Err(format!(
                "gate alias is not canonical-equivalent for its declared fixture: {}",
                artifact.gate_relative
            ));
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
        } else if artifact.extension == "png" {
            #[cfg(feature = "png")]
            {
                let image = image::load_from_memory(&gate_bytes)
                    .map_err(|error| format!("invalid gate PNG: {error}"))?;
                if image.width() != 1200
                    || image.height() != 840
                    || image.color() != image::ColorType::Rgb8
                {
                    return Err(format!(
                        "invalid gate PNG dimensions: {}",
                        artifact.gate_relative
                    ));
                }
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
                "evidence/v3-fix-gate-publication-bridge-evidence/frames/{:02}-{}-{}.vision.txt",
                artifact.step_index, artifact.flow, artifact.screen_id
            );
            let evidence_path = Path::new(mission_directory).join(&evidence_relative);
            let role = format!(
                "first-three gate frame {} step {:02} {}",
                artifact.flow, artifact.step_index, artifact.screen_id
            );
            let review = read_review_record(&evidence_path)?;
            let record_is_current =
                require_review_field(&review, "artifact_path", &gate.display().to_string())
                    .and_then(|_| require_review_field(&review, "role", &role))
                    .and_then(|_| require_review_field(&review, "sha256", &current_sha256))
                    .and_then(|_| {
                        require_review_field(&review, "model", "anthropic/claude-opus-4.8-fast")
                    })
                    .and_then(|_| require_review_field(&review, "verdict", "pass"))
                    .is_ok()
                    && review
                        .get("review_session_id")
                        .is_some_and(|value| validate_review_session_id(value).is_ok())
                    && review
                        .get("response_sha256")
                        .is_some_and(|value| value.len() == 64);
            Ok(VisionFinding {
                flow: artifact.flow,
                gate_png: artifact.gate_relative.clone(),
                current_sha256: current_sha256.clone(),
                evidence_path: evidence_relative,
                role,
                model: review.get("model").cloned().unwrap_or_default(),
                record_is_current,
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
            "could not read persisted integration report {}: {error}",
            path.display()
        )
    })?;
    let expected_bridge_names = pilot_gate_bridges()
        .iter()
        .map(|bridge| bridge.case_id)
        .collect::<Vec<_>>();
    for flow in &report.flow_names {
        if !text.contains(&format!("`{flow}`")) {
            return Err(format!("persisted integration report omits flow {flow}"));
        }
    }
    for required in [
        format!("Ordered steps: {}", report.step_count),
        format!("Published TXT/PNG artifacts: {}", report.artifact_count),
        format!("Bridge cases: {} named replays", report.bridge_count),
        "Stale-finding count: **0**".to_owned(),
    ] {
        if !text.contains(&required) {
            return Err(format!(
                "persisted integration report is stale: missing {required}"
            ));
        }
    }
    for bridge in expected_bridge_names {
        if !text.contains(&format!("`{bridge}`")) {
            return Err(format!(
                "persisted integration report omits bridge {bridge}"
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
