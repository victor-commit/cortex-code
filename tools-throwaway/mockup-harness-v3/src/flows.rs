//! Stable, ordered gate manifests for the six v3 user journeys.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::registry::{canonical_inventory, FlowOwner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionKind {
    Direct,
    Automatic,
    Summarized,
    IllustrativeDetour,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowStep {
    pub screen_id: &'static str,
    pub user_action: &'static str,
    pub transition_kind: TransitionKind,
    pub transition_path: &'static str,
    pub expected_next_frame: &'static str,
    pub origin: &'static str,
    pub success_return: &'static str,
    pub cancel_return: &'static str,
    pub terminal_return: Option<&'static str>,
    pub acceptance_checks: &'static [&'static str],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowManifest {
    pub name: &'static str,
    pub steps: Vec<FlowStep>,
}

const DAILY_WORK: &[&str] = &[
    "home",
    "chat-turn",
    "streaming",
    "tool-running",
    "tool-success",
    "rich-response",
];
const INPUT_AND_APPROVAL: &[&str] = &[
    "composer-multiline",
    "file-mention",
    "permission-shell",
    "diff-edit",
    "diff-viewer-staging",
    "chat-turn",
];
const WORK_CONFIGURATION: &[&str] = &[
    "onboarding",
    "model-picker",
    "agent-picker",
    "mode-switch",
    "session-picker",
    "timeline",
];
const PROVIDER_AND_MCP_CONNECTION: &[&str] = &[
    "login",
    "connect-provider",
    "connect-method",
    "connect-oauth",
    "connect-connected",
    "mcp",
];
const RECOVERY: &[&str] = &[
    "turn-error",
    "error-network",
    "rate-limit",
    "chat-turn",
    "checkpoint-undo",
];
const MISSION: &[&str] = &[
    "workflow-launch-blocked",
    "workflow-launch",
    "workflow-monitor",
    "workflow-monitor-failure",
    "reviewer-output",
    "reviewer-approved",
];

const FLOW_NAMES: [&str; 6] = [
    "daily-work",
    "input-and-approval",
    "work-configuration",
    "provider-and-mcp-connection",
    "recovery",
    "mission",
];

const ENTRY_CHECK: &[&str] = &["screen-is-canonical", "origin-is-preserved"];
const TRANSITION_CHECK: &[&str] = &["screen-is-canonical", "next-frame-is-declared"];
const RETURN_CHECK: &[&str] = &["screen-is-canonical", "return-path-is-preserved"];
const RECOVERY_CHECK: &[&str] = &["screen-is-canonical", "work-is-preserved"];
const MISSION_CHECK: &[&str] = &["screen-is-canonical", "mission-origin-is-preserved"];

/// Returns all six manifests in their public, stable review order.
pub fn flow_manifests() -> &'static [FlowManifest] {
    static MANIFESTS: OnceLock<Vec<FlowManifest>> = OnceLock::new();
    MANIFESTS
        .get_or_init(|| {
            vec![
                FlowManifest {
                    name: "daily-work",
                    steps: vec![
                        step(
                            "home",
                            "Start a request",
                            TransitionKind::Direct,
                            "composer submission opens the current conversation",
                            "chat-turn",
                            "home",
                            "chat-turn",
                            "home",
                            None,
                            ENTRY_CHECK,
                        ),
                        step(
                            "chat-turn",
                            "Send the next request",
                            TransitionKind::Direct,
                            "the reply begins in the existing transcript",
                            "streaming",
                            "chat-turn",
                            "streaming",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "streaming",
                            "Run the requested tool",
                            TransitionKind::Direct,
                            "the inline activity becomes the one live tool row",
                            "tool-running",
                            "chat-turn",
                            "tool-running",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "tool-running",
                            "Wait for the active tool",
                            TransitionKind::Direct,
                            "the active operation resolves inline",
                            "tool-success",
                            "chat-turn",
                            "tool-success",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "tool-success",
                            "Continue from the tool result",
                            TransitionKind::Direct,
                            "the assistant completes the answer in the same transcript",
                            "rich-response",
                            "chat-turn",
                            "rich-response",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "rich-response",
                            "Continue the conversation",
                            TransitionKind::Direct,
                            "the composer remains available after the completed response",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            Some("chat-turn"),
                            RETURN_CHECK,
                        ),
                    ],
                },
                FlowManifest {
                    name: "input-and-approval",
                    steps: vec![
                        step(
                            "composer-multiline",
                            "Mention a project file",
                            TransitionKind::Direct,
                            "the preserved multiline draft opens a file mention dock",
                            "file-mention",
                            "chat-turn",
                            "file-mention",
                            "chat-turn",
                            None,
                            ENTRY_CHECK,
                        ),
                        step(
                            "file-mention",
                            "Submit the prepared prompt",
                            TransitionKind::Direct,
                            "submission pauses the same conversation for permission",
                            "permission-shell",
                            "chat-turn",
                            "permission-shell",
                            "composer-multiline",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "permission-shell",
                            "Approve the requested edit",
                            TransitionKind::Direct,
                            "approval opens the pending edit in the paused conversation",
                            "diff-edit",
                            "chat-turn",
                            "diff-edit",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "diff-edit",
                            "Inspect staged changes",
                            TransitionKind::Direct,
                            "the pending edit opens focused staging evidence",
                            "diff-viewer-staging",
                            "chat-turn",
                            "diff-viewer-staging",
                            "chat-turn",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "diff-viewer-staging",
                            "Apply the included changes",
                            TransitionKind::Direct,
                            "the original conversation resumes after the focused review",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "diff-edit",
                            Some("chat-turn"),
                            RETURN_CHECK,
                        ),
                        step(
                            "chat-turn",
                            "Resume the paused conversation",
                            TransitionKind::Summarized,
                            "the applied edit is summarized inline before continuing",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            Some("chat-turn"),
                            RETURN_CHECK,
                        ),
                    ],
                },
                FlowManifest {
                    name: "work-configuration",
                    steps: vec![
                        step(
                            "onboarding",
                            "Choose a model",
                            TransitionKind::Direct,
                            "the first configuration choice opens at the action dock",
                            "model-picker",
                            "onboarding",
                            "onboarding",
                            "onboarding",
                            None,
                            ENTRY_CHECK,
                        ),
                        step(
                            "model-picker",
                            "Review how Code should work",
                            TransitionKind::IllustrativeDetour,
                            "the selected model returns to configuration before the work-style choice",
                            "agent-picker",
                            "onboarding",
                            "onboarding",
                            "onboarding",
                            None,
                            RETURN_CHECK,
                        ),
                        step(
                            "agent-picker",
                            "Choose an access-changing work style",
                            TransitionKind::Direct,
                            "a work-style change that expands access requires confirmation",
                            "mode-switch",
                            "onboarding",
                            "mode-switch",
                            "onboarding",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "mode-switch",
                            "Return to configuration",
                            TransitionKind::IllustrativeDetour,
                            "the confirmed work style returns before session selection",
                            "session-picker",
                            "onboarding",
                            "agent-picker",
                            "agent-picker",
                            None,
                            RETURN_CHECK,
                        ),
                        step(
                            "session-picker",
                            "Inspect session history",
                            TransitionKind::IllustrativeDetour,
                            "the selected session returns to configuration before opening history",
                            "timeline",
                            "onboarding",
                            "onboarding",
                            "onboarding",
                            None,
                            RETURN_CHECK,
                        ),
                        step(
                            "timeline",
                            "Return to the configured conversation",
                            TransitionKind::Direct,
                            "the focused history view returns to its configuration origin",
                            "onboarding",
                            "onboarding",
                            "onboarding",
                            "onboarding",
                            Some("onboarding"),
                            RETURN_CHECK,
                        ),
                    ],
                },
                FlowManifest {
                    name: "provider-and-mcp-connection",
                    steps: vec![
                        step(
                            "login",
                            "Connect a provider",
                            TransitionKind::Direct,
                            "the blocked model action opens provider selection",
                            "connect-provider",
                            "model-picker",
                            "connect-provider",
                            "model-picker",
                            None,
                            ENTRY_CHECK,
                        ),
                        step(
                            "connect-provider",
                            "Choose a connection method",
                            TransitionKind::Direct,
                            "the chosen provider exposes its applicable methods",
                            "connect-method",
                            "model-picker",
                            "connect-method",
                            "model-picker",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "connect-method",
                            "Use browser sign-in",
                            TransitionKind::Direct,
                            "the provider branch opens the selected sign-in method",
                            "connect-oauth",
                            "model-picker",
                            "connect-oauth",
                            "connect-provider",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "connect-oauth",
                            "Finish provider sign-in",
                            TransitionKind::Direct,
                            "successful authentication records the connected provider",
                            "connect-connected",
                            "model-picker",
                            "connect-connected",
                            "connect-method",
                            None,
                            TRANSITION_CHECK,
                        ),
                        step(
                            "connect-connected",
                            "Open MCP connections separately",
                            TransitionKind::IllustrativeDetour,
                            "provider success returns to its blocked model origin before an explicit MCP action",
                            "mcp",
                            "model-picker",
                            "model-picker",
                            "model-picker",
                            None,
                            RETURN_CHECK,
                        ),
                        step(
                            "mcp",
                            "Return to the conversation",
                            TransitionKind::Direct,
                            "the capability view closes to its recorded conversation origin",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            Some("chat-turn"),
                            RETURN_CHECK,
                        ),
                    ],
                },
                FlowManifest {
                    name: "recovery",
                    steps: vec![
                        step(
                            "turn-error",
                            "Inspect the connection problem",
                            TransitionKind::Direct,
                            "the interrupted turn keeps its work while showing the recovery cause",
                            "error-network",
                            "chat-turn",
                            "error-network",
                            "chat-turn",
                            None,
                            RECOVERY_CHECK,
                        ),
                        step(
                            "error-network",
                            "Wait for the retry window",
                            TransitionKind::Direct,
                            "a bounded automatic wait moves to the rate limit state",
                            "rate-limit",
                            "chat-turn",
                            "rate-limit",
                            "chat-turn",
                            None,
                            RECOVERY_CHECK,
                        ),
                        step(
                            "rate-limit",
                            "Resume after the automatic retry",
                            TransitionKind::Automatic,
                            "the successful retry restores the paused conversation",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            None,
                            RECOVERY_CHECK,
                        ),
                        step(
                            "chat-turn",
                            "Undo the last checkpoint",
                            TransitionKind::Direct,
                            "the reversible checkpoint action opens beside the same conversation",
                            "checkpoint-undo",
                            "chat-turn",
                            "checkpoint-undo",
                            "chat-turn",
                            None,
                            RECOVERY_CHECK,
                        ),
                        step(
                            "checkpoint-undo",
                            "Return to the recovered conversation",
                            TransitionKind::Direct,
                            "the checkpoint result restores the preserved conversation",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            Some("chat-turn"),
                            RECOVERY_CHECK,
                        ),
                    ],
                },
                FlowManifest {
                    name: "mission",
                    steps: vec![
                        step(
                            "workflow-launch-blocked",
                            "Add the missing launch detail",
                            TransitionKind::Direct,
                            "automatic preflight returns to the launch decision after the blocker is resolved",
                            "workflow-launch",
                            "chat-turn",
                            "workflow-launch",
                            "chat-turn",
                            None,
                            MISSION_CHECK,
                        ),
                        step(
                            "workflow-launch",
                            "Launch the mission",
                            TransitionKind::Direct,
                            "the accepted plan enters compact exception-first monitoring",
                            "workflow-monitor",
                            "chat-turn",
                            "workflow-monitor",
                            "chat-turn",
                            None,
                            MISSION_CHECK,
                        ),
                        step(
                            "workflow-monitor",
                            "Open the validation failure",
                            TransitionKind::IllustrativeDetour,
                            "a summarized validation stage surfaces only the actionable failure",
                            "workflow-monitor-failure",
                            "chat-turn",
                            "workflow-monitor",
                            "chat-turn",
                            None,
                            MISSION_CHECK,
                        ),
                        step(
                            "workflow-monitor-failure",
                            "Create a fix task",
                            TransitionKind::Summarized,
                            "fix work and fresh validation complete before review opens",
                            "reviewer-output",
                            "chat-turn",
                            "workflow-monitor",
                            "workflow-monitor",
                            None,
                            MISSION_CHECK,
                        ),
                        step(
                            "reviewer-output",
                            "Approve completion",
                            TransitionKind::Direct,
                            "human approval records the completed mission review",
                            "reviewer-approved",
                            "chat-turn",
                            "reviewer-approved",
                            "workflow-monitor",
                            None,
                            MISSION_CHECK,
                        ),
                        step(
                            "reviewer-approved",
                            "Return to the conversation",
                            TransitionKind::Direct,
                            "the approved mission returns to the original conversation",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            "chat-turn",
                            Some("chat-turn"),
                            MISSION_CHECK,
                        ),
                    ],
                },
            ]
        })
        .as_slice()
}

/// Rejects an incomplete, renamed, reordered, or unresolved public journey map.
pub fn validate_flow_manifests(
    manifests: &[FlowManifest],
    inventory: &[crate::registry::ScreenSpec],
) -> Result<(), String> {
    let canonical_ids = inventory
        .iter()
        .map(|screen| screen.id)
        .collect::<BTreeSet<_>>();
    let mut actual_names = BTreeSet::new();

    for manifest in manifests {
        if !actual_names.insert(manifest.name) {
            return Err(format!("duplicate flow manifest: {}", manifest.name));
        }
        if !FLOW_NAMES.contains(&manifest.name) {
            return Err(format!("extra flow manifest: {}", manifest.name));
        }
    }
    for name in FLOW_NAMES {
        if !actual_names.contains(name) {
            return Err(format!("missing flow manifest: {name}"));
        }
    }
    if manifests
        .iter()
        .map(|manifest| manifest.name)
        .collect::<Vec<_>>()
        != FLOW_NAMES
    {
        return Err(format!(
            "flow manifest order mismatch: expected {:?}",
            FLOW_NAMES
        ));
    }

    for manifest in manifests {
        for (index, step) in manifest.steps.iter().enumerate() {
            if !canonical_ids.contains(step.screen_id) {
                return Err(format!("unknown canonical screen: {}", step.screen_id));
            }
            for (name, value) in [
                ("user_action", step.user_action),
                ("transition_path", step.transition_path),
                ("expected_next_frame", step.expected_next_frame),
                ("origin", step.origin),
                ("success_return", step.success_return),
                ("cancel_return", step.cancel_return),
            ] {
                if value.trim().is_empty() {
                    return Err(format!("empty {name} for {}", step.screen_id));
                }
            }
            for target in [
                step.expected_next_frame,
                step.origin,
                step.success_return,
                step.cancel_return,
            ] {
                if !canonical_ids.contains(target) {
                    return Err(format!(
                        "unresolved transition target {target} for {}",
                        step.screen_id
                    ));
                }
            }
            if let Some(target) = step.terminal_return {
                if !canonical_ids.contains(target) {
                    return Err(format!(
                        "unresolved terminal return {target} for {}",
                        step.screen_id
                    ));
                }
            }
            if step.acceptance_checks.is_empty()
                || step
                    .acceptance_checks
                    .iter()
                    .any(|check| check.trim().is_empty())
            {
                return Err(format!("missing acceptance checks for {}", step.screen_id));
            }
            if step.transition_kind == TransitionKind::Direct {
                if let Some(next_step) = manifest.steps.get(index + 1) {
                    if step.expected_next_frame != next_step.screen_id {
                        return Err(format!(
                            "undeclared direct edge from {} to {}",
                            step.screen_id, step.expected_next_frame
                        ));
                    }
                }
            }
        }

        let expected_screens = expected_flow_screens(manifest.name)
            .expect("names were validated against the fixed public manifest list");
        let actual_screens = manifest
            .steps
            .iter()
            .map(|step| step.screen_id)
            .collect::<Vec<_>>();
        if actual_screens != expected_screens {
            return Err(format!(
                "ordered frames mismatch for flow {}: expected {:?}, got {:?}",
                manifest.name, expected_screens, actual_screens
            ));
        }
    }

    if manifests
        .iter()
        .map(|manifest| manifest.steps.len())
        .sum::<usize>()
        != 35
    {
        return Err("gate manifests must contain exactly 35 steps".to_owned());
    }
    if canonical_inventory().len() != 86 {
        return Err("canonical inventory must contain exactly 86 states".to_owned());
    }

    Ok(())
}

fn expected_flow_screens(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "daily-work" => Some(DAILY_WORK),
        "input-and-approval" => Some(INPUT_AND_APPROVAL),
        "work-configuration" => Some(WORK_CONFIGURATION),
        "provider-and-mcp-connection" => Some(PROVIDER_AND_MCP_CONNECTION),
        "recovery" => Some(RECOVERY),
        "mission" => Some(MISSION),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    screen_id: &'static str,
    user_action: &'static str,
    transition_kind: TransitionKind,
    transition_path: &'static str,
    expected_next_frame: &'static str,
    origin: &'static str,
    success_return: &'static str,
    cancel_return: &'static str,
    terminal_return: Option<&'static str>,
    acceptance_checks: &'static [&'static str],
) -> FlowStep {
    FlowStep {
        screen_id,
        user_action,
        transition_kind,
        transition_path,
        expected_next_frame,
        origin,
        success_return,
        cancel_return,
        terminal_return,
        acceptance_checks,
    }
}

/// Binds the public flow names to the authoritative owner enum.
pub fn flow_owner(name: &str) -> Option<FlowOwner> {
    FlowOwner::ALL
        .into_iter()
        .find(|owner| owner.as_str() == name)
}
