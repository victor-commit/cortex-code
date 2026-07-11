//! Deterministic named interaction scenarios for typed audit evidence.

use std::collections::BTreeMap;

use crate::audit::{
    action_json, audit_screen, field, quote, sha256_hex, transition_json, ActionEvidence,
    AuditDocument, TransitionTrace,
};
use crate::screens::input_and_approval::{INSERTED_FILE, MENTION_FRAGMENT, MULTILINE_DRAFT};

const FLOW2_SESSION_ID: &str = "session.retry-review.18";
const FLOW2_PAUSED_ACTIVITY_ID: &str = "activity.retry-policy-edit.18";
const FLOW2_SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
const FLOW2_LATER_DRAFT: &str = "Add a short note about the remaining test hunk.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioState {
    pub screen_id: String,
    pub session_id: String,
    pub submitted_prompt: Option<String>,
    pub paused_activity_id: String,
    pub origin_stack: Vec<String>,
    pub draft: String,
    pub transcript: String,
    pub queued_follow_up: String,
    pub scroll_anchor: String,
    pub selection: String,
    pub focus: String,
    pub composer_height: String,
    pub pending_action: Option<String>,
    pub return_policy: String,
    pub virtual_tick: u32,
    pub state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreservedFieldHash {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptEvidence {
    pub attempt_id: String,
    pub owner_id: String,
    pub count: u32,
    pub outcome: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideEffectEvidence {
    pub effect_id: String,
    pub kind: String,
    pub status: String,
    pub simulated: bool,
    pub effect_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistentPermissionScope {
    pub matcher: String,
    pub duration: String,
    pub project_root: String,
    pub work_style: String,
    pub child_inheritance: String,
    pub child_cap: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionEvidence {
    pub execution_bytes: Vec<u8>,
    pub display_notation: String,
    pub command_sha256: String,
    pub byte_offset: usize,
    pub visible_total_length: usize,
    pub previous_offset: Option<usize>,
    pub next_offset: Option<usize>,
    pub at_boundary: bool,
    pub execution_working_directory: String,
    pub invocation_count: u32,
    pub persisted_rule_count: u32,
    pub verdict: String,
    pub persistent_scope: Option<PersistentPermissionScope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunkEvidence {
    pub hunk_id: String,
    pub file_path: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub state: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffEvidence {
    pub patch_revision: String,
    pub file_path: String,
    pub hunks: Vec<DiffHunkEvidence>,
    pub focused_hunk: String,
    pub included_count: usize,
    pub applied_hunks: Vec<String>,
    pub excluded_hunks: Vec<String>,
    pub write_count: u32,
    pub working_tree_before: String,
    pub working_tree_after: String,
    pub git_index_before: String,
    pub git_index_after: String,
}

impl PermissionEvidence {
    pub fn reconstructed_bytes(&self) -> Vec<u8> {
        decode_escaped_bytes(&self.display_notation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaseReplay {
    pub case_id: String,
    pub requested_tick: u32,
    pub virtual_tick: u32,
    pub before: ScenarioState,
    pub after: ScenarioState,
    pub actions: Vec<ActionEvidence>,
    pub transitions: Vec<TransitionTrace>,
    pub preserved_field_hashes: BTreeMap<String, PreservedFieldHash>,
    pub attempts: Vec<AttemptEvidence>,
    pub side_effects: Vec<SideEffectEvidence>,
    pub permission: Option<PermissionEvidence>,
    pub diff: Option<DiffEvidence>,
    pub audit: AuditDocument,
}

impl CaseReplay {
    pub fn validate(&self) -> Result<(), String> {
        if self.case_id.trim().is_empty()
            || self.requested_tick != self.virtual_tick
            || self.virtual_tick == 0
        {
            return Err("case replay must name one nonzero virtual tick".to_owned());
        }
        validate_state(&self.before)?;
        validate_state(&self.after)?;
        if self.before.virtual_tick + 1 != self.after.virtual_tick
            || self.after.virtual_tick != self.virtual_tick
        {
            return Err(
                "case replay must move through exactly one deterministic virtual tick".to_owned(),
            );
        }
        if self.before.origin_stack != self.after.origin_stack {
            return Err("scenario transition did not preserve its origin stack".to_owned());
        }
        if self.before.session_id != self.after.session_id
            || self.before.paused_activity_id != self.after.paused_activity_id
            || (self.case_id != "flow2-submit-bridge"
                && self.before.submitted_prompt != self.after.submitted_prompt)
        {
            return Err(
                "scenario transition did not preserve its session, submitted prompt, or paused activity identity"
                    .to_owned(),
            );
        }
        for state in [&self.before, &self.after] {
            if state
                .submitted_prompt
                .as_ref()
                .is_some_and(|prompt| !state.transcript.contains(prompt))
            {
                return Err(
                    "immutable submitted prompt is missing from transcript evidence".to_owned(),
                );
            }
        }
        if self.actions.len() != 1 || self.transitions.len() != 1 {
            return Err(
                "each foundation scenario tick must emit one action and one transition".to_owned(),
            );
        }
        let action = &self.actions[0];
        if action.action_id.trim().is_empty() || !action.enabled {
            return Err("case replay emitted an inaccessible action".to_owned());
        }
        let transition = &self.transitions[0];
        if transition.action_id != action.action_id
            || transition.from_screen_id != self.before.screen_id
            || transition.to_screen_id != self.after.screen_id
        {
            return Err(
                "case replay transition does not bind the exact action and states".to_owned(),
            );
        }
        if self.preserved_field_hashes.is_empty()
            || self
                .preserved_field_hashes
                .values()
                .any(|hash| hash.before.len() != 64 || hash.after.len() != 64)
        {
            return Err("case replay needs complete SHA-256 preservation evidence".to_owned());
        }
        if self.attempts.len() != 1
            || (self.attempts[0].count != 1
                && !((self.permission.is_some() || self.diff.is_some())
                    && self.attempts[0].count == 0))
            || self.attempts[0].attempt_id.trim().is_empty()
            || self.attempts[0].owner_id.trim().is_empty()
        {
            return Err("case replay requires exactly one bounded attempt".to_owned());
        }
        if self.side_effects.len() != 1
            || !self.side_effects[0].simulated
            || self.side_effects[0].effect_hash.len() != 64
        {
            return Err("case replay requires one hash-bound simulated side effect".to_owned());
        }
        self.audit.validate()?;
        if self.audit.screen_id != self.after.screen_id {
            return Err("case audit must describe the resulting rendered state".to_owned());
        }
        if let Some(permission) = &self.permission {
            if permission.command_sha256 != sha256_hex(&permission.execution_bytes)
                || permission.reconstructed_bytes() != permission.execution_bytes
            {
                return Err(
                    "permission evidence does not reconstruct exact execution bytes".to_owned(),
                );
            }
            if permission.execution_working_directory != "~/project" {
                return Err(
                    "permission evidence must name the canonical working directory".to_owned(),
                );
            }
            if permission.invocation_count > 1 || permission.persisted_rule_count > 1 {
                return Err(
                    "permission evidence exceeds single invocation or rule bounds".to_owned(),
                );
            }
            if permission.byte_offset > permission.visible_total_length
                || permission.visible_total_length != permission.execution_bytes.len()
                || permission.at_boundary
                    != (permission.byte_offset == 0
                        || permission.byte_offset == permission.visible_total_length)
            {
                return Err("permission page boundary evidence is inconsistent".to_owned());
            }
            if permission.verdict == "denied" && permission.invocation_count != 0 {
                return Err("denied permission evidence contains an invocation".to_owned());
            }
        }
        if let Some(diff) = &self.diff {
            validate_diff(diff)?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let mut output = String::new();
        output.push_str("{\"document_type\":\"case-replay\",\"schema_version\":1");
        field(&mut output, "case_id", &self.case_id);
        output.push_str(",\"requested_tick\":");
        output.push_str(&self.requested_tick.to_string());
        output.push_str(",\"virtual_tick\":");
        output.push_str(&self.virtual_tick.to_string());
        output.push_str(",\"before\":");
        state_json(&mut output, &self.before);
        output.push_str(",\"after\":");
        state_json(&mut output, &self.after);
        output.push_str(",\"actions\":[");
        for (index, action) in self.actions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            action_json(&mut output, action);
        }
        output.push_str("],\"transitions\":[");
        for (index, transition) in self.transitions.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            transition_json(&mut output, transition);
        }
        output.push_str("],\"preserved_field_hashes\":{");
        for (index, (field_name, hash)) in self.preserved_field_hashes.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            quote(&mut output, field_name);
            output.push_str(":{\"before\":");
            quote(&mut output, &hash.before);
            output.push_str(",\"after\":");
            quote(&mut output, &hash.after);
            output.push('}');
        }
        output.push_str("},\"attempts\":[");
        for (index, attempt) in self.attempts.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('{');
            raw_field(&mut output, "attempt_id", &attempt.attempt_id);
            field(&mut output, "owner_id", &attempt.owner_id);
            output.push_str(",\"count\":");
            output.push_str(&attempt.count.to_string());
            field(&mut output, "outcome", &attempt.outcome);
            output.push('}');
        }
        output.push_str("],\"side_effects\":[");
        for (index, effect) in self.side_effects.iter().enumerate() {
            if index > 0 {
                output.push(',');
            }
            output.push('{');
            raw_field(&mut output, "effect_id", &effect.effect_id);
            field(&mut output, "kind", &effect.kind);
            field(&mut output, "status", &effect.status);
            output.push_str(",\"simulated\":");
            output.push_str(if effect.simulated { "true" } else { "false" });
            field(&mut output, "effect_hash", &effect.effect_hash);
            output.push('}');
        }
        output.push_str("],\"permission\":");
        match &self.permission {
            Some(permission) => permission_json(&mut output, permission),
            None => output.push_str("null"),
        }
        output.push_str(",\"diff\":");
        match &self.diff {
            Some(diff) => diff_json(&mut output, diff),
            None => output.push_str("null"),
        }
        output.push_str(",\"audit\":");
        output.push_str(&self.audit.to_json());
        output.push('}');
        output
    }
}

/// Replays a fixed named case through one virtual transition without any I/O.
pub fn replay_case(case_id: &str, tick: u32) -> Result<CaseReplay, String> {
    if tick != 1 {
        return Err(format!("unknown virtual tick {tick} for case {case_id}"));
    }
    let (before, after, action, transition, attempt, effect) = match case_id {
        "flow2-submit-bridge" => flow2_submit_bridge(),
        "permission-allow-once" => permission_allow_once(),
        "permission-shell-short" => permission_shell_short(),
        "permission-shell-long" => permission_shell_long(),
        "permission-shell-long-page-0" => permission_shell_long(),
        "permission-shell-long-page-44" => permission_shell_long(),
        "permission-persistent-scope" => permission_persistent_scope(),
        "permission-persistent-scope-outside" => permission_persistent_scope_outside(),
        "permission-shell-unsafe-byte" => permission_shell_unsafe_byte(),
        "permission-deny" => permission_deny(),
        "permission-cancel" => permission_cancel(),
        "automatic-retry" => automatic_retry(),
        "streaming-interrupt" => streaming_interrupt(),
        "tool-running-interrupt" => tool_running_interrupt(),
        "tool-running-complete" => tool_running_complete(),
        "tool-success-continue" => tool_success_continue(),
        "file-mention-insert" => file_mention_insert(),
        "file-mention-dismiss" => file_mention_dismiss(),
        "file-mention-no-results" => file_mention_no_results(),
        "diff-review" => diff_review(),
        "diff-apply-all" => diff_apply_all(),
        "diff-cancel" => diff_cancel(),
        "staging-include-hunk" => staging_include_hunk(),
        "staging-exclude-hunk" => staging_exclude_hunk(),
        "staging-include-all" => staging_include_all(),
        "staging-finish" => staging_finish(),
        "staging-cancel" | "staging-back" => staging_cancel(),
        _ => return Err(format!("unknown named case: {case_id}")),
    };
    let mut preserved_field_hashes = preserved_hashes(&before, &after);
    if case_id == "file-mention-insert" {
        preserved_field_hashes.insert(
            "draft_surrounding".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.draft.replacen(MENTION_FRAGMENT, "", 1).as_bytes()),
                after: sha256_hex(after.draft.replacen(INSERTED_FILE, "", 1).as_bytes()),
            },
        );
    }
    let audit = audit_screen(&after.screen_id, &format!("case.{case_id}.tick.{tick}"))?;
    let replay = CaseReplay {
        case_id: case_id.to_owned(),
        requested_tick: tick,
        virtual_tick: tick,
        before,
        after,
        actions: vec![action],
        transitions: vec![transition],
        preserved_field_hashes,
        attempts: vec![attempt],
        side_effects: vec![effect],
        permission: permission_evidence(case_id),
        diff: diff_evidence(case_id),
        audit,
    };
    replay.validate()?;
    Ok(replay)
}

fn flow2_submit_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let inserted_prompt = MULTILINE_DRAFT.replacen(MENTION_FRAGMENT, INSERTED_FILE, 1);
    debug_assert_eq!(inserted_prompt, FLOW2_SUBMITTED_PROMPT);
    let mut before = state(
        "file-mention",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        &inserted_prompt,
        "turn-18",
        "src/net/retry_policy.rs",
        "selected-file",
        Some("mention.query.retry"),
        "return-to-multiline-draft",
        0,
    );
    before.session_id = FLOW2_SESSION_ID.to_owned();
    before.paused_activity_id = FLOW2_PAUSED_ACTIVITY_ID.to_owned();
    before.transcript =
        "Review the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.state_hash = state_hash(&before);

    let after = flow2_state(
        "permission-shell",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        "turn-18",
        "allow-once",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.file-mention.submit-prepared-prompt".to_owned(),
        label: "Submit prepared prompt".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "one explicit submit fixes the prepared prompt in the transcript and pauses the same session activity for permission"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.prompt.submit.1".to_owned(),
        owner_id: FLOW2_PAUSED_ACTIVITY_ID.to_owned(),
        count: 1,
        outcome: "submitted-once".to_owned(),
    };
    let effect = simulated_effect(
        "effect.prompt.submit",
        "prompt-submission",
        "submitted-once",
        "the selected file was inserted into one immutable transcript prompt without running the command",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_allow_once() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = flow2_state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "turn-18",
        "allow-once",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = flow2_state(
        "diff-edit",
        vec!["chat-turn", "permission-shell"],
        "turn-18",
        "allow-once",
        "pending-edit",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.allow-once".to_owned(),
        label: "Allow once".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "allow once runs the disclosed command once and resumes the paused transcript in the pending edit state"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.allow-once.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "approved".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.pending",
        "permission-decision",
        "executed-once",
        "permission allow-once executed the disclosed command once and resumed the paused turn",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_shell_short() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "short-command",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = before.clone();
    let mut after = after;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.permission.inspect-command".to_owned(),
        label: "Inspect command".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "short command stays inline while permission focus remains unchanged".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.short.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 0,
        outcome: "inspected".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.short",
        "permission-disclosure",
        "no-execution",
        "short command disclosure did not execute the command",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_shell_long() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (mut before, _, _, _, _, _) = permission_shell_short();
    before.selection = "long-command-page-0".to_owned();
    before.focus = "command-detail".to_owned();
    before.pending_action = Some("pending.shell.project-check".to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.permission.command-next".to_owned(),
        label: "Next page".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "command detail advances by the fixed byte page without changing execution bytes"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.long-page.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "page-inspected".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.long-page",
        "permission-disclosure",
        "no-execution",
        "long command page navigation did not execute the command",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_persistent_scope() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "persistent-scope",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "persistent-scope",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.allow-persistent".to_owned(),
        label: "Allow for this project".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the disclosed project-session scope is stored once before the command resumes"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.persistent.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "approved-with-scoped-rule".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.persistent",
        "permission-decision",
        "executed-once",
        "the command ran once and stored one bounded project-session rule",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_persistent_scope_outside() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (before, after, _, _, _, _) = permission_persistent_scope();
    let action = ActionEvidence {
        action_id: "action.permission.scope-outside".to_owned(),
        label: "Request permission again".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "a command outside the matcher, root, work style, child cap, or duration requests permission again"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.scope-outside.1".to_owned(),
        owner_id: "pending.shell.outside-scope".to_owned(),
        count: 0,
        outcome: "permission-requested".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.scope-outside",
        "permission-decision",
        "no-execution",
        "out-of-scope command requested a fresh permission decision",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_shell_unsafe_byte() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "unsafe-byte",
        "permission-choice",
        Some("pending.shell.unsafe-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "unsafe-byte",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.allow-once".to_owned(),
        label: "Allow once".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "escaped unsafe bytes execute exactly once after explicit approval".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.unsafe-byte.1".to_owned(),
        owner_id: "pending.shell.unsafe-check".to_owned(),
        count: 1,
        outcome: "approved".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.unsafe-byte",
        "permission-decision",
        "executed-once",
        "escaped command bytes executed once",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_deny() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "deny",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "permission-deny-reason",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "deny",
        "reason-input",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.deny".to_owned(),
        label: "Deny".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "denial records a visible denied verdict and opens the optional reason dock"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.deny.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 0,
        outcome: "denied".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.denied",
        "permission-decision",
        "no-execution",
        "denial performed zero command invocations",
    );
    (before, after, action, transition, attempt, effect)
}

fn permission_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "permission-shell",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "cancel",
        "permission-choice",
        Some("pending.shell.project-check"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "permission-shell"],
        "Check the project files\nthen summarize the result.",
        "turn-18",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.permission.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancellation restores the exact paused conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.cancel.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 0,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.none",
        "permission-decision",
        "cancelled-without-execution",
        "permission cancellation performed no execution",
    );
    (before, after, action, transition, attempt, effect)
}

fn automatic_retry() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "rate-limit",
        vec!["chat-turn", "error-network", "rate-limit"],
        "Finish the release notes.",
        "turn-22",
        "automatic-retry",
        "recovery-wait",
        Some("pending.provider.retry"),
        "return-to-paused-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "error-network", "rate-limit"],
        "Finish the release notes.",
        "turn-22",
        "automatic-retry",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.recovery.retry-window".to_owned(),
        label: "Retry window completed".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "one bounded retry restored the paused conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.provider.retry.1".to_owned(),
        owner_id: "pending.provider.retry".to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.provider.retry",
        "provider-attempt",
        "simulated-success",
        "one bounded provider retry restored the conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn streaming_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "streaming",
        vec!["home", "chat-turn", "streaming"],
        "Also list the tests that should cover the change.",
        "assistant-partial-auth-refactor",
        "stop-reply",
        "streaming-interrupt",
        Some("active.reply.auth-refactor"),
        "return-to-paused-turn",
        0,
    );
    before.transcript = "The refactor moves token checks into one shared session guard.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the change. [attachment: auth-notes.md]".to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "chat-turn",
        vec!["home", "chat-turn", "streaming"],
        "Also list the tests that should cover the change.",
        "assistant-partial-auth-refactor",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.streaming.interrupt".to_owned(),
        label: "Stop reply".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "local interruption saves the partial reply and restores the queued follow-up"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.streaming.interrupt.1".to_owned(),
        owner_id: "active.reply.auth-refactor".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.streaming.reply",
        "generation",
        "generation-stopped",
        "the active reply stopped once without altering the saved transcript or queued follow-up",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_running_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "tool-running",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "stop-search",
        "tool-running-interrupt",
        Some("activity.tool.search"),
        "return-to-same-turn",
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code is running.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the search. [attachment: rate-limit-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "chat-turn",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "composer",
        "composer",
        None,
        "return-to-same-turn",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.tool-running.interrupt".to_owned(),
        label: "Stop search".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "only the active search stops; the completed check, queued follow-up, and same turn remain"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool.search.interrupt.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool.search",
        "tool-execution",
        "search-stopped",
        "the selected search stopped once without changing the completed session check",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_running_complete() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "tool-running",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "search-completion",
        "tool-running",
        Some("activity.tool.search"),
        "return-to-same-turn",
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code is running.".to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the search. [attachment: rate-limit-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "tool-success",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "search-completion",
        "composer",
        None,
        "return-to-same-turn",
        1,
    );
    after.transcript = "Checked session rules completed. Search project code completed with 3 matches in 400 files."
        .to_owned();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.tool-running.complete".to_owned(),
        label: "Search completed".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "the active search resolves once by replacing its live row with one compact terminal result"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool.search.complete.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "succeeded".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool.search",
        "tool-execution",
        "search-completed",
        "one search completion produced one terminal summary with three matches",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_success_continue() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "tool-success",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "continue-response",
        "composer",
        None,
        "return-to-same-turn",
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code completed with 3 matches in 400 files."
            .to_owned();
    before.queued_follow_up =
        "Also list the tests that should cover the search. [attachment: rate-limit-notes.md]"
            .to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "rich-response",
        vec!["home", "chat-turn", "streaming", "tool-running"],
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "continue-response",
        "composer",
        None,
        "return-to-same-turn",
        1,
    );
    after.transcript =
        "The shared request guard applies the limit before route work begins.".to_owned();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.tool-success.continue".to_owned(),
        label: "Continue response".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the completed search expands into a readable answer in the same conversation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-success.continue.1".to_owned(),
        owner_id: "turn.rate-limit-summary".to_owned(),
        count: 1,
        outcome: "completed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.reply.rich-response",
        "response-completion",
        "completed",
        "one completed response added prose, a table, and code to the same turn",
    );
    (before, after, action, transition, attempt, effect)
}

fn file_mention_insert() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let inserted_draft = MULTILINE_DRAFT.replacen(MENTION_FRAGMENT, INSERTED_FILE, 1);
    let mut before = state(
        "file-mention",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        MULTILINE_DRAFT,
        "turn-retry-review",
        "active-mention-span-24..30",
        "draft-cursor-30",
        Some("mention.query.retry"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Review the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.queued_follow_up =
        "After this review, summarize the result. [attachment: retry-notes.md]".to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        &inserted_draft,
        "turn-retry-review",
        "no-selection",
        "draft-cursor-48",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.file-mention.insert-selected".to_owned(),
        label: "Insert file".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "only the active @retry span became @src/net/retry_policy.rs; surrounding authored lines remain byte-for-byte stable"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.file-mention.insert.1".to_owned(),
        owner_id: "mention.span.24..30".to_owned(),
        count: 1,
        outcome: "inserted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.file-mention.insert",
        "draft-edit",
        "simulated-span-replacement",
        "one selected project-relative file replaced only the active mention span",
    );
    (before, after, action, transition, attempt, effect)
}

fn file_mention_dismiss() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "file-mention",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        MULTILINE_DRAFT,
        "turn-retry-review",
        "active-mention-span-24..30",
        "draft-cursor-30",
        Some("mention.query.retry"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Review the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.queued_follow_up =
        "After this review, summarize the result. [attachment: retry-notes.md]".to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        MULTILINE_DRAFT,
        "turn-retry-review",
        "active-mention-span-24..30",
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.queued_follow_up = before.queued_follow_up.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.file-mention.dismiss".to_owned(),
        label: "Dismiss".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "dismissal restores the literal @retry fragment at the original cursor".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.file-mention.dismiss.1".to_owned(),
        owner_id: "mention.span.24..30".to_owned(),
        count: 1,
        outcome: "dismissed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.file-mention.dismiss",
        "draft-edit",
        "simulated-no-change",
        "dismissal leaves the exact multiline draft untouched",
    );
    (before, after, action, transition, attempt, effect)
}

fn file_mention_no_results() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let no_result_draft = MULTILINE_DRAFT.replacen(MENTION_FRAGMENT, "@zzq", 1);
    let mut before = state(
        "file-mention",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        MULTILINE_DRAFT,
        "turn-retry-review",
        "active-mention-span-24..30",
        "draft-cursor-30",
        Some("mention.query.retry"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Review the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "file-mention",
        vec!["chat-turn", "composer-multiline", "file-mention"],
        &no_result_draft,
        "turn-retry-review",
        "no-result",
        "draft-cursor-28",
        Some("mention.query.zzq"),
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.file-mention.filter-no-results".to_owned(),
        label: "Filter project files".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path:
            "typing keeps the literal @zzq query visible while the focused list reports no matches"
                .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.file-mention.filter.1".to_owned(),
        owner_id: "mention.span.24..28".to_owned(),
        count: 1,
        outcome: "no-results".to_owned(),
    };
    let effect = simulated_effect(
        "effect.file-mention.filter",
        "filter",
        "simulated-no-results",
        "the visible @zzq query produced zero project-file rows without discarding the draft",
    );
    (before, after, action, transition, attempt, effect)
}

fn diff_review() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = flow2_state(
        "diff-edit",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        "review-changes",
        "pending-edit",
        Some("pending.edit.patch-retry-18"),
        "return-to-paused-turn",
        0,
    );
    let after = flow2_state(
        "diff-viewer-staging",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        "hunk.retry-test",
        "staging-detail",
        Some("pending.edit.patch-retry-18"),
        "return-to-pending-edit",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.diff-edit.review-changes".to_owned(),
        label: "Review changes".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "read-only review opens the same patch revision without writing or approving it"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.diff.review.1".to_owned(),
        owner_id: "pending.edit.patch-retry-18".to_owned(),
        count: 0,
        outcome: "opened-read-only".to_owned(),
    };
    let effect = simulated_effect(
        "effect.diff.review",
        "patch-review",
        "no-write",
        "review opened exact file and hunk identities without changing the working tree or index",
    );
    (before, after, action, transition, attempt, effect)
}

fn diff_apply_all() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = diff_pending_state("apply-all", 0);
    let after = flow2_state(
        "chat-turn",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        "apply-all",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = diff_action("action.diff-edit.apply-all", "Apply all");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        "apply all writes the shown patch atomically and resumes the paused activity",
    );
    let attempt = diff_attempt("attempt.diff.apply-all.1", "applied-all", 1);
    let effect = simulated_effect(
        "effect.diff.apply-all",
        "patch-application",
        "applied-all-hunks",
        "both disclosed hunks were applied in one simulated atomic write",
    );
    (before, after, action, transition, attempt, effect)
}

fn diff_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = diff_pending_state("cancel", 0);
    let after = flow2_state(
        "chat-turn",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        "cancel",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = diff_action("action.diff-edit.cancel", "Cancel");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "cancel",
        "cancel writes nothing and restores the paused conversation and draft",
    );
    let attempt = diff_attempt("attempt.diff.cancel.1", "cancelled-without-write", 0);
    let effect = simulated_effect(
        "effect.diff.cancel",
        "patch-decision",
        "no-write",
        "pending edit cancellation discarded no hunk and restored the exact origin",
    );
    (before, after, action, transition, attempt, effect)
}

fn staging_include_hunk() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    staging_toggle("include", "hunk.retry-test", "Include")
}

fn staging_exclude_hunk() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    staging_toggle("exclude", "hunk.retry-guard", "Exclude")
}

fn staging_include_all() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = staging_state("include-all", 0);
    let after = staging_state("include-all", 1);
    let action = diff_action("action.staging.include-all", "Include all");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        "Include all marks every eligible hunk Included without writing the working tree",
    );
    let attempt = diff_attempt("attempt.staging.include-all.1", "include-all", 0);
    let effect = simulated_effect(
        "effect.staging.include-all",
        "hunk-selection",
        "no-write",
        "all eligible hunks were selected while the working tree and Git index stayed unchanged",
    );
    (before, after, action, transition, attempt, effect)
}

fn staging_finish() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = staging_state("finish", 0);
    let after = flow2_state(
        "chat-turn",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        "finish",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    let action = diff_action("action.staging.finish", "Finish");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        "Finish applies only the included hunk atomically and resumes the paused activity",
    );
    let attempt = diff_attempt("attempt.staging.finish.1", "applied-included-hunks", 1);
    let effect = simulated_effect(
        "effect.staging.finish",
        "patch-application",
        "applied-included-hunks",
        "one included hunk was written atomically and the excluded hunk remained pending",
    );
    (before, after, action, transition, attempt, effect)
}

fn staging_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = staging_state("cancel", 0);
    let after = diff_pending_state("return", 1);
    let action = diff_action("action.staging.cancel", "Cancel");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "cancel",
        "Cancel applies none and returns one level to the unchanged pending edit",
    );
    let attempt = diff_attempt("attempt.staging.cancel.1", "cancelled-without-write", 0);
    let effect = simulated_effect(
        "effect.staging.cancel",
        "patch-review",
        "no-write",
        "staging cancellation preserved the patch revision and included states without writing",
    );
    (before, after, action, transition, attempt, effect)
}

fn diff_pending_state(selection: &str, tick: u32) -> ScenarioState {
    flow2_state(
        "diff-edit",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        selection,
        "pending-edit",
        Some("pending.edit.patch-retry-18"),
        "return-to-paused-turn",
        tick,
    )
}

fn staging_state(selection: &str, tick: u32) -> ScenarioState {
    flow2_state(
        "diff-viewer-staging",
        vec!["chat-turn", "permission-shell", "diff-edit"],
        "turn-18",
        selection,
        "staging-detail",
        Some("pending.edit.patch-retry-18"),
        "return-to-pending-edit",
        tick,
    )
}

fn diff_action(action_id: &str, label: &str) -> ActionEvidence {
    ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic: false,
    }
}

fn diff_transition(
    action: &ActionEvidence,
    before: &ScenarioState,
    after: &ScenarioState,
    kind: &str,
    path: &str,
) -> TransitionTrace {
    TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    }
}

fn diff_attempt(attempt_id: &str, outcome: &str, count: u32) -> AttemptEvidence {
    AttemptEvidence {
        attempt_id: attempt_id.to_owned(),
        owner_id: "pending.edit.patch-retry-18".to_owned(),
        count,
        outcome: outcome.to_owned(),
    }
}

fn staging_toggle(
    name: &str,
    focused_hunk: &str,
    label: &str,
) -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = staging_state(name, 0);
    let after = staging_state(name, 1);
    let action = diff_action(&format!("action.staging.{}", name), label);
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        &format!("{} changes only the focused hunk {focused_hunk} and writes nothing", label),
    );
    let attempt = diff_attempt(
        &format!("attempt.staging.{name}.1"),
        "selection-updated",
        0,
    );
    let effect = simulated_effect(
        &format!("effect.staging.{name}"),
        "hunk-selection",
        "no-write",
        &format!("{label} changed only focused hunk {focused_hunk}"),
    );
    (before, after, action, transition, attempt, effect)
}

#[allow(clippy::too_many_arguments)]
fn flow2_state(
    screen_id: &str,
    origin_stack: Vec<&str>,
    scroll_anchor: &str,
    selection: &str,
    focus: &str,
    pending_action: Option<&str>,
    return_policy: &str,
    virtual_tick: u32,
) -> ScenarioState {
    let mut state = state(
        screen_id,
        origin_stack,
        FLOW2_LATER_DRAFT,
        scroll_anchor,
        selection,
        focus,
        pending_action,
        return_policy,
        virtual_tick,
    );
    state.session_id = FLOW2_SESSION_ID.to_owned();
    state.submitted_prompt = Some(FLOW2_SUBMITTED_PROMPT.to_owned());
    state.paused_activity_id = FLOW2_PAUSED_ACTIVITY_ID.to_owned();
    state.transcript = format!("You · {FLOW2_SUBMITTED_PROMPT}");
    state.state_hash = state_hash(&state);
    state
}

#[allow(clippy::too_many_arguments)]
fn state(
    screen_id: &str,
    origin_stack: Vec<&str>,
    draft: &str,
    scroll_anchor: &str,
    selection: &str,
    focus: &str,
    pending_action: Option<&str>,
    return_policy: &str,
    virtual_tick: u32,
) -> ScenarioState {
    let mut state = ScenarioState {
        screen_id: screen_id.to_owned(),
        session_id: "session.daily-work.1".to_owned(),
        submitted_prompt: None,
        paused_activity_id: "activity.current-turn".to_owned(),
        origin_stack: origin_stack.into_iter().map(str::to_owned).collect(),
        draft: draft.to_owned(),
        transcript: "The current conversation remains available.".to_owned(),
        queued_follow_up: String::new(),
        scroll_anchor: scroll_anchor.to_owned(),
        selection: selection.to_owned(),
        focus: focus.to_owned(),
        composer_height: "5-visual-rows".to_owned(),
        pending_action: pending_action.map(str::to_owned),
        return_policy: return_policy.to_owned(),
        virtual_tick,
        state_hash: String::new(),
    };
    state.state_hash = state_hash(&state);
    state
}

fn validate_state(state: &ScenarioState) -> Result<(), String> {
    if state.screen_id.trim().is_empty()
        || state.session_id.trim().is_empty()
        || state.paused_activity_id.trim().is_empty()
        || state.origin_stack.is_empty()
        || state.draft.is_empty()
        || state.scroll_anchor.is_empty()
        || state.selection.is_empty()
        || state.focus.is_empty()
        || state.composer_height.is_empty()
        || state.return_policy.is_empty()
        || state.state_hash.len() != 64
    {
        return Err("scenario state requires origin and preserved field evidence".to_owned());
    }
    if state_hash(state) != state.state_hash {
        return Err("scenario state hash does not match its stable fields".to_owned());
    }
    Ok(())
}

fn validate_diff(diff: &DiffEvidence) -> Result<(), String> {
    if diff.patch_revision != "patch-retry-18"
        || diff.file_path != "src/net/retry_policy.rs"
        || diff.hunks.len() != 2
        || diff.focused_hunk.trim().is_empty()
        || diff.git_index_before != diff.git_index_after
        || diff.hunks.iter().any(|hunk| {
            hunk.file_path != diff.file_path
                || hunk.hunk_id.trim().is_empty()
                || !matches!(hunk.state.as_str(), "Included" | "Excluded" | "Pending")
                || hunk.content_hash.len() != 64
        })
        || diff.included_count != diff
            .hunks
            .iter()
            .filter(|hunk| hunk.state == "Included")
            .count()
    {
        return Err("diff evidence has incomplete identity, state, or index preservation".to_owned());
    }
    if diff.applied_hunks.iter().any(|hunk| {
        !diff
            .hunks
            .iter()
            .any(|candidate| candidate.hunk_id == *hunk && candidate.state == "Included")
    }) {
        return Err("diff evidence applies a hunk that is not included".to_owned());
    }
    if diff.excluded_hunks.iter().any(|hunk| {
        !diff
            .hunks
            .iter()
            .any(|candidate| candidate.hunk_id == *hunk && candidate.state != "Included")
    }) {
        return Err("diff evidence loses an excluded hunk".to_owned());
    }
    if diff.write_count > 1 {
        return Err("diff evidence exceeds one atomic write".to_owned());
    }
    Ok(())
}

fn state_hash(state: &ScenarioState) -> String {
    sha256_hex(
        format!(
            "screen={}\nsession={}\nsubmitted_prompt={}\npaused_activity={}\norigins={}\ndraft={}\ntranscript={}\nqueued_follow_up={}\nscroll={}\nselection={}\nfocus={}\ncomposer_height={}\npending={}\nreturn={}\ntick={}",
            state.screen_id,
            state.session_id,
            state.submitted_prompt.as_deref().unwrap_or(""),
            state.paused_activity_id,
            state.origin_stack.join(">"),
            state.draft,
            state.transcript,
            state.queued_follow_up,
            state.scroll_anchor,
            state.selection,
            state.focus,
            state.composer_height,
            state.pending_action.as_deref().unwrap_or(""),
            state.return_policy,
            state.virtual_tick,
        )
        .as_bytes(),
    )
}

fn preserved_hashes(
    before: &ScenarioState,
    after: &ScenarioState,
) -> BTreeMap<String, PreservedFieldHash> {
    let mut hashes = BTreeMap::new();
    for (name, first, second) in [
        (
            "session_id",
            before.session_id.as_str(),
            after.session_id.as_str(),
        ),
        (
            "submitted_prompt",
            before.submitted_prompt.as_deref().unwrap_or(""),
            after.submitted_prompt.as_deref().unwrap_or(""),
        ),
        (
            "paused_activity_id",
            before.paused_activity_id.as_str(),
            after.paused_activity_id.as_str(),
        ),
        ("draft", before.draft.as_str(), after.draft.as_str()),
        (
            "transcript",
            before.transcript.as_str(),
            after.transcript.as_str(),
        ),
        (
            "queued_follow_up",
            before.queued_follow_up.as_str(),
            after.queued_follow_up.as_str(),
        ),
        (
            "scroll_anchor",
            before.scroll_anchor.as_str(),
            after.scroll_anchor.as_str(),
        ),
        (
            "origin_stack",
            &before.origin_stack.join(">"),
            &after.origin_stack.join(">"),
        ),
        (
            "return_policy",
            before.return_policy.as_str(),
            after.return_policy.as_str(),
        ),
        (
            "selection",
            before.selection.as_str(),
            after.selection.as_str(),
        ),
        ("focus", before.focus.as_str(), after.focus.as_str()),
        (
            "patch_revision",
            "patch-retry-18",
            "patch-retry-18",
        ),
        (
            "composer_height",
            before.composer_height.as_str(),
            after.composer_height.as_str(),
        ),
    ] {
        hashes.insert(
            name.to_owned(),
            PreservedFieldHash {
                before: sha256_hex(first.as_bytes()),
                after: sha256_hex(second.as_bytes()),
            },
        );
    }
    hashes
}

fn diff_hunks(states: [&str; 2]) -> Vec<DiffHunkEvidence> {
    vec![
        DiffHunkEvidence {
            hunk_id: "hunk.retry-guard".to_owned(),
            file_path: "src/net/retry_policy.rs".to_owned(),
            old_start: 42,
            old_lines: 2,
            new_start: 42,
            new_lines: 3,
            state: states[0].to_owned(),
            content_hash: sha256_hex(b"hunk.retry-guard\n-    return retry_after\n+    return retry_after + jitter"),
        },
        DiffHunkEvidence {
            hunk_id: "hunk.retry-test".to_owned(),
            file_path: "src/net/retry_policy.rs".to_owned(),
            old_start: 88,
            old_lines: 2,
            new_start: 88,
            new_lines: 2,
            state: states[1].to_owned(),
            content_hash: sha256_hex(b"hunk.retry-test\n-assert_eq!(wait, 4)\n+assert_eq!(wait, 5)"),
        },
    ]
}

fn diff_evidence(case_id: &str) -> Option<DiffEvidence> {
    if !matches!(
        case_id,
        "diff-review"
            | "diff-apply-all"
            | "diff-cancel"
            | "staging-include-hunk"
            | "staging-exclude-hunk"
            | "staging-include-all"
            | "staging-finish"
            | "staging-cancel"
            | "staging-back"
    ) {
        return None;
    }
    let (states, focused_hunk, applied_hunks, excluded_hunks, write_count, tree_after) =
        match case_id {
            "diff-apply-all" => (
                ["Included", "Included"],
                "hunk.retry-guard",
                vec!["hunk.retry-guard".to_owned(), "hunk.retry-test".to_owned()],
                vec![],
                1,
                "tree-after-all",
            ),
            "staging-include-all" => (
                ["Included", "Included"],
                "hunk.retry-guard",
                vec![],
                vec![],
                0,
                "tree-before",
            ),
            "staging-include-hunk" => (
                ["Included", "Included"],
                "hunk.retry-test",
                vec![],
                vec![],
                0,
                "tree-before",
            ),
            "staging-exclude-hunk" => (
                ["Excluded", "Pending"],
                "hunk.retry-guard",
                vec![],
                vec![],
                0,
                "tree-before",
            ),
            "staging-finish" => (
                ["Included", "Pending"],
                "hunk.retry-guard",
                vec!["hunk.retry-guard".to_owned()],
                vec!["hunk.retry-test".to_owned()],
                1,
                "tree-after-guard",
            ),
            _ => (
                ["Included", "Pending"],
                "hunk.retry-test",
                vec![],
                vec!["hunk.retry-test".to_owned()],
                0,
                "tree-before",
            ),
        };
    Some(DiffEvidence {
        patch_revision: "patch-retry-18".to_owned(),
        file_path: "src/net/retry_policy.rs".to_owned(),
        hunks: diff_hunks(states),
        focused_hunk: focused_hunk.to_owned(),
        included_count: states.iter().filter(|state| **state == "Included").count(),
        applied_hunks,
        excluded_hunks,
        write_count,
        working_tree_before: "tree-before".to_owned(),
        working_tree_after: tree_after.to_owned(),
        git_index_before: "index-unchanged".to_owned(),
        git_index_after: "index-unchanged".to_owned(),
    })
}

fn simulated_effect(effect_id: &str, kind: &str, status: &str, detail: &str) -> SideEffectEvidence {
    SideEffectEvidence {
        effect_id: effect_id.to_owned(),
        kind: kind.to_owned(),
        status: status.to_owned(),
        simulated: true,
        effect_hash: sha256_hex(detail.as_bytes()),
    }
}

fn permission_evidence(case_id: &str) -> Option<PermissionEvidence> {
    use crate::screens::input_and_approval::{
        LONG_SHELL_COMMAND, SHORT_SHELL_COMMAND, UNSAFE_SHELL_COMMAND,
    };
    let (execution_bytes, display_notation) = if case_id.contains("unsafe-byte") {
        (
            UNSAFE_SHELL_COMMAND.to_vec(),
            command_notation(UNSAFE_SHELL_COMMAND, true),
        )
    } else if case_id.contains("long") {
        (
            LONG_SHELL_COMMAND.to_vec(),
            command_notation(LONG_SHELL_COMMAND, false),
        )
    } else {
        (
            SHORT_SHELL_COMMAND.to_vec(),
            command_notation(SHORT_SHELL_COMMAND, false),
        )
    };
    let persistent_scope =
        case_id
            .starts_with("permission-persistent-scope")
            .then(|| PersistentPermissionScope {
                matcher: "git diff -- src/net/*".to_owned(),
                duration: "this session".to_owned(),
                project_root: "~/project".to_owned(),
                work_style: "Careful edits".to_owned(),
                child_inheritance: "children inherit only read-only checks".to_owned(),
                child_cap: "read-only, same project root, same session".to_owned(),
            });
    let visible_total_length = execution_bytes.len();
    Some(PermissionEvidence {
        command_sha256: sha256_hex(&execution_bytes),
        execution_bytes,
        display_notation,
        byte_offset: if case_id.contains("page-44") { 44 } else { 0 },
        visible_total_length,
        previous_offset: if case_id.contains("page-44") {
            Some(0)
        } else {
            None
        },
        next_offset: if case_id.contains("long") && !case_id.contains("page-44") {
            Some(44)
        } else {
            None
        },
        at_boundary: !case_id.contains("page-44"),
        execution_working_directory: "~/project".to_owned(),
        invocation_count: if case_id == "permission-allow-once"
            || case_id == "permission-persistent-scope"
            || case_id.contains("unsafe-byte")
        {
            1
        } else {
            0
        },
        persisted_rule_count: u32::from(case_id == "permission-persistent-scope"),
        verdict: if case_id == "permission-deny" {
            "denied"
        } else if case_id == "permission-cancel" {
            "pending"
        } else {
            "approved"
        }
        .to_owned(),
        persistent_scope,
    })
}

fn command_notation(bytes: &[u8], escape_unsafe: bool) -> String {
    bytes
        .iter()
        .map(|byte| {
            if escape_unsafe && (*byte < 0x20 || *byte == 0x7f) {
                format!("\\x{byte:02x}")
            } else {
                char::from(*byte).to_string()
            }
        })
        .collect()
}

fn decode_escaped_bytes(notation: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(notation.len());
    let bytes = notation.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let escaped =
            if index + 3 < bytes.len() && bytes[index] == b'\\' && bytes[index + 1] == b'x' {
                u8::from_str_radix(
                    std::str::from_utf8(&bytes[index + 2..index + 4]).unwrap_or(""),
                    16,
                )
                .ok()
            } else {
                None
            };
        if let Some(value) = escaped {
            output.push(value);
            index += 4;
        } else {
            let character = notation[index..].chars().next().unwrap_or_default();
            let mut encoded = [0; 4];
            output.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            index += character.len_utf8();
        }
    }
    output
}

fn permission_json(output: &mut String, permission: &PermissionEvidence) {
    output.push('{');
    output.push_str("\"execution_bytes\":[");
    for (index, byte) in permission.execution_bytes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&byte.to_string());
    }
    output.push(']');
    field(output, "display_notation", &permission.display_notation);
    field(output, "command_sha256", &permission.command_sha256);
    output.push_str(",\"byte_offset\":");
    output.push_str(&permission.byte_offset.to_string());
    output.push_str(",\"visible_total_length\":");
    output.push_str(&permission.visible_total_length.to_string());
    output.push_str(",\"previous_offset\":");
    match permission.previous_offset {
        Some(offset) => output.push_str(&offset.to_string()),
        None => output.push_str("null"),
    }
    output.push_str(",\"next_offset\":");
    match permission.next_offset {
        Some(offset) => output.push_str(&offset.to_string()),
        None => output.push_str("null"),
    }
    output.push_str(",\"at_boundary\":");
    output.push_str(if permission.at_boundary {
        "true"
    } else {
        "false"
    });
    field(
        output,
        "execution_working_directory",
        &permission.execution_working_directory,
    );
    output.push_str(",\"invocation_count\":");
    output.push_str(&permission.invocation_count.to_string());
    output.push_str(",\"persisted_rule_count\":");
    output.push_str(&permission.persisted_rule_count.to_string());
    field(output, "verdict", &permission.verdict);
    output.push_str(",\"persistent_scope\":");
    match &permission.persistent_scope {
        Some(scope) => {
            output.push('{');
            field(output, "matcher", &scope.matcher);
            field(output, "duration", &scope.duration);
            field(output, "project_root", &scope.project_root);
            field(output, "work_style", &scope.work_style);
            field(output, "child_inheritance", &scope.child_inheritance);
            field(output, "child_cap", &scope.child_cap);
            output.push('}');
        }
        None => output.push_str("null"),
    }
    output.push('}');
}

fn diff_json(output: &mut String, diff: &DiffEvidence) {
    output.push('{');
    raw_field(output, "patch_revision", &diff.patch_revision);
    field(output, "file_path", &diff.file_path);
    output.push_str(",\"hunks\":[");
    for (index, hunk) in diff.hunks.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push('{');
        raw_field(output, "hunk_id", &hunk.hunk_id);
        field(output, "file_path", &hunk.file_path);
        output.push_str(",\"old_start\":");
        output.push_str(&hunk.old_start.to_string());
        output.push_str(",\"old_lines\":");
        output.push_str(&hunk.old_lines.to_string());
        output.push_str(",\"new_start\":");
        output.push_str(&hunk.new_start.to_string());
        output.push_str(",\"new_lines\":");
        output.push_str(&hunk.new_lines.to_string());
        field(output, "state", &hunk.state);
        field(output, "content_hash", &hunk.content_hash);
        output.push('}');
    }
    output.push(']');
    field(output, "focused_hunk", &diff.focused_hunk);
    output.push_str(",\"included_count\":");
    output.push_str(&diff.included_count.to_string());
    string_array(output, "applied_hunks", &diff.applied_hunks);
    string_array(output, "excluded_hunks", &diff.excluded_hunks);
    output.push_str(",\"write_count\":");
    output.push_str(&diff.write_count.to_string());
    field(output, "working_tree_before", &diff.working_tree_before);
    field(output, "working_tree_after", &diff.working_tree_after);
    field(output, "git_index_before", &diff.git_index_before);
    field(output, "git_index_after", &diff.git_index_after);
    output.push('}');
}

fn string_array(output: &mut String, name: &str, values: &[String]) {
    output.push(',');
    quote(output, name);
    output.push_str(":[");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        quote(output, value);
    }
    output.push(']');
}

fn state_json(output: &mut String, state: &ScenarioState) {
    output.push('{');
    raw_field(output, "screen_id", &state.screen_id);
    field(output, "session_id", &state.session_id);
    output.push_str(",\"submitted_prompt\":");
    match &state.submitted_prompt {
        Some(prompt) => quote(output, prompt),
        None => output.push_str("null"),
    }
    field(output, "paused_activity_id", &state.paused_activity_id);
    output.push_str(",\"origin_stack\":[");
    for (index, origin) in state.origin_stack.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        quote(output, origin);
    }
    output.push(']');
    field(output, "draft", &state.draft);
    field(output, "transcript", &state.transcript);
    field(output, "queued_follow_up", &state.queued_follow_up);
    field(output, "scroll_anchor", &state.scroll_anchor);
    field(output, "selection", &state.selection);
    field(output, "focus", &state.focus);
    field(output, "composer_height", &state.composer_height);
    output.push_str(",\"pending_action\":");
    match &state.pending_action {
        Some(action) => quote(output, action),
        None => output.push_str("null"),
    }
    field(output, "return_policy", &state.return_policy);
    output.push_str(",\"virtual_tick\":");
    output.push_str(&state.virtual_tick.to_string());
    field(output, "state_hash", &state.state_hash);
    output.push('}');
}

fn raw_field(output: &mut String, name: &str, value: &str) {
    quote(output, name);
    output.push(':');
    quote(output, value);
}

#[cfg(feature = "png")]
pub fn write_scenario_png(
    destination: &std::path::Path,
    replay: &CaseReplay,
) -> Result<String, String> {
    use std::fs::OpenOptions;
    use std::io::Write as _;

    validate_png_destination(destination)?;
    let bytes = render_audit_png(&replay.audit)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("could not create temporary scenario PNG: {error}"))?;
    output
        .write_all(&bytes)
        .map_err(|error| format!("could not write temporary scenario PNG: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("could not finalize temporary scenario PNG: {error}"))?;
    Ok(sha256_hex(&bytes))
}

/// Rasterizes any validated typed audit with the same deterministic renderer
/// used by public scenario and gallery artifacts.
#[cfg(feature = "png")]
pub fn render_audit_png(audit: &AuditDocument) -> Result<Vec<u8>, String> {
    use image::{DynamicImage, ImageFormat, Rgb as Pixel, RgbImage};

    audit.validate()?;
    let canvas = &audit.styled_cells;
    let palette = crate::chrome::tokens::palette(crate::chrome::tokens::Theme::Dark);
    let canvas_color = palette.color(crate::chrome::tokens::ColorToken::Canvas);
    let mut image = RgbImage::from_pixel(
        crate::chrome::tokens::PNG_WIDTH,
        crate::chrome::tokens::PNG_HEIGHT,
        Pixel([canvas_color.red, canvas_color.green, canvas_color.blue]),
    );
    let (regular, bold) = crate::chrome::tokens::embedded_fonts()?;
    for cell in canvas {
        if cell.glyph == ' ' {
            continue;
        }
        let foreground = palette.color(cell.style.foreground);
        let cell_x = cell.column as u32 * 10;
        let cell_y = cell.row as u32 * 21;
        if crate::chrome::tokens::glyph_rendering(cell.glyph)
            == Some(crate::chrome::tokens::GlyphRendering::SyntheticElbow)
        {
            draw_synthetic_elbow(
                &mut image,
                cell_x,
                cell_y,
                Pixel([foreground.red, foreground.green, foreground.blue]),
            );
            continue;
        }
        let font = if cell.style.bold { &bold } else { &regular };
        let (metrics, bitmap) = font.rasterize(cell.glyph, 16.0);
        let base_x = cell.column as i32 * 10 + (10 - metrics.width as i32) / 2;
        let base_y = cell.row as i32 * 21 + (21 - metrics.height as i32) / 2;
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let alpha = bitmap[y * metrics.width + x] as u16;
                if alpha == 0 {
                    continue;
                }
                let px = base_x + x as i32;
                let py = base_y + y as i32;
                if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                    continue;
                }
                let background = image.get_pixel(px as u32, py as u32).0;
                let blend = |foreground: u8, background: u8| {
                    ((foreground as u16 * alpha + background as u16 * (255 - alpha)) / 255) as u8
                };
                image.put_pixel(
                    px as u32,
                    py as u32,
                    Pixel([
                        blend(foreground.red, background[0]),
                        blend(foreground.green, background[1]),
                        blend(foreground.blue, background[2]),
                    ]),
                );
            }
        }
    }
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| format!("could not encode scenario PNG: {error}"))?;
    Ok(bytes)
}

/// Draws the approved U+23BF result elbow without consulting a system font.
/// The fixed two-pixel strokes match the 10-by-21 terminal cell geometry.
#[cfg(feature = "png")]
fn draw_synthetic_elbow(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
) {
    let put = |image: &mut image::RgbImage, x: u32, y: u32| {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    };
    let corner_y = cell_y + 13;
    for y in cell_y + 1..=corner_y {
        put(image, cell_x + 4, y);
        put(image, cell_x + 5, y);
    }
    for x in cell_x + 4..cell_x + 10 {
        put(image, x, corner_y);
        put(image, x, corner_y + 1);
    }
}

#[cfg(feature = "png")]
pub fn validate_png_destination(destination: &std::path::Path) -> Result<(), String> {
    if !destination.is_absolute() {
        return Err("scenario PNG needs an explicit absolute temporary path".to_owned());
    }
    if destination
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("png")
    {
        return Err("scenario PNG path must end in .png".to_owned());
    }
    if destination.exists() {
        return Err("scenario PNG destination must be a new temporary path".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "scenario PNG destination needs a parent directory".to_owned())?
        .canonicalize()
        .map_err(|error| format!("scenario PNG parent is unavailable: {error}"))?;
    let resolved = parent.join(
        destination
            .file_name()
            .ok_or_else(|| "scenario PNG destination needs a file name".to_owned())?,
    );
    for root in [
        "/projects/cortex-code/plans/assets/mockups-v3",
        "/projects/cortex-code/plans/assets/mockups-v3-gates",
    ] {
        let root = std::path::Path::new(root)
            .canonicalize()
            .map_err(|error| format!("could not resolve protected scenario root: {error}"))?;
        if resolved.starts_with(root) {
            return Err(
                "scenario PNG destination must be a temporary path outside canonical and gate roots"
                    .to_owned(),
            );
        }
    }
    Ok(())
}
