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
pub struct PilotGateBridge {
    pub flow: &'static str,
    pub case_id: &'static str,
    pub tick: u32,
    pub expected_before: &'static str,
    pub expected_after: &'static str,
    pub expected_kind: &'static str,
    pub expected_action: &'static str,
    pub evidence_txt: Option<&'static str>,
    pub evidence_png: Option<&'static str>,
    pub audit_json: Option<&'static str>,
    pub vision_evidence: Option<&'static str>,
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
            evidence_txt: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/tool-success-continue-tick-1.txt",
            ),
            evidence_png: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/tool-success-continue-tick-1.png",
            ),
            audit_json: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/tool-success-continue-tick-1.json",
            ),
            vision_evidence: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/tool-success-continue-tick-1-vision.md",
            ),
        },
        PilotGateBridge {
            flow: "daily-work",
            case_id: "streaming-interrupt",
            tick: 1,
            expected_before: "streaming",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.streaming.interrupt",
            evidence_txt: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/streaming-interrupt-tick-1.txt",
            ),
            evidence_png: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/streaming-interrupt-tick-1.png",
            ),
            audit_json: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/streaming-interrupt-tick-1.json",
            ),
            vision_evidence: Some(
                "evidence/v3-pilot-flow1-storyboard/bridges/streaming-interrupt-tick-1-vision.md",
            ),
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "flow2-submit-bridge",
            tick: 1,
            expected_before: "file-mention",
            expected_after: "permission-shell",
            expected_kind: "direct",
            expected_action: "action.file-mention.submit-prepared-prompt",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "permission-allow-once",
            tick: 1,
            expected_before: "permission-shell",
            expected_after: "diff-edit",
            expected_kind: "direct",
            expected_action: "action.permission.allow-once",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "diff-review",
            tick: 1,
            expected_before: "diff-edit",
            expected_after: "diff-viewer-staging",
            expected_kind: "direct",
            expected_action: "action.diff-edit.review-changes",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "input-and-approval",
            case_id: "staging-finish",
            tick: 1,
            expected_before: "diff-viewer-staging",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.staging.finish",
            evidence_txt: Some(
                "evidence/v3-fix-flow2-storyboard-resumption/bridges/staging-finish-tick-1.txt",
            ),
            evidence_png: Some(
                "evidence/v3-fix-flow2-storyboard-resumption/bridges/staging-finish-tick-1.png",
            ),
            audit_json: Some(
                "evidence/v3-fix-flow2-storyboard-resumption/bridges/staging-finish-tick-1.json",
            ),
            vision_evidence: Some("evidence/v3-fix-flow2-storyboard-resumption/vision-evidence.md"),
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "model-picker-cancel",
            tick: 1,
            expected_before: "model-picker",
            expected_after: "chat-turn",
            expected_kind: "cancel",
            expected_action: "action.model-picker.cancel",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-pending",
            tick: 1,
            expected_before: "agent-picker",
            expected_after: "mode-switch",
            expected_kind: "direct",
            expected_action: "action.agent-picker.review-access",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-confirm",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.mode-switch.confirm",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "work-style-access-refused",
            tick: 1,
            expected_before: "mode-switch",
            expected_after: "mode-switch-refused",
            expected_kind: "direct",
            expected_action: "action.mode-switch-refused.choose-safer",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-timeline-back",
            tick: 1,
            expected_before: "timeline",
            expected_after: "session-picker",
            expected_kind: "cancel",
            expected_action: "action.timeline.back",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
        },
        PilotGateBridge {
            flow: "work-configuration",
            case_id: "session-resume-latest",
            tick: 1,
            expected_before: "session-picker",
            expected_after: "chat-turn",
            expected_kind: "direct",
            expected_action: "action.session-picker.resume",
            evidence_txt: None,
            evidence_png: None,
            audit_json: None,
            vision_evidence: None,
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
    validate_bridges(mission_directory)?;

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
            transition.kind == "cancel" || transition.to_screen_id == step.success_return
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

fn validate_bridges(mission_directory: &str) -> Result<(), String> {
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
        if let (Some(txt), Some(png), Some(audit), Some(vision)) = (
            bridge.evidence_txt,
            bridge.evidence_png,
            bridge.audit_json,
            bridge.vision_evidence,
        ) {
            let evidence_root = Path::new(mission_directory);
            let txt_path = evidence_root.join(txt);
            let png_path = evidence_root.join(png);
            let audit_path = evidence_root.join(audit);
            let vision_path = evidence_root.join(vision);
            let txt_bytes = fs::read(&txt_path)
                .map_err(|error| format!("missing bridge TXT {}: {error}", txt_path.display()))?;
            let txt = std::str::from_utf8(&txt_bytes)
                .map_err(|error| format!("bridge TXT is not UTF-8: {error}"))?;
            if txt.lines().count() != 40
                || !txt.ends_with('\n')
                || txt.lines().any(|line| line.chars().count() != 120)
            {
                return Err(format!(
                    "invalid bridge TXT dimensions: {}",
                    txt_path.display()
                ));
            }
            let png_bytes = fs::read(&png_path)
                .map_err(|error| format!("missing bridge PNG {}: {error}", png_path.display()))?;
            let png_hash = sha256_hex(&png_bytes);
            let audit = fs::read_to_string(&audit_path).map_err(|error| {
                format!("missing bridge audit {}: {error}", audit_path.display())
            })?;
            if !audit.contains(bridge.case_id)
                || !audit.contains(bridge.expected_action)
                || !audit.contains(&format!("\"virtual_tick\":{}", bridge.tick))
                || !audit.contains(bridge.expected_before)
                || !audit.contains(bridge.expected_after)
            {
                return Err(format!(
                    "bridge audit does not bind case, action, tick, and states: {}",
                    audit_path.display()
                ));
            }
            let vision = fs::read_to_string(&vision_path).map_err(|error| {
                format!(
                    "missing bridge vision evidence {}: {error}",
                    vision_path.display()
                )
            })?;
            if !vision.contains("anthropic/claude-opus-4.8-fast")
                || !vision.contains(&png_path.display().to_string())
                || !vision.contains(&png_hash)
                || !vision.contains("Role:")
            {
                return Err(format!(
                    "bridge vision evidence is not hash/path/model bound: {}",
                    vision_path.display()
                ));
            }
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
        if expected_bytes != gate_bytes {
            return Err(format!(
                "gate alias is not canonical-equivalent for its declared fixture: {}",
                artifact.gate_relative
            ));
        }
        if canonical_bytes == gate_bytes && artifact.fixture_id.trim().is_empty() {
            return Err(format!(
                "gate alias used an empty fixture fallback: {}",
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
            let evidence_text = fs::read_to_string(&evidence_path)
                .map_err(|error| format!("could not read {}: {error}", evidence_path.display()))?;
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
