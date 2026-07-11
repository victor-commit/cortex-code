//! Programmatic integration evidence for the first three human-gate journeys.
//!
//! This module joins existing canonical renderers, ordered flow metadata,
//! deterministic bridge replays, published aliases, and the pilot vision
//! records. It intentionally does not render or mutate any canonical screen.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::audit::{audit_screen, sha256_hex};
use crate::flows::{flow_manifests, FlowManifest, TransitionKind};
use crate::registry::canonical_inventory;
use crate::scenario::replay_case;

const PILOT_FLOWS: [&str; 3] = [
    "daily-work",
    "input-and-approval",
    "work-configuration",
];

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
    pub canonical_relative: String,
    pub gate_relative: String,
    pub extension: &'static str,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisionFinding {
    pub flow: &'static str,
    pub gate_png: String,
    pub current_sha256: String,
    pub evidence_path: String,
    pub evidence_contains_current_hash: bool,
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
        },
        PilotGateBridge {
            flow: "daily-work",
            case_id: "streaming-interrupt",
            tick: 1,
            expected_before: "streaming",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.streaming.interrupt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "flow2-submit-bridge",
            tick: 1,
            expected_before: "file-mention",
            expected_after: "permission-shell",
            expected_kind: "direct",
            expected_action: "action.file-mention.submit-prepared-prompt",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "permission-allow-once",
            tick: 1,
            expected_before: "permission-shell",
            expected_after: "diff-edit",
            expected_kind: "direct",
            expected_action: "action.permission.allow-once",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "diff-review",
            tick: 1,
            expected_before: "diff-edit",
            expected_after: "diff-viewer-staging",
            expected_kind: "direct",
            expected_action: "action.diff-edit.review-changes",
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "staging-finish",
            tick: 1,
            expected_before: "diff-viewer-staging",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.staging.finish",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "model-picker-cancel",
            tick: 1,
            expected_before: "model-picker",
            expected_after: "chat-turn",
            expected_kind: "cancel",
            expected_action: "action.model-picker.cancel",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-pending",
            tick: 1,
            expected_before: "agent-picker",
            expected_after: "mode-switch",
            expected_kind: "direct",
            expected_action: "action.agent-picker.review-access",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-confirm",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.mode-switch.confirm",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-refused",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "mode-switch-refused",
            expected_kind: "direct",
            expected_action: "action.mode-switch-refused.choose-safer",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-timeline-back",
            tick: 1,
            expected_before: "timeline",
            expected_after: "session-picker",
            expected_kind: "cancel",
            expected_action: "action.timeline.back",
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-resume-latest",
            tick: 1,
            expected_before: "session-picker",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.session-picker.resume",
        },
    ]
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
    validate_bridges()?;

    let artifacts = pilot_gate_artifacts();
    validate_artifacts(repository, &artifacts)?;

    // Only after all programmatic checks above do we inspect the existing,
    // read-only pilot vision records. A missing current hash is a finding that
    // requires pilot iteration, not a reason to silently reuse stale evidence.
    let vision_findings = collect_vision_findings(repository, mission_directory, &artifacts)?;
    let required_pilot_iteration = vision_findings
        .iter()
        .any(|finding| !finding.evidence_contains_current_hash);

    Ok(PilotGateReport {
        flow_names: PILOT_FLOWS.to_vec(),
        step_count: pilot_manifests.iter().map(|flow| flow.steps.len()).sum(),
        artifact_count: artifacts.len(),
        bridge_count: pilot_gate_bridges().len(),
        programmatic_checks_before_vision: true,
        vision_findings,
        required_pilot_iteration,
    })
}

fn validate_manifest_steps(flow: &FlowManifest) -> Result<(), String> {
    for (index, step) in flow.steps.iter().enumerate() {
        if step.acceptance_checks.iter().any(|check| {
            !ACCEPTANCE_CHECKS.contains(check)
        }) {
            return Err(format!(
                "{} step {} has an unknown acceptance check",
                flow.name,
                index + 1
            ));
        }
        let audit = audit_screen(
            step.screen_id,
            &format!("gate-integration.{}.{}", flow.name, index + 1),
        )?;
        audit.validate()?;
        if step.transition_kind == TransitionKind::Direct {
            let expected_kind = "direct";
            if !audit.transitions.iter().any(|transition| {
                transition.to_screen_id == step.expected_next_frame
                    && transition.kind == expected_kind
            }) {
                return Err(format!(
                    "{} step {} has no rendered direct edge to {}",
                    flow.name,
                    index + 1,
                    step.expected_next_frame
                ));
            }
        } else if !audit.transitions.iter().any(|transition| {
            transition.kind == "cancel"
                || transition.to_screen_id == step.success_return
        }) {
            return Err(format!(
                "{} step {} illustrative/automatic return is not rendered",
                flow.name,
                index + 1
            ));
        }
    }
    Ok(())
}

fn validate_bridges() -> Result<(), String> {
    let canonical_ids = canonical_inventory()
        .iter()
        .map(|screen| screen.id)
        .collect::<BTreeSet<_>>();
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
            return Err(format!("duplicate gate artifact: {}", artifact.gate_relative));
        }
        let canonical = canonical_root.join(&artifact.canonical_relative);
        let gate = gate_root.join(&artifact.gate_relative);
        let canonical_bytes = fs::read(&canonical)
            .map_err(|error| format!("could not read {}: {error}", canonical.display()))?;
        let gate_bytes = fs::read(&gate)
            .map_err(|error| format!("could not read {}: {error}", gate.display()))?;
        if canonical_bytes != gate_bytes {
            return Err(format!(
                "gate alias is not byte-identical to canonical artifact: {}",
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
                return Err(format!("invalid gate TXT dimensions: {}", artifact.gate_relative));
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
                    return Err(format!("invalid gate PNG dimensions: {}", artifact.gate_relative));
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
    let evidence = [
        (
            "daily-work",
            "evidence/v3-pilot-flow1-storyboard/vision-and-transition-evidence.md",
        ),
        (
            "input-and-approval",
            "evidence/v3-pilot-flow2-storyboard/vision-and-transition-evidence.md",
        ),
        (
            "work-configuration",
            "evidence/v3-pilot-flow3-storyboard/vision-and-transition-evidence.md",
        ),
    ];
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
            let evidence_relative = evidence
                .iter()
                .find(|(flow, _)| *flow == artifact.flow)
                .map(|(_, path)| *path)
                .ok_or_else(|| format!("missing vision evidence mapping for {}", artifact.flow))?;
            let evidence_path = Path::new(mission_directory).join(evidence_relative);
            let evidence_text = fs::read_to_string(&evidence_path).map_err(|error| {
                format!("could not read {}: {error}", evidence_path.display())
            })?;
            Ok(VisionFinding {
                flow: artifact.flow,
                gate_png: artifact.gate_relative.clone(),
                current_sha256: current_sha256.clone(),
                evidence_path: evidence_relative.to_owned(),
                evidence_contains_current_hash: evidence_text.contains(&current_sha256),
            })
        })
        .collect()
}
