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
    pub conversation_id: String,
    pub transcript_id: String,
    pub turn_id: String,
    pub user_task: String,
    pub visible_origin: String,
    pub session_id: String,
    pub submitted_prompt: Option<String>,
    pub paused_activity_id: String,
    pub origin_stack: Vec<String>,
    pub draft: String,
    pub attachment: String,
    pub transcript: String,
    pub partial_response: String,
    pub queue: String,
    pub queued_follow_up: String,
    pub scroll_anchor: String,
    pub selection: String,
    pub focus: String,
    pub composer_height: String,
    pub pending_action: Option<String>,
    pub return_policy: String,
    pub work_style: String,
    pub pending_work_style: Option<String>,
    pub access_scope: String,
    pub requested_command: String,
    pub requested_working_directory: String,
    pub permission_request_status: String,
    pub staging_hunk_states: [String; 2],
    pub focused_hunk: String,
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
    pub page_notation: String,
    pub command_sha256: String,
    pub page_sha256: String,
    pub byte_offset: usize,
    pub page_end: usize,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffStageState {
    pub hunk_states: [String; 2],
    pub focused_hunk: String,
    pub apply_failed: bool,
}

/// Returns the staging state represented by a public diff scenario.
///
/// Rendering consumes this projection so a replay and its TXT/PNG evidence
/// cannot drift apart through a second set of hard-coded staging labels.
pub(crate) fn diff_stage_state(case_id: &str) -> DiffStageState {
    let (hunk_states, focused_hunk, apply_failed) = match case_id {
        "staging-include-hunk" => (["Included", "Included"], "hunk.retry-test", false),
        "staging-exclude-hunk" => (["Excluded", "Pending"], "hunk.retry-guard", false),
        "staging-include-all" | "include-all" | "apply-all" | "diff-apply-all" => {
            (["Included", "Included"], "hunk.retry-guard", false)
        }
        "diff-review" | "hunk.retry-test" => (["Included", "Pending"], "hunk.retry-test", false),
        "include" => (["Included", "Included"], "hunk.retry-test", false),
        "exclude" => (["Excluded", "Pending"], "hunk.retry-guard", false),
        "staging-apply-partial-failure"
        | "staging-apply-failure"
        | "staging-finish-failed"
        | "staging-partial-apply-failure"
        | "staging-partial-failure"
        | "apply-partial-failure" => (["Included", "Pending"], "hunk.retry-guard", true),
        _ => (["Included", "Pending"], "hunk.retry-guard", false),
    };
    DiffStageState {
        hunk_states: hunk_states.map(str::to_owned),
        focused_hunk: focused_hunk.to_owned(),
        apply_failed,
    }
}

pub(crate) fn diff_stage_state_for_fixture(fixture_id: &str) -> DiffStageState {
    let case_id = fixture_id
        .strip_prefix("case.")
        .and_then(|value| value.split(".tick.").next())
        .unwrap_or(fixture_id);
    diff_stage_state(case_id)
}

pub(crate) fn diff_evidence_for_fixture(fixture_id: &str) -> Option<DiffEvidence> {
    let case_id = fixture_id
        .strip_prefix("case.")
        .and_then(|value| value.split(".tick.").next())
        .unwrap_or(fixture_id);
    let after = match case_id {
        "diff-review" => diff_review().1,
        "diff-apply-all" => diff_apply_all().1,
        "diff-cancel" => diff_cancel().1,
        "staging-finish" => staging_finish().1,
        "staging-include-hunk" => staging_include_hunk().1,
        "staging-exclude-hunk" => staging_exclude_hunk().1,
        "staging-include-all" => staging_include_all().1,
        "staging-apply-partial-failure"
        | "staging-apply-failure"
        | "staging-finish-failed"
        | "staging-partial-apply-failure"
        | "staging-partial-failure"
        | "apply-partial-failure" => staging_apply_partial_failure().1,
        _ if case_id.starts_with("staging-") => staging_state("initial", 1),
        _ => return None,
    };
    diff_evidence(case_id, &after)
}

impl PermissionEvidence {
    pub fn reconstructed_bytes(&self) -> Vec<u8> {
        decode_escaped_bytes(&self.display_notation)
    }

    pub fn reconstructed_page_bytes(&self) -> Vec<u8> {
        decode_escaped_bytes(&self.page_notation)
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
        if ((self.before.session_id != self.after.session_id
            || self.before.paused_activity_id != self.after.paused_activity_id)
            && self.case_id != "session-resume-latest")
            || (self.case_id != "flow2-submit-bridge"
                && self.before.submitted_prompt != self.after.submitted_prompt)
        {
            return Err(
                "scenario transition did not preserve its session, submitted prompt, or paused activity identity"
                    .to_owned(),
            );
        }
        if matches!(
            self.case_id.as_str(),
            "streaming-interrupt"
                | "tool-running-interrupt"
                | "tool-running-complete"
                | "tool-success-continue"
                | "flow2-submit-bridge"
                | "permission-allow-once"
                | "diff-review"
                | "staging-finish"
        ) {
            for (name, before, after) in [
                (
                    "conversation_id",
                    &self.before.conversation_id,
                    &self.after.conversation_id,
                ),
                (
                    "transcript_id",
                    &self.before.transcript_id,
                    &self.after.transcript_id,
                ),
                (
                    "session_id",
                    &self.before.session_id,
                    &self.after.session_id,
                ),
                ("turn_id", &self.before.turn_id, &self.after.turn_id),
                ("user_task", &self.before.user_task, &self.after.user_task),
                (
                    "visible_origin",
                    &self.before.visible_origin,
                    &self.after.visible_origin,
                ),
            ] {
                if before != after {
                    return Err(format!(
                        "storyboard identity changed without a bridge: {name}"
                    ));
                }
            }
        }
        if self.case_id.starts_with("work-style-") {
            let Some(work_style) = self.audit.work_style.as_ref() else {
                return Err("work-style scenario must emit one work_style audit object".to_owned());
            };
            if work_style.applied != self.after.work_style
                || work_style.pending != self.after.pending_work_style
                || work_style.access_scope != self.after.access_scope
                || self
                    .audit
                    .events
                    .iter()
                    .filter(|event| event.identity.family == "work-style")
                    .count()
                    != 1
            {
                return Err(
                    "work-style scenario audit does not reconcile the visible resulting state"
                        .to_owned(),
                );
            }
            for field in ["work_style", "pending_work_style", "access_scope"] {
                if !self.preserved_field_hashes.contains_key(field) {
                    return Err(format!(
                        "work-style scenario is missing {field} preservation hashes"
                    ));
                }
            }
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
                    && self.attempts[0].count == 0)
                && !(self.case_id.starts_with("session-") && self.attempts[0].count == 0))
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
                || permission.page_sha256 != sha256_hex(&permission.reconstructed_page_bytes())
            {
                return Err(
                    "permission evidence does not reconstruct exact execution bytes".to_owned(),
                );
            }
            if permission.execution_working_directory != "~/project"
                && permission.execution_working_directory != "~/other-project"
            {
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
                || permission.page_end < permission.byte_offset
                || permission.page_end > permission.visible_total_length
                || permission.page_end
                    != permission.byte_offset + permission.reconstructed_page_bytes().len()
                || permission.at_boundary
                    != (permission.byte_offset == 0
                        || permission.page_end == permission.visible_total_length)
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
        "permission-shell-long" | "permission-shell-long-page-0" => {
            permission_shell_long_page(0, 44)
        }
        "permission-shell-long-page-44" | "permission-shell-long-middle" => {
            permission_shell_long_page(44, 88)
        }
        "permission-shell-long-page-88" | "permission-shell-long-final" => {
            permission_shell_long_page(44, 88)
        }
        "permission-shell-long-return" => permission_shell_long_return(),
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
        "staging-apply-partial-failure"
        | "staging-apply-failure"
        | "staging-finish-failed"
        | "staging-partial-apply-failure" => staging_apply_partial_failure(),
        "staging-cancel" | "staging-back" => staging_cancel(),
        "onboarding-complete" => onboarding_complete(),
        "onboarding-skip" => onboarding_skip(),
        "onboarding-back" => onboarding_back(),
        "onboarding-existing-cancel" => onboarding_existing_cancel(),
        "onboarding-unusable-model" => onboarding_unusable_model(),
        "model-picker-apply" => model_picker_apply(),
        "model-picker-cancel" => model_picker_cancel(),
        "model-picker-connect" => model_picker_connect(),
        "work-style-unchanged" => work_style_unchanged(),
        "work-style-access-pending" => work_style_access_pending(),
        "work-style-access-confirm" => work_style_access_confirm(),
        "work-style-access-cancel" => work_style_access_cancel(),
        "work-style-access-refused" => work_style_access_refused(),
        "work-style-refusal-choose-unchanged" => work_style_refusal_choose_unchanged(),
        "work-style-refusal-return" => work_style_refusal_return(),
        "session-preview-open" => session_preview_open(),
        "session-selected-docs-preview" => session_selected_docs_preview(),
        "session-preview-back" => session_preview_back(),
        "session-timeline-open" => session_timeline_open(),
        "session-timeline-back" => session_timeline_back(),
        "session-resume-latest" => session_resume_latest(),
        "session-no-sessions-open" => session_no_sessions_open(),
        "session-no-sessions" => session_no_sessions(),
        "session-no-matches-filter" => session_no_matches_filter(),
        "session-no-matches" => session_no_matches(),
        _ => return Err(format!("unknown named case: {case_id}")),
    };
    let mut preserved_field_hashes = preserved_hashes(&before, &after);
    if matches!(case_id, "onboarding-complete" | "onboarding-skip") {
        preserved_field_hashes.insert(
            "pending_configuration".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.selection.as_bytes()),
                after: sha256_hex(before.selection.as_bytes()),
            },
        );
    }
    if case_id == "file-mention-insert" {
        preserved_field_hashes.insert(
            "draft_surrounding".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.draft.replacen(MENTION_FRAGMENT, "", 1).as_bytes()),
                after: sha256_hex(after.draft.replacen(INSERTED_FILE, "", 1).as_bytes()),
            },
        );
    }
    if case_id.starts_with("session-") {
        preserved_field_hashes.insert(
            "source_history".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b"retry-policy-review-history-v1"),
                after: sha256_hex(b"retry-policy-review-history-v1"),
            },
        );
    }
    let audit_fixture = if case_id == "flow2-submit-bridge" {
        "flow2.input-and-approval.stable".to_owned()
    } else if case_id == "staging-finish" {
        "case.staging-finish.tick.1.flow2.input-and-approval.stable".to_owned()
    } else if case_id.starts_with("work-style-") {
        match case_id {
            "work-style-unchanged" => "flow3-work-style-unchanged",
            "work-style-access-pending" => "flow3-work-style-access",
            "work-style-access-confirm" => "flow3-work-style-access",
            "work-style-access-cancel" => "flow3-work-style-current-only",
            "work-style-access-refused" => "flow3-work-style-refused",
            "work-style-refusal-choose-unchanged" => "flow3-work-style-refusal-unchanged",
            "work-style-refusal-return" => "flow3-work-style-refused-return",
            _ => unreachable!(),
        }
        .to_owned()
    } else {
        format!("case.{case_id}.tick.{tick}")
    };
    let audit_fixture = if matches!(
        case_id,
        "staging-apply-partial-failure"
            | "staging-apply-failure"
            | "staging-finish-failed"
            | "staging-partial-apply-failure"
    ) {
        "case.staging-apply-partial-failure.tick.1".to_owned()
    } else {
        audit_fixture
    };
    let audit = if after.screen_id == "connect-provider"
        && matches!(
            case_id,
            "model-picker-connect" | "onboarding-unusable-model"
        ) {
        blocked_model_recovery_audit(case_id, &after)?
    } else {
        audit_screen(&after.screen_id, &audit_fixture)?
    };
    let diff = diff_evidence(case_id, &after);
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
        diff,
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
    apply_flow2_identity(&mut before);
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

fn onboarding_complete() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "onboarding",
        vec!["onboarding"],
        "Start with a safe model.",
        "first-run",
        "recommended-claude-sonnet-4-5",
        "onboarding-default",
        Some("pending.model-choice"),
        "return-to-onboarding",
        0,
    );
    let mut after = state(
        "empty",
        vec!["onboarding"],
        "",
        "first-run",
        "recommended-claude-sonnet-4-5",
        "composer",
        None,
        "return-to-onboarding",
        1,
    );
    after.draft = "Ask Code to start".to_owned();
    after.transcript = "Setup complete with a safe default. Start a new conversation.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.onboarding.apply-default".to_owned(),
        label: "Continue".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the recommended model is applied before entering the usable empty conversation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.onboarding.apply-default.1".to_owned(),
        owner_id: "pending.model-choice".to_owned(),
        count: 1,
        outcome: "applied-safe-default".to_owned(),
    };
    let effect = simulated_effect(
        "effect.onboarding.apply-default",
        "configuration",
        "safe-default-applied",
        "the recommended model was applied once before entering empty",
    );
    (before, after, action, transition, attempt, effect)
}

fn onboarding_skip() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (before, mut after, _, _, _, _) = onboarding_complete();
    after.transcript =
        "Setup skipped. A safe default is ready for your first conversation.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.onboarding.skip".to_owned(),
        label: "Skip with this safe default".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "skip chooses the safe default and enters the usable empty conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.onboarding.skip.1".to_owned(),
        owner_id: "pending.model-choice".to_owned(),
        count: 1,
        outcome: "skipped-with-safe-default".to_owned(),
    };
    let effect = simulated_effect(
        "effect.onboarding.skip",
        "configuration",
        "safe-default-applied",
        "skip applied the safe default once",
    );
    (before, after, action, transition, attempt, effect)
}

fn onboarding_back() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "onboarding",
        vec!["onboarding"],
        "Start with a safe model.",
        "first-run",
        "recommended-claude-sonnet-4-5",
        "onboarding-default",
        Some("pending.model-choice"),
        "return-to-onboarding",
        0,
    );
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.onboarding.back".to_owned(),
        label: "Back".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "first-run back remains at the preceding onboarding step and applies nothing"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.onboarding.back.1".to_owned(),
        owner_id: "pending.model-choice".to_owned(),
        count: 0,
        outcome: "returned-to-onboarding".to_owned(),
    };
    let effect = simulated_effect(
        "effect.onboarding.back",
        "configuration",
        "no-change",
        "first-run back applied no model",
    );
    (before, after, action, transition, attempt, effect)
}

fn onboarding_existing_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "onboarding",
        vec!["chat-turn", "onboarding"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "recommended-claude-sonnet-4-5",
        "onboarding-default",
        Some("pending.model-choice"),
        "return-to-chat-turn",
        0,
    );
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.onboarding.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel restores the existing conversation without applying pending setup".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.onboarding.cancel.1".to_owned(),
        owner_id: "pending.model-choice".to_owned(),
        count: 0,
        outcome: "cancelled-without-change".to_owned(),
    };
    let effect = simulated_effect(
        "effect.onboarding.cancel",
        "configuration",
        "no-change",
        "existing-origin onboarding cancellation changed no applied configuration",
    );
    (before, after, action, transition, attempt, effect)
}

fn onboarding_unusable_model() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "model-picker",
        vec!["onboarding", "model-picker"],
        "Start with a safe model.",
        "first-run",
        "gpt-5-2-codex",
        "model-selection",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-onboarding",
        0,
    );
    apply_flow3_onboarding_identity(&mut before);
    let mut after = state(
        "connect-provider",
        vec!["onboarding", "model-picker"],
        "Start with a safe model.",
        "first-run",
        "gpt-5-2-codex",
        "provider-choice",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        1,
    );
    apply_flow3_onboarding_identity(&mut after);
    let action = ActionEvidence {
        action_id: "action.model-picker.connect".to_owned(),
        label: "Connect provider".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the unavailable onboarding choice opens connection without losing setup state"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.onboarding.connect.1".to_owned(),
        owner_id: "pending.model.gpt-5-2-codex".to_owned(),
        count: 0,
        outcome: "connection-requested".to_owned(),
    };
    let effect = simulated_effect(
        "effect.onboarding.connect",
        "provider-connection",
        "no-model-switch",
        "onboarding connection requested without applying the unavailable model",
    );
    (before, after, action, transition, attempt, effect)
}

fn model_picker_apply() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "model-picker",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "claude-opus-4-8",
        "model-selection",
        Some("pending.model.claude-opus-4-8"),
        "return-to-chat-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "claude-opus-4-8",
        "composer",
        None,
        "return-to-chat-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.model-picker.apply".to_owned(),
        label: "Apply model".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the selected model applies once and restores the exact conversation origin"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.model-picker.apply.1".to_owned(),
        owner_id: "pending.model.claude-opus-4-8".to_owned(),
        count: 1,
        outcome: "applied".to_owned(),
    };
    let effect = simulated_effect(
        "effect.model-picker.apply",
        "configuration",
        "model-applied",
        "the selected model was applied once without changing the draft or conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn model_picker_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "model-picker",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "claude-opus-4-8",
        "model-selection",
        Some("pending.model.claude-opus-4-8"),
        "return-to-chat-turn",
        0,
    );
    let after = state(
        "chat-turn",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "claude-sonnet-4-5",
        "composer",
        None,
        "return-to-chat-turn",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.model-picker.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel discards the pending model and restores the exact conversation origin"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.model-picker.cancel.1".to_owned(),
        owner_id: "pending.model.claude-opus-4-8".to_owned(),
        count: 0,
        outcome: "cancelled-without-change".to_owned(),
    };
    let effect = simulated_effect(
        "effect.model-picker.cancel",
        "configuration",
        "no-change",
        "model cancellation changed no applied configuration",
    );
    (before, after, action, transition, attempt, effect)
}

fn model_picker_connect() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "model-picker",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "gpt-5-2-codex",
        "model-selection",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-chat-turn",
        0,
    );
    let after = state(
        "connect-provider",
        vec!["chat-turn", "model-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "gpt-5-2-codex",
        "provider-choice",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        1,
    );
    let action = ActionEvidence {
        action_id: "action.model-picker.connect".to_owned(),
        label: "Connect provider".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the unavailable model opens connection while retaining the pending model and draft"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.model-picker.connect.1".to_owned(),
        owner_id: "pending.model.gpt-5-2-codex".to_owned(),
        count: 0,
        outcome: "connection-requested".to_owned(),
    };
    let effect = simulated_effect(
        "effect.model-picker.connect",
        "provider-connection",
        "no-model-switch",
        "connection was requested without applying the unavailable model",
    );
    (before, after, action, transition, attempt, effect)
}

fn blocked_model_recovery_audit(
    case_id: &str,
    state: &ScenarioState,
) -> Result<AuditDocument, String> {
    let fixture_id = format!("case.{case_id}.tick.{}", state.virtual_tick);
    let origin = if case_id == "onboarding-unusable-model" {
        "Onboarding"
    } else {
        "Current conversation"
    };
    let return_policy = if case_id == "onboarding-unusable-model" {
        "Return to onboarding"
    } else {
        "Return to model choices"
    };
    crate::audit::blocked_model_recovery_audit(
        &fixture_id,
        &state.visible_origin,
        origin,
        return_policy,
    )
}

fn work_style_unchanged() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "agent-picker",
        vec!["chat-turn", "agent-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "quick-implementation",
        "work-style-selection",
        Some("pending.work-style.quick"),
        "return-to-chat-turn",
        0,
    );
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.work_style = "Quick implementation".to_owned();
    after.access_scope = "asks before editing project files".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.agent-picker.apply-unchanged".to_owned(),
        label: "Use Quick implementation".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the unchanged-access work style applies directly and returns to the paused conversation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.agent-picker.apply-unchanged.1".to_owned(),
        owner_id: "pending.work-style.quick".to_owned(),
        count: 1,
        outcome: "applied-without-access-change".to_owned(),
    };
    let effect = simulated_effect(
        "effect.agent-picker.apply-unchanged",
        "configuration",
        "work-style-applied",
        "Quick implementation replaced the pending style while project access stayed unchanged",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_access_pending() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "agent-picker",
        vec!["chat-turn", "agent-picker"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "autonomous-changes",
        "work-style-selection",
        Some("pending.work-style.autonomous"),
        "return-to-chat-turn",
        0,
    );
    let mut after = before.clone();
    after.screen_id = "mode-switch".to_owned();
    after.focus = "access-confirmation".to_owned();
    after.pending_work_style = Some("Autonomous changes".to_owned());
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.agent-picker.review-access".to_owned(),
        label: "Review access change".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the access-changing work style opens one nested confirmation before changing access"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.agent-picker.review-access.1".to_owned(),
        owner_id: "pending.work-style.autonomous".to_owned(),
        count: 0,
        outcome: "awaiting-access-confirmation".to_owned(),
    };
    let effect = simulated_effect(
        "effect.agent-picker.review-access",
        "configuration",
        "pending-confirmation",
        "no access changed while Autonomous changes waits for explicit confirmation",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_access_confirm() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "mode-switch",
        vec!["chat-turn", "agent-picker", "mode-switch"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "autonomous-changes",
        "access-confirmation",
        Some("pending.work-style.autonomous"),
        "return-to-chat-turn",
        0,
    );
    before.pending_work_style = Some("Autonomous changes".to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.work_style = "Autonomous changes".to_owned();
    after.pending_work_style = None;
    after.access_scope =
        "can edit project files without asking each time, in this project".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mode-switch.confirm".to_owned(),
        label: "Apply access change".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path:
            "confirmation applies the one work_style value and returns to the paused conversation"
                .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mode-switch.confirm.1".to_owned(),
        owner_id: "pending.work-style.autonomous".to_owned(),
        count: 1,
        outcome: "access-change-applied".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mode-switch.confirm",
        "configuration",
        "work-style-and-access-applied",
        "Autonomous changes now applies only within this project until another style is chosen",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_access_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "mode-switch",
        vec!["chat-turn", "agent-picker", "mode-switch"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "autonomous-changes",
        "access-confirmation",
        Some("pending.work-style.autonomous"),
        "return-to-chat-turn",
        0,
    );
    before.pending_work_style = Some("Autonomous changes".to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "agent-picker".to_owned();
    after.selection = "careful-edits".to_owned();
    after.focus = "work-style-selection".to_owned();
    after.pending_action = None;
    after.pending_work_style = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mode-switch.cancel".to_owned(),
        label: "Keep Careful edits".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path:
            "cancel discards the pending access change and returns to the work-style choice with Careful edits still applied"
                .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mode-switch.cancel.1".to_owned(),
        owner_id: "pending.work-style.autonomous".to_owned(),
        count: 0,
        outcome: "access-change-cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mode-switch.cancel",
        "configuration",
        "no-change",
        "the pending Autonomous changes selection was discarded without changing work style or access",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_access_refused() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "mode-switch",
        vec!["chat-turn", "agent-picker", "mode-switch"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "autonomous-changes",
        "access-confirmation",
        Some("pending.work-style.autonomous"),
        "return-to-chat-turn",
        0,
    );
    before.pending_work_style = Some("Autonomous changes".to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "mode-switch-refused".to_owned();
    after.focus = "refusal-recovery".to_owned();
    after.pending_action = None;
    after.pending_work_style = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mode-switch-refused.choose-safer".to_owned(),
        label: "Choose Quick implementation · same access".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the refused access change becomes a safe recovery bridge with Careful edits still active"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mode-switch-refused.1".to_owned(),
        owner_id: "pending.work-style.autonomous".to_owned(),
        count: 0,
        outcome: "access-change-refused".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mode-switch-refused",
        "configuration",
        "no-change",
        "approval was refused and the current work style and access remained active",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_refusal_choose_unchanged() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "mode-switch-refused",
        vec!["chat-turn", "agent-picker", "mode-switch"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "careful-edits",
        "refusal-recovery",
        None,
        "return-to-chat-turn",
        0,
    );
    let mut after = before.clone();
    after.screen_id = "agent-picker".to_owned();
    after.selection = "quick-implementation".to_owned();
    after.focus = "work-style-selection".to_owned();
    after.pending_action = Some("pending.work-style.quick".to_owned());
    after.pending_work_style = Some("Quick implementation".to_owned());
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mode-switch-refused.choose-unchanged".to_owned(),
        label: "Choose Quick implementation · same access".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path:
            "the refusal recovery opens Quick implementation as an unchanged-access choice while Careful edits remains applied"
                .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mode-switch-refused.choose-unchanged.1".to_owned(),
        owner_id: "pending.work-style.quick".to_owned(),
        count: 0,
        outcome: "unchanged-access-choice-opened".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mode-switch-refused.choose-unchanged",
        "configuration",
        "no-change",
        "the rejected escalation stayed closed while Quick implementation became a pending same-access choice",
    );
    (before, after, action, transition, attempt, effect)
}

fn work_style_refusal_return() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = state(
        "mode-switch-refused",
        vec!["chat-turn", "agent-picker", "mode-switch"],
        "Continue the retry conversation.",
        "turn-retry-review",
        "careful-edits",
        "refusal-recovery",
        None,
        "return-to-chat-turn",
        0,
    );
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mode-switch-refused.return".to_owned(),
        label: "Return to work".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path:
            "return restores the paused conversation with Careful edits and ask-before-editing access unchanged"
                .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mode-switch-refused.return.1".to_owned(),
        owner_id: "pending.work-style.autonomous".to_owned(),
        count: 0,
        outcome: "returned-without-change".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mode-switch-refused.return",
        "configuration",
        "no-change",
        "return changed no configuration and restored the paused conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn session_preview_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "saved-conversations",
        Some("pending.session-picker"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.choose",
        "Choose Retry policy review",
        "direct",
        "choosing a row opens its read-only preview without changing the current session or source history",
        "attempt.session-picker.preview.1",
        0,
        "preview-opened",
        "effect.session-picker.preview",
        "session-preview",
        "read-only",
        "the selected row gained one preview without changing current or source session state",
    )
}

fn session_preview_back() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        None,
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.preview-back",
        "Back to session rows",
        "cancel",
        "preview Back restores the same picker filter, focused row, scroll position, and source history",
        "attempt.session-picker.preview-back.1",
        0,
        "preview-closed",
        "effect.session-picker.preview-back",
        "session-preview",
        "no-change",
        "preview Back closed read-only detail and preserved the current session",
    )
}

fn session_selected_docs_preview() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "session-picker",
        "all",
        "document-release-steps",
        Some("preview.document-release-steps"),
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.choose",
        "Choose Document release steps",
        "direct",
        "choosing another row replaces only the read-only destination preview",
        "attempt.session-picker.preview-docs.1",
        0,
        "preview-opened",
        "effect.session-picker.preview-docs",
        "session-preview",
        "read-only",
        "the selected docs row gained one preview without changing current or source session state",
    )
}

fn session_timeline_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "timeline",
        "all",
        "retry-policy-review",
        Some("timeline.retry-policy-review"),
        "return-to-session-preview",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.timeline",
        "View timeline",
        "direct",
        "timeline opens as a read-only temporary detail tied to the selected preview",
        "attempt.session-picker.timeline.1",
        0,
        "timeline-opened",
        "effect.session-picker.timeline",
        "session-history",
        "read-only",
        "timeline opened without changing the current session or source history",
    )
}

fn session_timeline_back() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "timeline",
        "all",
        "retry-policy-review",
        Some("timeline.retry-policy-review"),
        "return-to-session-preview",
        0,
    );
    let after = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.timeline.back",
        "Back to session preview",
        "cancel",
        "timeline Back restores the same preview, filter, focus, scroll position, and conversation origin",
        "attempt.timeline.back.1",
        0,
        "preview-restored",
        "effect.timeline.back",
        "session-history",
        "no-change",
        "timeline Back restored the exact read-only session preview",
    )
}

fn session_resume_latest() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        0,
    );
    let mut after = session_state(
        "chat-turn",
        "all",
        "retry-policy-review",
        None,
        "return-to-session-picker",
        1,
    );
    after.session_id = "session.retry-policy-review".to_owned();
    after.paused_activity_id = "activity.retry-policy-review.latest".to_owned();
    after.transcript =
        "Retry policy review\nTests pass; review is ready at the latest stable state.".to_owned();
    after.draft = "Add the final review note.".to_owned();
    after.focus = "composer".to_owned();
    after.scroll_anchor = "retry-policy-review-latest".to_owned();
    after.state_hash = state_hash(&after);
    session_case(
        before,
        after,
        "action.session-picker.resume",
        "Resume latest",
        "direct",
        "resume makes the selected latest stable conversation current without forking or changing source history",
        "attempt.session-picker.resume.1",
        1,
        "selected-session-resumed",
        "effect.session-picker.resume",
        "session-current-selection",
        "selected-session-resumed",
        "the selected conversation became current exactly once at its latest stable state",
    )
}

fn session_no_sessions() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "none",
        "no-saved-conversations",
        Some("empty.session-list"),
        "return-to-chat-turn",
        0,
    );
    let mut after = session_state(
        "empty",
        "none",
        "new-conversation",
        None,
        "return-to-chat-turn",
        1,
    );
    after.focus = "composer".to_owned();
    after.state_hash = state_hash(&after);
    session_case(
        before,
        after,
        "action.session-picker.start",
        "Start a conversation",
        "direct",
        "the local empty recovery opens a usable conversation while preserving the origin draft",
        "attempt.session-picker.start.1",
        1,
        "new-conversation-opened",
        "effect.session-picker.start",
        "conversation-start",
        "started-once",
        "one empty conversation opened without modifying the paused origin",
    )
}

fn session_no_sessions_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = session_state(
        "chat-turn",
        "none",
        "resume-sessions",
        None,
        "return-to-chat-turn",
        0,
    );
    before.focus = "composer".to_owned();
    before.state_hash = state_hash(&before);
    let after = session_state(
        "session-picker",
        "none",
        "no-saved-conversations",
        Some("empty.session-list"),
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.chat-turn.open-sessions",
        "Resume a conversation",
        "direct",
        "opening session choice with no saved conversations stays local to the exact conversation origin",
        "attempt.session-picker.open-empty.1",
        0,
        "empty-session-list-opened",
        "effect.session-picker.open-empty",
        "session-picker-open",
        "read-only",
        "the local empty state opened without changing the current session or draft",
    )
}

fn session_no_matches_filter() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        Some("preview.retry-policy-review"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "session-picker",
        "billing",
        "no-filter-matches",
        Some("filter.billing"),
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.filter",
        "Filter saved conversations",
        "direct",
        "a nonmatching query keeps the query visible and changes only the local session rows",
        "attempt.session-picker.filter.1",
        0,
        "no-matches",
        "effect.session-picker.filter",
        "session-filter",
        "read-only",
        "the filter produced no matches without changing current or source session state",
    )
}

fn session_no_matches() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = session_state(
        "session-picker",
        "billing",
        "no-filter-matches",
        Some("filter.billing"),
        "return-to-chat-turn",
        0,
    );
    let after = session_state(
        "session-picker",
        "all",
        "retry-policy-review",
        None,
        "return-to-chat-turn",
        1,
    );
    session_case(
        before,
        after,
        "action.session-picker.clear-filter",
        "Clear filter",
        "direct",
        "clear filter restores the saved-session rows with the origin and source history unchanged",
        "attempt.session-picker.clear-filter.1",
        0,
        "filter-cleared",
        "effect.session-picker.clear-filter",
        "session-filter",
        "no-session-change",
        "the local filter cleared without changing current or source session state",
    )
}

#[allow(clippy::too_many_arguments)]
fn session_state(
    screen_id: &str,
    filter: &str,
    selection: &str,
    pending_action: Option<&str>,
    return_policy: &str,
    virtual_tick: u32,
) -> ScenarioState {
    let mut state = state(
        screen_id,
        vec!["chat-turn", "session-picker"],
        "Continue the current draft while choosing a saved conversation.",
        "turn-current-latest",
        selection,
        if screen_id == "timeline" {
            "timeline-point"
        } else {
            "session-row"
        },
        pending_action,
        return_policy,
        virtual_tick,
    );
    state.transcript = "Current conversation · Retry behavior before session choice.".to_owned();
    state.queued_follow_up = format!("session-filter={filter}");
    state.work_style = "Careful edits".to_owned();
    state.access_scope = "asks before editing project files".to_owned();
    state.state_hash = state_hash(&state);
    state
}

#[allow(clippy::too_many_arguments)]
fn session_case(
    before: ScenarioState,
    after: ScenarioState,
    action_id: &str,
    label: &str,
    kind: &str,
    path: &str,
    attempt_id: &str,
    attempt_count: u32,
    outcome: &str,
    effect_id: &str,
    effect_kind: &str,
    effect_status: &str,
    effect_detail: &str,
) -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let action = ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: attempt_id.to_owned(),
        owner_id: "selected-session-preview".to_owned(),
        count: attempt_count,
        outcome: outcome.to_owned(),
    };
    let effect = simulated_effect(effect_id, effect_kind, effect_status, effect_detail);
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

fn permission_shell_long_page(
    before_offset: usize,
    after_offset: usize,
) -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (mut before, _, _, _, _, _) = permission_shell_short();
    before.selection = format!("long-command-page-{before_offset}");
    before.focus = "command-detail".to_owned();
    before.pending_action = Some("pending.shell.project-check".to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = format!("long-command-page-{after_offset}");
    after.focus = "command-detail".to_owned();
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
        path: format!(
            "command detail moves from byte offset {before_offset} to {after_offset} without changing execution bytes"
        ),
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

fn permission_shell_long_return() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (mut before, _, _, _, _, _) = permission_shell_long_page(44, 88);
    before.selection = "long-command-page-88".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "permission-choice".to_owned();
    after.focus = "permission-choice".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.permission.command-return".to_owned(),
        label: "Return to permission choices".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "Return closes command detail and restores the same permission choices and focus"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.long-page.return.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 0,
        outcome: "returned-to-permission-choice".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.long-page.return",
        "permission-disclosure",
        "no-execution",
        "return from command detail did not execute the command",
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
    let (mut before, _, _, _, _, _) = permission_persistent_scope();
    before.screen_id = "permission-shell".to_owned();
    before.selection = "persistent-scope".to_owned();
    before.focus = "permission-choice".to_owned();
    before.pending_action = Some("pending.shell.project-check".to_owned());
    before.virtual_tick = 0;
    before.requested_command = "git diff -- src/net/retry_policy.rs".to_owned();
    before.requested_working_directory = "~/project".to_owned();
    before.permission_request_status = "existing rule is available".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "out-of-scope-request".to_owned();
    after.pending_action = Some("pending.shell.outside-scope".to_owned());
    after.requested_command = "git diff -- src/docs/release_notes.md".to_owned();
    after.requested_working_directory = "~/other-project".to_owned();
    after.permission_request_status = "new permission required".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
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
        "Add a short note about the auth refactor.",
        "assistant-partial-auth-refactor",
        "stop-reply",
        "streaming-interrupt",
        Some("active.reply.auth-refactor"),
        "return-to-paused-turn",
        0,
    );
    before.attachment = "auth-notes.md".to_owned();
    before.partial_response = "The refactor moves token checks into one shared session guard.\n\
It keeps sign-in, refresh, and sign-out on the same safety rule."
        .to_owned();
    before.transcript = before.partial_response.clone();
    before.queue =
        "1: Also list the tests that should cover the change. [attachment: auth-notes.md]"
            .to_owned();
    before.queued_follow_up = before.queue.clone();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "chat-turn",
        vec!["home", "chat-turn", "streaming"],
        "Add a short note about the auth refactor.",
        "assistant-partial-auth-refactor",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    after.transcript = before.transcript.clone();
    after.attachment = before.attachment.clone();
    after.partial_response = before.partial_response.clone();
    after.queue = before.queue.clone();
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
    let before = staging_state("initial", 0);
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

fn staging_apply_partial_failure() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = staging_state("apply-partial-failure", 0);
    let after = staging_state("apply-partial-failure", 1);
    let action = diff_action("action.staging.finish", "Finish");
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        "Finish rejected the stale patch before writing so every selected byte stayed unchanged",
    );
    let attempt = diff_attempt("attempt.staging.finish.1", "apply-failed", 1);
    let effect = simulated_effect(
        "effect.staging.finish",
        "patch-application",
        "apply-failed-no-write",
        "stale patch validation failed before the atomic write boundary; no partial mutation occurred",
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
    let before = staging_state("initial", 0);
    let after = staging_state(name, 1);
    let action = diff_action(&format!("action.staging.{}", name), label);
    let transition = diff_transition(
        &action,
        &before,
        &after,
        "direct",
        &format!(
            "{} changes only the focused hunk {focused_hunk} and writes nothing",
            label
        ),
    );
    let attempt = diff_attempt(&format!("attempt.staging.{name}.1"), "selection-updated", 0);
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
    apply_flow2_identity(&mut state);
    let staging = diff_stage_state(selection);
    state.staging_hunk_states = staging.hunk_states;
    state.focused_hunk = staging.focused_hunk;
    state.transcript = format!("You · {FLOW2_SUBMITTED_PROMPT}");
    state.state_hash = state_hash(&state);
    state
}

fn apply_flow2_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.retry-review".to_owned();
    state.transcript_id = "transcript.retry-review".to_owned();
    state.turn_id = "turn.retry-review.18".to_owned();
    state.user_task = "Check the retry path before we merge.".to_owned();
    state.visible_origin = "Input and approval · paused conversation".to_owned();
}

fn apply_flow3_onboarding_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.configuration".to_owned();
    state.transcript_id = "transcript.configuration".to_owned();
    state.session_id = "session.configuration.1".to_owned();
    state.turn_id = "turn.configuration.1".to_owned();
    state.user_task = "Choose a model and how Code should work.".to_owned();
    state.visible_origin = "Work configuration · onboarding".to_owned();
    state.state_hash = state_hash(state);
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
        conversation_id: "conversation.daily-work".to_owned(),
        transcript_id: "transcript.daily-work".to_owned(),
        session_id: "session.daily-work.1".to_owned(),
        turn_id: "turn.daily-work.18".to_owned(),
        user_task: "Find where the server applies the rate limit and explain the safe path."
            .to_owned(),
        visible_origin: "Daily work · current conversation".to_owned(),
        submitted_prompt: None,
        paused_activity_id: "activity.current-turn".to_owned(),
        origin_stack: origin_stack.into_iter().map(str::to_owned).collect(),
        draft: draft.to_owned(),
        attachment: String::new(),
        transcript: "The current conversation remains available.".to_owned(),
        partial_response: String::new(),
        queue: String::new(),
        queued_follow_up: String::new(),
        scroll_anchor: scroll_anchor.to_owned(),
        selection: selection.to_owned(),
        focus: focus.to_owned(),
        composer_height: "5-visual-rows".to_owned(),
        pending_action: pending_action.map(str::to_owned),
        return_policy: return_policy.to_owned(),
        work_style: "Careful edits".to_owned(),
        pending_work_style: None,
        access_scope: "asks before editing project files".to_owned(),
        requested_command: "printf '%s\\n' \"retry check\"".to_owned(),
        requested_working_directory: "~/project".to_owned(),
        permission_request_status: "permission needed".to_owned(),
        staging_hunk_states: ["Included".to_owned(), "Pending".to_owned()],
        focused_hunk: "hunk.retry-guard".to_owned(),
        virtual_tick,
        state_hash: String::new(),
    };
    state.state_hash = state_hash(&state);
    state
}

fn validate_state(state: &ScenarioState) -> Result<(), String> {
    if state.screen_id.trim().is_empty()
        || state.conversation_id.trim().is_empty()
        || state.transcript_id.trim().is_empty()
        || state.session_id.trim().is_empty()
        || state.turn_id.trim().is_empty()
        || state.user_task.trim().is_empty()
        || state.visible_origin.trim().is_empty()
        || state.paused_activity_id.trim().is_empty()
        || state.origin_stack.is_empty()
        || state.draft.is_empty()
        || state.scroll_anchor.is_empty()
        || state.selection.is_empty()
        || state.focus.is_empty()
        || state.composer_height.is_empty()
        || state.return_policy.is_empty()
        || state.requested_command.is_empty()
        || state.requested_working_directory.is_empty()
        || state.permission_request_status.is_empty()
        || state
            .staging_hunk_states
            .iter()
            .any(|state| state.is_empty())
        || state.focused_hunk.is_empty()
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
        || diff.included_count
            != diff
                .hunks
                .iter()
                .filter(|hunk| hunk.state == "Included")
                .count()
    {
        return Err(
            "diff evidence has incomplete identity, state, or index preservation".to_owned(),
        );
    }
    if diff.working_tree_before.len() != 64
        || diff.working_tree_after.len() != 64
        || diff.git_index_before.len() != 64
        || diff.git_index_after.len() != 64
    {
        return Err(
            "diff evidence must use exact SHA-256 working-tree and Git-index hashes".to_owned(),
        );
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
            "screen={}\nconversation={}\ntranscript_id={}\nsession={}\nturn={}\ntask={}\norigin={}\nsubmitted_prompt={}\npaused_activity={}\norigins={}\ndraft={}\nattachment={}\ntranscript={}\npartial_response={}\nqueue={}\nqueued_follow_up={}\nscroll={}\nselection={}\nfocus={}\ncomposer_height={}\npending={}\nreturn={}\nwork_style={}\npending_work_style={}\naccess_scope={}\nrequested_command={}\nrequested_working_directory={}\npermission_request_status={}\nstaging_hunks={}\nfocused_hunk={}\ntick={}",
            state.screen_id,
            state.conversation_id,
            state.transcript_id,
            state.session_id,
            state.turn_id,
            state.user_task,
            state.visible_origin,
            state.submitted_prompt.as_deref().unwrap_or(""),
            state.paused_activity_id,
            state.origin_stack.join(">"),
            state.draft,
            state.attachment,
            state.transcript,
            state.partial_response,
            state.queue,
            state.queued_follow_up,
            state.scroll_anchor,
            state.selection,
            state.focus,
            state.composer_height,
            state.pending_action.as_deref().unwrap_or(""),
            state.return_policy,
            state.work_style,
            state.pending_work_style.as_deref().unwrap_or(""),
            state.access_scope,
            state.requested_command,
            state.requested_working_directory,
            state.permission_request_status,
            state.staging_hunk_states.join(">"),
            state.focused_hunk,
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
            "conversation_id",
            before.conversation_id.as_str(),
            after.conversation_id.as_str(),
        ),
        (
            "transcript_id",
            before.transcript_id.as_str(),
            after.transcript_id.as_str(),
        ),
        (
            "session_id",
            before.session_id.as_str(),
            after.session_id.as_str(),
        ),
        ("turn_id", before.turn_id.as_str(), after.turn_id.as_str()),
        (
            "user_task",
            before.user_task.as_str(),
            after.user_task.as_str(),
        ),
        (
            "visible_origin",
            before.visible_origin.as_str(),
            after.visible_origin.as_str(),
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
            "attachment",
            before.attachment.as_str(),
            after.attachment.as_str(),
        ),
        (
            "transcript",
            before.transcript.as_str(),
            after.transcript.as_str(),
        ),
        (
            "partial_response",
            before.partial_response.as_str(),
            after.partial_response.as_str(),
        ),
        ("queue", before.queue.as_str(), after.queue.as_str()),
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
            "origin",
            &before.origin_stack.join(">"),
            &after.origin_stack.join(">"),
        ),
        (
            "reading_anchor",
            before.scroll_anchor.as_str(),
            after.scroll_anchor.as_str(),
        ),
        (
            "return_policy",
            before.return_policy.as_str(),
            after.return_policy.as_str(),
        ),
        (
            "work_style",
            before.work_style.as_str(),
            after.work_style.as_str(),
        ),
        (
            "pending_work_style",
            before.pending_work_style.as_deref().unwrap_or(""),
            after.pending_work_style.as_deref().unwrap_or(""),
        ),
        (
            "access_scope",
            before.access_scope.as_str(),
            after.access_scope.as_str(),
        ),
        (
            "selection",
            before.selection.as_str(),
            after.selection.as_str(),
        ),
        ("focus", before.focus.as_str(), after.focus.as_str()),
        ("patch_revision", "patch-retry-18", "patch-retry-18"),
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

fn diff_hunks(states: [String; 2]) -> Vec<DiffHunkEvidence> {
    vec![
        DiffHunkEvidence {
            hunk_id: "hunk.retry-guard".to_owned(),
            file_path: "src/net/retry_policy.rs".to_owned(),
            old_start: 42,
            old_lines: 2,
            new_start: 42,
            new_lines: 3,
            state: states[0].clone(),
            content_hash: sha256_hex(
                b"hunk.retry-guard\n-    return retry_after\n+    return retry_after + jitter",
            ),
        },
        DiffHunkEvidence {
            hunk_id: "hunk.retry-test".to_owned(),
            file_path: "src/net/retry_policy.rs".to_owned(),
            old_start: 88,
            old_lines: 2,
            new_start: 88,
            new_lines: 2,
            state: states[1].clone(),
            content_hash: sha256_hex(
                b"hunk.retry-test\n-assert_eq!(wait, 4)\n+assert_eq!(wait, 5)",
            ),
        },
    ]
}

fn diff_evidence(case_id: &str, after: &ScenarioState) -> Option<DiffEvidence> {
    if !matches!(
        case_id,
        "diff-review"
            | "diff-apply-all"
            | "diff-cancel"
            | "staging-include-hunk"
            | "staging-exclude-hunk"
            | "staging-include-all"
            | "staging-finish"
            | "staging-apply-partial-failure"
            | "staging-apply-failure"
            | "staging-finish-failed"
            | "staging-partial-apply-failure"
            | "staging-cancel"
            | "staging-back"
    ) {
        return None;
    }
    let states = after.staging_hunk_states.clone();
    let focused_hunk = after.focused_hunk.clone();
    let apply_failed = matches!(
        case_id,
        "staging-apply-partial-failure"
            | "staging-finish-failed"
            | "staging-apply-failure"
            | "staging-partial-failure"
            | "apply-partial-failure"
    );
    let applied_hunks = match case_id {
        "diff-apply-all" => vec!["hunk.retry-guard".to_owned(), "hunk.retry-test".to_owned()],
        "staging-finish" => vec!["hunk.retry-guard".to_owned()],
        _ => vec![],
    };
    let excluded_hunks = states
        .iter()
        .enumerate()
        .filter(|(_, state)| state.as_str() == "Excluded" || state.as_str() == "Pending")
        .map(|(index, _)| {
            if index == 0 {
                "hunk.retry-guard".to_owned()
            } else {
                "hunk.retry-test".to_owned()
            }
        })
        .collect::<Vec<_>>();
    let write_count = u32::from(matches!(case_id, "diff-apply-all" | "staging-finish"));
    let working_tree_before = sha256_hex(b"retry_policy.rs before patch\n");
    let working_tree_after = if write_count == 1 && !apply_failed {
        if case_id == "diff-apply-all" {
            sha256_hex(b"retry_policy.rs after both hunks\n")
        } else {
            sha256_hex(b"retry_policy.rs after retry guard\n")
        }
    } else {
        working_tree_before.clone()
    };
    let git_index = sha256_hex(b"git index unchanged\n");
    Some(DiffEvidence {
        patch_revision: "patch-retry-18".to_owned(),
        file_path: "src/net/retry_policy.rs".to_owned(),
        hunks: diff_hunks(states.clone()),
        focused_hunk,
        included_count: states
            .iter()
            .filter(|state| state.as_str() == "Included")
            .count(),
        applied_hunks,
        excluded_hunks,
        write_count,
        working_tree_before,
        working_tree_after,
        git_index_before: git_index.clone(),
        git_index_after: git_index,
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
    if case_id.starts_with("session-") {
        return None;
    }
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
    } else if case_id.contains("scope-outside") {
        (
            b"git diff -- src/docs/release_notes.md".to_vec(),
            command_notation(b"git diff -- src/docs/release_notes.md", false),
        )
    } else {
        (
            SHORT_SHELL_COMMAND.to_vec(),
            command_notation(SHORT_SHELL_COMMAND, false),
        )
    };
    let persistent_scope =
        (case_id == "permission-persistent-scope").then(|| PersistentPermissionScope {
            matcher: "git diff -- src/net/*".to_owned(),
            duration: "this session".to_owned(),
            project_root: "~/project".to_owned(),
            work_style: "Careful edits".to_owned(),
            child_inheritance: "children inherit only read-only checks".to_owned(),
            child_cap: "read-only, same project root, same session".to_owned(),
        });
    let visible_total_length = execution_bytes.len();
    let byte_offset = if case_id.contains("page-88")
        || case_id.contains("long-final")
        || case_id == "permission-shell-long-return"
        || case_id.contains("middle")
    {
        88
    } else if case_id.contains("page-44") {
        44
    } else {
        0
    };
    let page_end = (byte_offset + 44).min(visible_total_length);
    let page_notation = command_notation(
        &execution_bytes[byte_offset..page_end],
        case_id.contains("unsafe-byte"),
    );
    Some(PermissionEvidence {
        command_sha256: sha256_hex(&execution_bytes),
        page_sha256: sha256_hex(&decode_escaped_bytes(&page_notation)),
        execution_bytes,
        display_notation,
        page_notation,
        byte_offset,
        page_end,
        visible_total_length,
        previous_offset: if byte_offset > 0 { Some(0) } else { None },
        next_offset: if case_id.contains("long") && page_end < visible_total_length {
            Some(page_end)
        } else {
            None
        },
        at_boundary: byte_offset == 0 || page_end == visible_total_length,
        execution_working_directory: if case_id.contains("scope-outside") {
            "~/other-project".to_owned()
        } else {
            "~/project".to_owned()
        },
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
        } else if matches!(
            case_id,
            "permission-cancel" | "permission-persistent-scope-outside"
        ) {
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
    field(output, "page_notation", &permission.page_notation);
    field(output, "command_sha256", &permission.command_sha256);
    field(output, "page_sha256", &permission.page_sha256);
    output.push_str(",\"byte_offset\":");
    output.push_str(&permission.byte_offset.to_string());
    output.push_str(",\"page_end\":");
    output.push_str(&permission.page_end.to_string());
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
    field(output, "conversation_id", &state.conversation_id);
    field(output, "transcript_id", &state.transcript_id);
    field(output, "session_id", &state.session_id);
    field(output, "turn_id", &state.turn_id);
    field(output, "user_task", &state.user_task);
    field(output, "visible_origin", &state.visible_origin);
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
    field(output, "attachment", &state.attachment);
    field(output, "transcript", &state.transcript);
    field(output, "partial_response", &state.partial_response);
    field(output, "queue", &state.queue);
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
    field(output, "work_style", &state.work_style);
    output.push_str(",\"pending_work_style\":");
    match &state.pending_work_style {
        Some(value) => quote(output, value),
        None => output.push_str("null"),
    }
    field(output, "access_scope", &state.access_scope);
    field(output, "requested_command", &state.requested_command);
    field(
        output,
        "requested_working_directory",
        &state.requested_working_directory,
    );
    field(
        output,
        "permission_request_status",
        &state.permission_request_status,
    );
    output.push_str(",\"staging_hunk_states\":[");
    for (index, hunk_state) in state.staging_hunk_states.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        quote(output, hunk_state);
    }
    output.push(']');
    field(output, "focused_hunk", &state.focused_hunk);
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
        if audit.screen_id == "chat-turn" && cell.glyph == '_' {
            draw_underscore(
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

#[cfg(feature = "png")]
fn draw_underscore(
    image: &mut image::RgbImage,
    cell_x: u32,
    cell_y: u32,
    foreground: image::Rgb<u8>,
) {
    let y = cell_y + 17;
    for x in cell_x + 1..cell_x + 9 {
        if x < image.width() && y < image.height() {
            image.put_pixel(x, y, foreground);
        }
    }
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
