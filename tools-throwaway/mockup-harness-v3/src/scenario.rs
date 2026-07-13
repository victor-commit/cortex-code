//! Deterministic named interaction scenarios for typed audit evidence.

use std::collections::BTreeMap;

use crate::audit::{
    action_json, audit_screen, field, quote, sha256_hex, transition_json, ActionEvidence,
    AuditDocument, TransitionTrace,
};
use crate::screens::input_and_approval::{
    ATTACHMENT_OPEN_DRAFT, ATTACHMENT_SELECTED_PATH, ATTACHMENT_TOKEN, EXPORT_ARTIFACT_MARKDOWN,
    EXPORT_DEFAULT_FORMAT, EXPORT_OPEN_DRAFT, EXPORT_SUCCESS_SUMMARY, INSERTED_FILE,
    MENTION_FRAGMENT, MULTILINE_DRAFT, PALETTE_PRESERVED_DRAFT, SLASH_ACTIVE_SPAN,
    SLASH_CONFIRMED_EXPORT, SLASH_OPEN_DRAFT, SLASH_PRE_OPEN_DRAFT,
};

const FLOW2_SESSION_ID: &str = "session.retry-review.18";
const FLOW2_PAUSED_ACTIVITY_ID: &str = "activity.retry-policy-edit.18";
const FLOW2_SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
const FLOW2_LATER_DRAFT: &str = "Add a short note about the remaining test hunk.";
const REFUSAL_UNCHANGED_ACTION_ID: &str = "action.mode-switch-refused.choose-unchanged";

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

const PERMISSION_PAGE_SIZE: usize = 44;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunkEvidence {
    pub hunk_id: String,
    pub file_path: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub old_payload: Vec<String>,
    pub new_payload: Vec<String>,
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

/// The deterministic file bytes used by every public diff-application replay.
///
/// Keeping this fixture as bytes, rather than a descriptive tree label, makes
/// the before/after evidence independently reproducible by validators.
pub fn diff_fixture_file_bytes() -> Vec<u8> {
    let mut lines = (1..=90)
        .map(|line| format!("// retry policy fixture line {line}"))
        .collect::<Vec<_>>();
    lines[41] = "    return retry_after".to_owned();
    lines[42] = "    backoff = backoff + 1".to_owned();
    lines[87] = "assert_eq!(wait, 4)".to_owned();
    lines[88] = "assert_eq!(attempts, 1)".to_owned();
    format!("{}\n", lines.join("\n")).into_bytes()
}

/// The index bytes remain unchanged for all diff scenarios.
pub fn diff_index_fixture_bytes() -> &'static [u8] {
    b"index entry: src/net/retry_policy.rs\nmode: unchanged\n"
}

#[derive(Clone, Copy)]
struct DiffHunkDefinition {
    hunk_id: &'static str,
    old_payload: &'static [&'static str],
    new_payload: &'static [&'static str],
}

const RETRY_GUARD_OLD: &[&str] = &["    return retry_after", "    backoff = backoff + 1"];
const RETRY_GUARD_NEW: &[&str] = &[
    "    return retry_after + jitter",
    "    backoff = backoff + jitter",
    "    log_retry_window(jitter)",
];
const RETRY_TEST_OLD: &[&str] = &["assert_eq!(wait, 4)", "assert_eq!(attempts, 1)"];
const RETRY_TEST_NEW: &[&str] = &["assert_eq!(wait, 5)", "assert_eq!(attempts, 1)"];

const DIFF_HUNK_DEFINITIONS: &[DiffHunkDefinition] = &[
    DiffHunkDefinition {
        hunk_id: "hunk.retry-guard",
        old_payload: RETRY_GUARD_OLD,
        new_payload: RETRY_GUARD_NEW,
    },
    DiffHunkDefinition {
        hunk_id: "hunk.retry-test",
        old_payload: RETRY_TEST_OLD,
        new_payload: RETRY_TEST_NEW,
    },
];

fn diff_lines(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(String::from_utf8(bytes.to_vec())
        .map_err(|_| "diff fixture bytes must be valid UTF-8".to_owned())?
        .lines()
        .map(str::to_owned)
        .collect())
}

fn unique_payload_start(
    lines: &[String],
    payload: &[&str],
    hunk_id: &str,
) -> Result<usize, String> {
    let matches = lines
        .windows(payload.len())
        .enumerate()
        .filter_map(|(index, window)| {
            window
                .iter()
                .map(String::as_str)
                .eq(payload.iter().copied())
                .then_some(index)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [start] => Ok(*start),
        [] => Err(format!(
            "{hunk_id} exact preimage payload is missing from the original bytes"
        )),
        _ => Err(format!(
            "{hunk_id} exact preimage payload is not unique in the original bytes"
        )),
    }
}

fn diff_hunk_payload_bytes(hunk: &DiffHunkEvidence) -> Vec<u8> {
    let mut payload = format!("{}\n", hunk.hunk_id);
    for line in &hunk.old_payload {
        payload.push('-');
        payload.push_str(line);
        payload.push('\n');
    }
    for (index, line) in hunk.new_payload.iter().enumerate() {
        payload.push('+');
        payload.push_str(line);
        if index + 1 < hunk.new_payload.len() {
            payload.push('\n');
        }
    }
    payload.into_bytes()
}

pub(crate) fn diff_hunk_content_hash(hunk: &DiffHunkEvidence) -> String {
    sha256_hex(&diff_hunk_payload_bytes(hunk))
}

fn derive_diff_hunks_from_bytes(
    bytes: &[u8],
    states: [String; 2],
) -> Result<Vec<DiffHunkEvidence>, String> {
    let lines = diff_lines(bytes)?;
    let mut cumulative_line_delta = 0_i64;
    let mut prior_old_end = 0_usize;
    let mut hunks = Vec::with_capacity(DIFF_HUNK_DEFINITIONS.len());

    for (definition, state) in DIFF_HUNK_DEFINITIONS.iter().zip(states) {
        let old_index = unique_payload_start(&lines, definition.old_payload, definition.hunk_id)?;
        if old_index < prior_old_end {
            return Err(format!(
                "{} is not ordered after the preceding exact payload",
                definition.hunk_id
            ));
        }
        let old_start =
            u32::try_from(old_index + 1).map_err(|_| "diff old_start exceeds u32".to_owned())?;
        let old_lines = u32::try_from(definition.old_payload.len())
            .map_err(|_| "diff old_lines exceeds u32".to_owned())?;
        let new_lines = u32::try_from(definition.new_payload.len())
            .map_err(|_| "diff new_lines exceeds u32".to_owned())?;
        let shifted_start = i64::from(old_start) + cumulative_line_delta;
        let new_start = u32::try_from(shifted_start)
            .map_err(|_| "diff cumulative new_start is out of range".to_owned())?;
        let mut hunk = DiffHunkEvidence {
            hunk_id: definition.hunk_id.to_owned(),
            file_path: "src/net/retry_policy.rs".to_owned(),
            old_start,
            old_lines,
            new_start,
            new_lines,
            old_payload: definition
                .old_payload
                .iter()
                .map(|line| (*line).to_owned())
                .collect(),
            new_payload: definition
                .new_payload
                .iter()
                .map(|line| (*line).to_owned())
                .collect(),
            state,
            content_hash: String::new(),
        };
        hunk.content_hash = diff_hunk_content_hash(&hunk);
        cumulative_line_delta += i64::from(new_lines) - i64::from(old_lines);
        prior_old_end = old_index + definition.old_payload.len();
        hunks.push(hunk);
    }
    Ok(hunks)
}

fn validate_diff_hunk_contract(bytes: &[u8], hunks: &[DiffHunkEvidence]) -> Result<(), String> {
    if hunks.len() != DIFF_HUNK_DEFINITIONS.len() {
        return Err("diff hunk contract requires every ordered exact payload".to_owned());
    }
    let states = [hunks[0].state.clone(), hunks[1].state.clone()];
    let expected = derive_diff_hunks_from_bytes(bytes, states)?;
    for (actual, expected) in hunks.iter().zip(expected.iter()) {
        if actual.hunk_id != expected.hunk_id || actual.file_path != expected.file_path {
            return Err("diff hunk identity or ordering drifted from exact payloads".to_owned());
        }
        if actual.old_lines != actual.old_payload.len() as u32
            || actual.new_lines != actual.new_payload.len() as u32
        {
            return Err("diff hunk line counts do not match exact payloads".to_owned());
        }
        if actual.old_payload != expected.old_payload || actual.new_payload != expected.new_payload
        {
            return Err("diff hunk payload drifted from the declared patch".to_owned());
        }
        if actual.old_start != expected.old_start {
            return Err(format!(
                "{} old coordinate does not match its exact preimage payload",
                actual.hunk_id
            ));
        }
        if actual.new_start != expected.new_start {
            return Err(format!(
                "{} cumulative new_start does not include preceding line deltas",
                actual.hunk_id
            ));
        }
        if actual.content_hash != diff_hunk_content_hash(actual) {
            return Err("diff hunk content hash does not match exact payloads".to_owned());
        }
    }
    Ok(())
}

fn apply_validated_diff_hunks(
    bytes: &[u8],
    hunks: &[DiffHunkEvidence],
    hunk_ids: &[String],
) -> Result<Vec<u8>, String> {
    validate_diff_hunk_contract(&diff_fixture_file_bytes(), hunks)?;
    let selected_positions = hunk_ids
        .iter()
        .map(|hunk_id| {
            hunks
                .iter()
                .position(|hunk| hunk.hunk_id == *hunk_id)
                .ok_or_else(|| format!("unknown disclosed hunk {hunk_id}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if selected_positions.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("selected hunks must be unique and follow validated patch order".to_owned());
    }

    let mut lines = diff_lines(bytes)?;
    let mut selected_line_delta = 0_i64;
    for position in selected_positions {
        let hunk = &hunks[position];
        let start = i64::from(hunk.old_start) - 1 + selected_line_delta;
        let start = usize::try_from(start)
            .map_err(|_| format!("{} apply coordinate is out of range", hunk.hunk_id))?;
        let end = start
            .checked_add(hunk.old_payload.len())
            .ok_or_else(|| format!("{} apply range overflowed", hunk.hunk_id))?;
        if lines.get(start..end) != Some(hunk.old_payload.as_slice()) {
            return Err(format!(
                "{} exact preimage bytes do not match its validated coordinate",
                hunk.hunk_id
            ));
        }
        lines.splice(start..end, hunk.new_payload.iter().cloned());
        selected_line_delta += i64::from(hunk.new_lines).saturating_sub(i64::from(hunk.old_lines));
    }
    Ok(format!("{}\n", lines.join("\n")).into_bytes())
}

/// Apply selected hunks only after their ordered payloads, coordinates, and
/// cumulative line deltas have been validated against the exact preimage.
pub fn apply_diff_hunks(bytes: &[u8], hunk_ids: &[String]) -> Result<Vec<u8>, String> {
    let hunks = derive_diff_hunks_from_bytes(
        &diff_fixture_file_bytes(),
        ["Included".to_owned(), "Included".to_owned()],
    )?;
    apply_validated_diff_hunks(bytes, &hunks, hunk_ids)
}

pub(crate) fn validated_diff_hunks_for_render() -> Result<Vec<DiffHunkEvidence>, String> {
    let bytes = diff_fixture_file_bytes();
    let hunks =
        derive_diff_hunks_from_bytes(&bytes, ["Included".to_owned(), "Pending".to_owned()])?;
    validate_diff_hunk_contract(&bytes, &hunks)?;
    Ok(hunks)
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

/// Validates an ordered page walk against the command bytes it discloses.
///
/// The caller supplies pages in traversal order. This deliberately checks
/// both the bytes and the adjacent offsets so a plausible-looking page label
/// cannot hide a gap, overlap, or fabricated notation.
pub fn validate_permission_traversal(pages: &[PermissionEvidence]) -> Result<(), String> {
    let Some(first) = pages.first() else {
        return Err("permission page traversal must contain at least one page".to_owned());
    };
    let execution_bytes = &first.execution_bytes;
    let mut reconstructed = Vec::with_capacity(execution_bytes.len());
    for (index, page) in pages.iter().enumerate() {
        if page.execution_bytes != *execution_bytes {
            return Err("permission page traversal changes execution bytes".to_owned());
        }
        if index == 0 {
            if page.byte_offset != 0 || page.previous_offset.is_some() {
                return Err("permission page traversal must start at byte offset 0".to_owned());
            }
        } else {
            let previous = &pages[index - 1];
            if previous.page_end != page.byte_offset {
                return Err(
                    "permission page traversal contains a gap or overlap between adjacent windows"
                        .to_owned(),
                );
            }
            if page.previous_offset != Some(previous.byte_offset)
                || previous.next_offset != Some(page.byte_offset)
            {
                return Err(
                    "permission page traversal has incorrect adjacent previous or next offsets"
                        .to_owned(),
                );
            }
        }
        validate_permission_page(page)?;
        reconstructed.extend(page.reconstructed_page_bytes());
    }
    let final_page = pages.last().expect("first page guarantees a final page");
    if final_page.page_end != execution_bytes.len() || final_page.next_offset.is_some() {
        return Err(
            "permission page traversal does not end at the execution byte boundary".to_owned(),
        );
    }
    if reconstructed != *execution_bytes {
        return Err(
            "permission page traversal does not fully reconstruct execution_bytes".to_owned(),
        );
    }
    Ok(())
}

fn validate_permission_page(permission: &PermissionEvidence) -> Result<(), String> {
    let page_length = permission.reconstructed_page_bytes().len();
    let expected_previous = (permission.byte_offset > 0)
        .then_some(permission.byte_offset.saturating_sub(PERMISSION_PAGE_SIZE));
    if permission.command_sha256 != sha256_hex(&permission.execution_bytes)
        || permission.reconstructed_bytes() != permission.execution_bytes
        || permission.page_sha256 != sha256_hex(&permission.reconstructed_page_bytes())
    {
        return Err("permission evidence does not reconstruct exact execution bytes".to_owned());
    }
    if permission.byte_offset > permission.visible_total_length
        || permission.visible_total_length != permission.execution_bytes.len()
        || permission.page_end < permission.byte_offset
        || permission.page_end > permission.visible_total_length
        || permission.page_end
            != permission.byte_offset + permission.reconstructed_page_bytes().len()
        || page_length == 0
        || page_length > PERMISSION_PAGE_SIZE
        || (permission.next_offset.is_some() && page_length != PERMISSION_PAGE_SIZE)
        || permission.at_boundary
            != (permission.byte_offset == 0
                || permission.page_end == permission.visible_total_length)
        || (permission.page_end == permission.visible_total_length
            && permission.next_offset.is_some())
        || permission
            .next_offset
            .is_some_and(|offset| offset != permission.page_end)
        || permission.previous_offset != expected_previous
    {
        return Err("permission page boundary evidence is inconsistent".to_owned());
    }
    if permission.reconstructed_page_bytes()
        != permission.execution_bytes[permission.byte_offset..permission.page_end]
    {
        return Err("permission evidence does not reconstruct exact execution bytes".to_owned());
    }
    if permission.execution_working_directory != "~/project"
        && permission.execution_working_directory != "~/other-project"
    {
        return Err("permission evidence must name the canonical working directory".to_owned());
    }
    if permission.invocation_count > 1 || permission.persisted_rule_count > 1 {
        return Err("permission evidence exceeds single invocation or rule bounds".to_owned());
    }
    Ok(())
}

fn validate_permission_replay_binding(
    case_id: &str,
    after: &ScenarioState,
    permission: &PermissionEvidence,
    audit: &AuditDocument,
) -> Result<(), String> {
    if after.requested_command.as_bytes() != permission.execution_bytes {
        return Err("requested_command bytes do not equal permission execution_bytes".to_owned());
    }
    if !case_id.starts_with("permission-shell-long") {
        return Ok(());
    }

    if after.selection == "permission-choice" {
        let stale_return_action = audit.events.iter().any(|event| {
            event
                .action
                .as_ref()
                .is_some_and(|action| action.action_id == "action.permission.command-return")
        });
        let stale_return_transition = audit
            .transitions
            .iter()
            .any(|transition| transition.action_id == "action.permission.command-return");
        let stale_command_detail = after.focus == "command-detail"
            || audit
                .events
                .iter()
                .any(|event| event.identity.family == "command-navigation")
            || audit
                .render_txt()
                .contains("Command detail · focused disclosure")
            || audit
                .render_txt()
                .contains("[r] Return to permission choices");
        if after.focus != "permission-choice"
            || stale_return_action
            || stale_return_transition
            || stale_command_detail
        {
            return Err(
                "completed permission-choice destination retains stale Return or command detail"
                    .to_owned(),
            );
        }
        return Ok(());
    }

    let expected_offset = after
        .selection
        .strip_prefix("long-command-page-")
        .and_then(|offset| offset.parse::<usize>().ok())
        .ok_or_else(|| {
            "permission replay rendered pre-transition state: resulting page selection is invalid"
                .to_owned()
        })?;
    let rendered_offsets = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "command-navigation")
        .filter_map(|event| {
            event
                .identity
                .subject_id
                .strip_prefix("page-")
                .and_then(|offset| offset.parse::<usize>().ok())
        })
        .collect::<Vec<_>>();
    if after.focus != "command-detail"
        || permission.byte_offset != expected_offset
        || rendered_offsets.as_slice() != [expected_offset]
    {
        return Err(format!(
            "permission replay rendered pre-transition state: after offset {expected_offset}, evidence offset {}, rendered offsets {:?}",
            permission.byte_offset, rendered_offsets
        ));
    }
    Ok(())
}

pub(crate) fn canonical_permission_pages() -> Vec<PermissionEvidence> {
    [
        (0, "canonical-page-0"),
        (44, "canonical-page-44"),
        (88, "canonical-page-88"),
    ]
    .into_iter()
    .map(|(offset, case_id)| {
        permission_evidence_at_offset(case_id, offset).expect("canonical permission page evidence")
    })
    .collect()
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
        // provider-to-mcp-bridge is an ordered illustrative detour that openly
        // rebinds conversation/session/task from provider identity to MCP.
        // session-resume-latest rebinds to the selected timeline session.
        if ((self.before.session_id != self.after.session_id
            || self.before.paused_activity_id != self.after.paused_activity_id)
            && self.case_id != "session-resume-latest"
            && self.case_id != "provider-to-mcp-bridge")
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
        if self.case_id == "work-style-access-refused"
            && (action.action_id != REFUSAL_UNCHANGED_ACTION_ID
                || transition.action_id != REFUSAL_UNCHANGED_ACTION_ID
                || !self.audit.events.iter().any(|event| {
                    event
                        .action
                        .as_ref()
                        .is_some_and(|action| action.action_id == REFUSAL_UNCHANGED_ACTION_ID)
                })
                || !self
                    .audit
                    .transitions
                    .iter()
                    .any(|transition| transition.action_id == REFUSAL_UNCHANGED_ACTION_ID))
        {
            return Err(
                "work-style refusal replay must use action.mode-switch-refused.choose-unchanged"
                    .to_owned(),
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
                && !(self.case_id.starts_with("session-") && self.attempts[0].count == 0)
                && !(self.case_id.starts_with("network-")
                    && (self.attempts[0].count == 0
                        || self.attempts[0].outcome == "exhausted"
                        || self.attempts[0].outcome == "none"
                        || self.attempts[0].outcome == "cancelled"))
                && !(self.case_id.starts_with("rate-limit-")
                    && (self.attempts[0].count == 0
                        || self.attempts[0].outcome == "none"
                        || self.attempts[0].outcome == "cancelled"))
                && !(self.case_id.starts_with("checkpoint-")
                    && (self.attempts[0].count == 0
                        || self.attempts[0].outcome == "none"
                        || self.attempts[0].outcome == "conflict"
                        || self.attempts[0].outcome == "no-op"
                        || self.attempts[0].outcome == "stale"
                        || self.attempts[0].outcome == "restored"
                        || self.attempts[0].outcome == "redone"))
                && !(self.case_id.starts_with("mission-")
                    && (self.attempts[0].count == 0
                        || self.attempts[0].outcome == "none"
                        || self.attempts[0].outcome == "cancelled"
                        || self.attempts[0].outcome == "accepted"
                        || self.attempts[0].outcome == "launch-ready"
                        || self.attempts[0].outcome == "still-blocked"
                        || self.attempts[0].outcome == "started"
                        || self.attempts[0].outcome == "progress-open")))
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
            validate_permission_page(permission)?;
            validate_permission_replay_binding(
                &self.case_id,
                &self.after,
                permission,
                &self.audit,
            )?;
            if self.case_id.starts_with("permission-shell-long")
                && self.case_id != "permission-shell-long-return"
                && self.case_id != "permission-shell-long-final-to-return"
            {
                validate_permission_traversal(&canonical_permission_pages())?;
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
            if permission.verdict == "denied" && permission.invocation_count != 0 {
                return Err("denied permission evidence contains an invocation".to_owned());
            }
        }
        if let Some(diff) = &self.diff {
            validate_diff(diff)?;
            if matches!(
                self.case_id.as_str(),
                "staging-apply-partial-failure"
                    | "staging-apply-failure"
                    | "staging-finish-failed"
                    | "staging-partial-apply-failure"
                    | "staging-partial-failure"
                    | "apply-partial-failure"
            ) && (self.after.screen_id != "diff-viewer-staging"
                || self.transitions[0].to_screen_id != "diff-viewer-staging"
                || self.side_effects[0].status != "apply-failed-no-write")
            {
                return Err(
                    "stale diff apply must remain open and report one no-write failure".to_owned(),
                );
            }
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
        "permission-shell-long-page-44" => permission_shell_long_page(44, 88),
        "permission-shell-long-middle" | "permission-shell-long-middle-to-final" => {
            permission_shell_long_final()
        }
        "permission-shell-long-page-88" | "permission-shell-long-final" => {
            permission_shell_long_final()
        }
        "permission-shell-long-return" | "permission-shell-long-final-to-return" => {
            permission_shell_long_return()
        }
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
        "reasoning-expand" => reasoning_expand(),
        "reasoning-collapse" => reasoning_collapse(),
        "tool-success-expand" => tool_success_expand(),
        "tool-success-collapse" => tool_success_collapse(),
        "tool-task-expand" => tool_task_expand(),
        "tool-task-collapse" => tool_task_collapse(),
        "compact-expand-history" => compact_expand_history(),
        "compact-collapse-history" => compact_collapse_history(),
        "shell-interrupt" => shell_interrupt(),
        "shell-complete" => shell_complete(),
        "todo-progress" => todo_progress(),
        "chat-long-scroll-return-latest" => chat_long_scroll_return_latest(),
        "chat-long-scroll-jump-earlier" => chat_long_scroll_jump_earlier(),
        "queued-interrupt-streaming" => queued_interrupt_streaming(),
        "queued-interrupt-tool" => queued_interrupt_tool(),
        "queued-interrupt-shell" => queued_interrupt_shell(),
        "agent-question-open" => agent_question_open(),
        "agent-question-submit" => agent_question_submit(),
        "agent-question-cancel" => agent_question_cancel(),
        "reading-anchor-output-arrive" => reading_anchor_output_arrive(),
        "reading-anchor-jump-latest" => reading_anchor_jump_latest(),
        "transcript-search-open" => transcript_search_open(),
        "transcript-search-next" => transcript_search_next(),
        "transcript-search-no-results" => transcript_search_no_results(),
        "transcript-search-close" => transcript_search_close(),
        "transcript-copy-message" => transcript_copy_message(),
        "shell-foreground-interrupt" => shell_foreground_interrupt(),
        "shell-background-stop" => shell_background_stop(),
        "tool-task-stop-selected" => tool_task_stop_selected(),
        "tool-task-retry-selected" => tool_task_retry_selected(),
        "tool-task-cancel-selected" => tool_task_cancel_selected(),
        "large-output-open" => large_output_open(),
        "large-output-search" => large_output_search(),
        "large-output-close" => large_output_close(),
        "file-mention-insert" => file_mention_insert(),
        "file-mention-dismiss" => file_mention_dismiss(),
        "file-mention-no-results" => file_mention_no_results(),
        "slash-confirm-export" => slash_confirm_export(),
        "slash-cancel" => slash_cancel(),
        "slash-invalid-trigger" => slash_invalid_trigger(),
        "attachment-confirm" => attachment_confirm(),
        "attachment-cancel" => attachment_cancel(),
        "export-confirm" => export_confirm(),
        "export-cancel" => export_cancel(),
        "palette-open-submenu" => palette_open_submenu(),
        "palette-submenu-back" => palette_submenu_back(),
        "palette-cancel" => palette_cancel(),
        "palette-submenu-cancel" => palette_submenu_cancel(),
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
        | "staging-partial-apply-failure"
        | "staging-partial-failure"
        | "apply-partial-failure" => staging_apply_partial_failure(),
        "staging-cancel" | "staging-back" => staging_cancel(),
        "onboarding-complete" => onboarding_complete(),
        "onboarding-skip" => onboarding_skip(),
        "onboarding-back" => onboarding_back(),
        "onboarding-existing-cancel" => onboarding_existing_cancel(),
        "onboarding-unusable-model" => onboarding_unusable_model(),
        "model-picker-apply" => model_picker_apply(),
        "model-picker-cancel" => model_picker_cancel(),
        "model-picker-connect" => model_picker_connect(),
        "model-to-work-style-bridge" => model_to_work_style_bridge(),
        "work-style-unchanged" => work_style_unchanged(),
        "work-style-access-pending" => work_style_access_pending(),
        "work-style-access-confirm" => work_style_access_confirm(),
        "work-style-access-cancel" => work_style_access_cancel(),
        "work-style-access-refused" => work_style_access_refused(),
        "work-style-refusal-choose-unchanged" => work_style_refusal_choose_unchanged(),
        "work-style-refusal-return" => work_style_refusal_return(),
        "mode-switch-to-session-bridge" => mode_switch_to_session_bridge(),
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
        "login-cancel" => login_cancel(),
        "connect-provider-select" => connect_provider_select(),
        "connect-method-back" => connect_method_back(),
        "connect-connected-return" => connect_connected_return(),
        "mcp-return" => mcp_return(),
        "turn-error-retry" => turn_error_retry(),
        "turn-error-cancel" => turn_error_cancel(),
        "turn-error-inspect-connection" => turn_error_inspect_connection(),
        "network-offline-review-setting" => network_offline_review_setting(),
        "network-offline-resume" => network_offline_resume(),
        "network-backoff-cancel" => network_backoff_cancel(),
        "network-backoff-exhaust" => network_backoff_exhaust(),
        "network-backoff-race-retry-wins" => network_backoff_race_retry_wins(),
        "network-backoff-race-cancel-wins" => network_backoff_race_cancel_wins(),
        "network-check-connection" => network_check_connection(),
        "network-check-connection-return" => network_check_connection_return(),
        "network-backoff-snapshot" => network_backoff_snapshot(),
        "network-evidence-snapshot" => network_evidence_snapshot(),
        "rate-limit-automatic-success" => rate_limit_automatic_success(),
        "rate-limit-retry-now" => rate_limit_retry_now(),
        "rate-limit-race-retry-wins" => rate_limit_race_retry_wins(),
        "rate-limit-race-cancel-wins" => rate_limit_race_cancel_wins(),
        "checkpoint-undo-success" => checkpoint_undo_success(),
        "checkpoint-redo-success" => checkpoint_redo_success(),
        "checkpoint-undo-conflict" => checkpoint_undo_conflict(),
        "checkpoint-undo-noop" => checkpoint_undo_noop(),
        "checkpoint-redo-stale" => checkpoint_redo_stale(),
        "checkpoint-continue-chat" => checkpoint_continue_chat(),
        "chat-turn-open-checkpoint-undo" => chat_turn_open_checkpoint_undo(),
        "mission-proposal-accept" => mission_proposal_accept(),
        "mission-resolve-input-ready" => mission_resolve_input_ready(),
        "mission-resolve-input-still-blocked" => mission_resolve_input_still_blocked(),
        "mission-start" => mission_start(),
        "mission-launch-cancel" => mission_launch_cancel(),
        "mission-blocked-cancel" => mission_blocked_cancel(),
        "mission-view-progress" => mission_view_progress(),
        "mission-create-fix-task" => mission_create_fix_task(),
        "mission-fix-working" => mission_fix_working(),
        "mission-fix-validating" => mission_fix_validating(),
        "mission-validation-success" => mission_validation_success(),
        "mission-validation-failure" => mission_validation_failure(),
        "mission-review-create-fix" => mission_review_create_fix(),
        "mission-review-fix-validating" => mission_review_fix_validating(),
        "mission-completion-validation" => mission_completion_validation(),
        "mission-request-changes-open" => mission_request_changes_open(),
        "mission-request-changes-submit" => mission_request_changes_submit(),
        "mission-request-changes-cancel" => mission_request_changes_cancel(),
        "mission-approve-completion" => mission_approve_completion(),
        "mission-fix-loop-rerepair" => mission_fix_loop_rerepair(),
        "login-choose-provider" => login_choose_provider(),
        "connect-method-oauth" => connect_method_oauth(),
        "connect-oauth-finish" => connect_oauth_finish(),
        "provider-to-mcp-bridge" => provider_to_mcp_bridge(),
        "error-network-to-rate-limit-bridge" => error_network_to_rate_limit_bridge(),
        "mission-monitor-to-failure-bridge" => mission_monitor_to_failure_bridge(),
        "mission-failure-to-review-bridge" => mission_failure_to_review_bridge(),
        "mission-review-to-approved-bridge" => mission_review_to_approved_bridge(),
        _ => return Err(format!("unknown named case: {case_id}")),
    };
    let mut preserved_field_hashes = preserved_hashes(&before, &after);
    if matches!(
        case_id,
        "compact-expand-history" | "compact-collapse-history"
    ) {
        let order = "turn-1,turn-8,turn-18";
        preserved_field_hashes.insert(
            "history_order".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(order.as_bytes()),
                after: sha256_hex(order.as_bytes()),
            },
        );
    }
    if matches!(case_id, "shell-interrupt" | "shell-complete") {
        let captured = "git status output · cargo test partial · master ahead 1";
        preserved_field_hashes.insert(
            "captured_output".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(captured.as_bytes()),
                after: sha256_hex(captured.as_bytes()),
            },
        );
    }
    if matches!(
        case_id,
        "shell-foreground-interrupt" | "shell-background-stop"
    ) {
        let captured = "mixed shell: git status · cargo test partial · watch tail lines";
        preserved_field_hashes.insert(
            "captured_output".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(captured.as_bytes()),
                after: sha256_hex(captured.as_bytes()),
            },
        );
        let background_process = "activity.shell.background-watch|running|rev=1";
        let foreground_process = "activity.shell.foreground-cargo|running|rev=1";
        if case_id == "shell-foreground-interrupt" {
            preserved_field_hashes.insert(
                "background_process".to_owned(),
                PreservedFieldHash {
                    before: sha256_hex(background_process.as_bytes()),
                    after: sha256_hex(background_process.as_bytes()),
                },
            );
        }
        if case_id == "shell-background-stop" {
            preserved_field_hashes.insert(
                "foreground_process".to_owned(),
                PreservedFieldHash {
                    before: sha256_hex(foreground_process.as_bytes()),
                    after: sha256_hex(foreground_process.as_bytes()),
                },
            );
        }
    }
    if matches!(
        case_id,
        "tool-task-stop-selected" | "tool-task-retry-selected" | "tool-task-cancel-selected"
    ) {
        let siblings =
            "task-audit-auth|complete|6|task-build-suite|queued|0|task-refresh-risk|blocked|1";
        preserved_field_hashes.insert(
            "sibling_task_hashes".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(siblings.as_bytes()),
                after: sha256_hex(siblings.as_bytes()),
            },
        );
        let selected_before = match case_id {
            "tool-task-stop-selected" => "task-add-tests|running|2of3",
            "tool-task-retry-selected" => "task-refresh-risk|blocked|needs-decision",
            _ => "task-build-suite|queued|waiting",
        };
        let selected_after = match case_id {
            "tool-task-stop-selected" => "task-add-tests|stopped|2of3",
            "tool-task-retry-selected" => "task-refresh-risk|retrying|1",
            _ => "task-build-suite|cancelled|waiting",
        };
        preserved_field_hashes.insert(
            "selected_task".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(selected_before.as_bytes()),
                after: sha256_hex(selected_after.as_bytes()),
            },
        );
    }
    if matches!(
        case_id,
        "large-output-open" | "large-output-search" | "large-output-close"
    ) {
        let complete = "cargo-test.full-output.240-lines.sha256fixture";
        preserved_field_hashes.insert(
            "complete_output".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(complete.as_bytes()),
                after: sha256_hex(complete.as_bytes()),
            },
        );
    }
    if case_id == "transcript-copy-message" {
        let turn_count = "turn-count=4";
        preserved_field_hashes.insert(
            "turn_count".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(turn_count.as_bytes()),
                after: sha256_hex(turn_count.as_bytes()),
            },
        );
        let copied = "The shared request guard applies the limit before route work begins.";
        preserved_field_hashes.insert(
            "copied_text".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(copied.as_bytes()),
                after: sha256_hex(copied.as_bytes()),
            },
        );
    }
    if case_id == "todo-progress" {
        preserved_field_hashes.insert(
            "user_task".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.user_task.as_bytes()),
                after: sha256_hex(after.user_task.as_bytes()),
            },
        );
    }
    match case_id {
        "checkpoint-undo-success" => insert_checkpoint_file_hashes(
            &mut preserved_field_hashes,
            FLOW5_FILE_RELEASE_BEFORE,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RISKS_BEFORE,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_CHECKPOINT,
            FLOW5_CHECKPOINT,
        ),
        "checkpoint-redo-success" => insert_checkpoint_file_hashes(
            &mut preserved_field_hashes,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RELEASE_BEFORE,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_FILE_RISKS_BEFORE,
            FLOW5_CHECKPOINT,
            FLOW5_CHECKPOINT,
        ),
        "checkpoint-undo-conflict" => insert_checkpoint_file_hashes(
            &mut preserved_field_hashes,
            FLOW5_FILE_RELEASE_BEFORE,
            FLOW5_FILE_RELEASE_BEFORE,
            FLOW5_FILE_RISKS_CONFLICT,
            FLOW5_FILE_RISKS_CONFLICT,
            FLOW5_CHECKPOINT,
            FLOW5_CHECKPOINT,
        ),
        "checkpoint-undo-noop" | "checkpoint-continue-chat" => insert_checkpoint_file_hashes(
            &mut preserved_field_hashes,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_CHECKPOINT,
            FLOW5_CHECKPOINT,
        ),
        "checkpoint-redo-stale" => insert_checkpoint_file_hashes(
            &mut preserved_field_hashes,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RELEASE_AFTER,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_FILE_RISKS_AFTER,
            FLOW5_CHECKPOINT,
            FLOW5_CHECKPOINT,
        ),
        _ => {}
    }
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
    if case_id == "slash-confirm-export" {
        preserved_field_hashes.insert(
            "draft_surrounding".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(
                    before
                        .draft
                        .replacen(SLASH_ACTIVE_SPAN, "", 1)
                        .as_bytes(),
                ),
                after: sha256_hex(
                    after
                        .draft
                        .replacen(SLASH_CONFIRMED_EXPORT, "", 1)
                        .as_bytes(),
                ),
            },
        );
    }
    if case_id == "attachment-confirm" {
        let insertion_unit = format!(" {ATTACHMENT_TOKEN}");
        preserved_field_hashes.insert(
            "draft_surrounding".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.draft.as_bytes()),
                after: sha256_hex(after.draft.replacen(&insertion_unit, "", 1).as_bytes()),
            },
        );
        preserved_field_hashes.insert(
            "attachment_count".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b"0"),
                after: sha256_hex(b"1"),
            },
        );
    }
    if case_id == "attachment-cancel" {
        preserved_field_hashes.insert(
            "attachment".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(before.attachment.as_bytes()),
                after: sha256_hex(after.attachment.as_bytes()),
            },
        );
    }
    if case_id == "export-confirm" {
        preserved_field_hashes.insert(
            "export_artifact".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b""),
                after: sha256_hex(EXPORT_ARTIFACT_MARKDOWN.as_bytes()),
            },
        );
        preserved_field_hashes.insert(
            "result_event_count".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b"0"),
                after: sha256_hex(b"1"),
            },
        );
    }
    if case_id == "export-cancel" {
        preserved_field_hashes.insert(
            "export_artifact".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b""),
                after: sha256_hex(b""),
            },
        );
        preserved_field_hashes.insert(
            "result_event_count".to_owned(),
            PreservedFieldHash {
                before: sha256_hex(b"0"),
                after: sha256_hex(b"0"),
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
    } else if case_id.starts_with("permission-shell-long") {
        match after.selection.as_str() {
            "long-command-page-44" => {
                "case.permission-shell-long-after-offset-44.tick.1".to_owned()
            }
            "long-command-page-88" => {
                "case.permission-shell-long-after-offset-88.tick.1".to_owned()
            }
            "permission-choice" => "case.permission-shell-long-final-to-return.tick.1".to_owned(),
            selection => {
                return Err(format!(
                    "long permission replay has unknown resulting selection: {selection}"
                ));
            }
        }
    } else if case_id == "staging-finish" {
        "case.staging-finish.tick.1.flow2.input-and-approval.stable".to_owned()
    } else if case_id == "model-to-work-style-bridge" {
        "gate.work-configuration.agent-picker".to_owned()
    } else if case_id == "mode-switch-to-session-bridge" {
        "gate.work-configuration.session-picker".to_owned()
    } else if case_id.starts_with("work-style-") {
        match case_id {
            "work-style-unchanged" => "flow3-work-style-unchanged",
            "work-style-access-pending" => "flow3-work-style-access",
            "work-style-access-confirm" => "flow3-work-style-access",
            "work-style-access-cancel" => "gate.work-configuration.agent-picker.current-only",
            "work-style-access-refused" => "flow3-work-style-refused",
            "work-style-refusal-choose-unchanged" => "flow3-work-style-refusal-unchanged",
            "work-style-refusal-return" => "flow3-work-style-refused-return",
            _ => unreachable!(),
        }
        .to_owned()
    } else if case_id.starts_with("session-")
        && matches!(after.screen_id.as_str(), "session-picker" | "timeline")
    {
        format!(
            "gate.work-configuration.{}.case.{case_id}.tick.{tick}",
            after.screen_id
        )
    } else if matches!(
        case_id,
        "login-cancel"
            | "connect-provider-select"
            | "connect-method-back"
            | "connect-connected-return"
            | "mcp-return"
            | "login-choose-provider"
            | "connect-method-oauth"
            | "connect-oauth-finish"
            | "provider-to-mcp-bridge"
    ) {
        match after.screen_id.as_str() {
            "login" => "gate.provider-and-mcp.login".to_owned(),
            "connect-provider" => "gate.provider-and-mcp.connect-provider".to_owned(),
            "connect-method" => "gate.provider-and-mcp.connect-method".to_owned(),
            "connect-oauth" => "gate.provider-and-mcp.connect-oauth".to_owned(),
            "connect-connected" => "gate.provider-and-mcp.connect-connected".to_owned(),
            "mcp" => "gate.provider-and-mcp.mcp".to_owned(),
            "model-picker" => "gate.work-configuration.model-picker".to_owned(),
            "chat-turn" => "flow1.daily-work.stable".to_owned(),
            other => format!("case.{case_id}.after.{other}.tick.{tick}"),
        }
    } else if matches!(
        case_id,
        "turn-error-retry"
            | "turn-error-cancel"
            | "turn-error-inspect-connection"
            | "error-network-to-rate-limit-bridge"
            | "network-offline-review-setting"
            | "network-offline-resume"
            | "network-backoff-cancel"
            | "network-backoff-exhaust"
            | "network-backoff-race-retry-wins"
            | "network-backoff-race-cancel-wins"
            | "network-check-connection"
            | "network-check-connection-return"
            | "network-backoff-snapshot"
            | "network-evidence-snapshot"
            | "rate-limit-automatic-success"
            | "rate-limit-retry-now"
            | "rate-limit-race-retry-wins"
            | "rate-limit-race-cancel-wins"
            | "checkpoint-undo-success"
            | "checkpoint-redo-success"
            | "checkpoint-undo-conflict"
            | "checkpoint-undo-noop"
            | "checkpoint-redo-stale"
            | "checkpoint-continue-chat"
            | "chat-turn-open-checkpoint-undo"
    ) {
        match (case_id, after.screen_id.as_str()) {
            ("turn-error-retry", "chat-turn") => "gate.recovery.chat-turn".to_owned(),
            ("turn-error-cancel", "turn-error") => "gate.recovery.turn-error".to_owned(),
            ("turn-error-inspect-connection", "error-network") => {
                "gate.recovery.error-network".to_owned()
            }
            ("chat-turn-open-checkpoint-undo", "checkpoint-undo") => {
                "gate.recovery.checkpoint-undo".to_owned()
            }
            ("error-network-to-rate-limit-bridge", "rate-limit") => {
                "gate.recovery.rate-limit".to_owned()
            }
            ("network-offline-review-setting", "connect-provider") => {
                "gate.provider-and-mcp.connect-provider".to_owned()
            }
            ("network-offline-resume", "error-network") => "error-network.offline".to_owned(),
            ("network-backoff-cancel", "error-network") => "error-network.exhausted".to_owned(),
            ("network-backoff-exhaust", "error-network") => "error-network.exhausted".to_owned(),
            ("network-backoff-race-retry-wins", "chat-turn") => "gate.recovery.chat-turn".to_owned(),
            ("network-backoff-race-cancel-wins", "error-network") => {
                "error-network.exhausted".to_owned()
            }
            ("network-check-connection", "connect-provider") => {
                "gate.provider-and-mcp.connect-provider".to_owned()
            }
            ("network-check-connection-return", "error-network") => {
                "canonical.error-network".to_owned()
            }
            ("network-backoff-snapshot", "error-network") => "error-network.backoff".to_owned(),
            ("network-evidence-snapshot", "error-network") => "error-network.evidence".to_owned(),
            ("rate-limit-automatic-success", "chat-turn")
            | ("rate-limit-retry-now", "chat-turn")
            | ("rate-limit-race-retry-wins", "chat-turn")
            | ("checkpoint-redo-success", "chat-turn")
            | ("checkpoint-continue-chat", "chat-turn") => "gate.recovery.chat-turn".to_owned(),
            ("rate-limit-race-cancel-wins", "rate-limit") => "rate-limit.cancelled".to_owned(),
            ("checkpoint-undo-success", "checkpoint-undo") => {
                "gate.recovery.checkpoint-undo".to_owned()
            }
            ("checkpoint-undo-conflict", "checkpoint-undo") => {
                "checkpoint-undo.conflict".to_owned()
            }
            ("checkpoint-undo-noop", "checkpoint-undo") => "checkpoint-undo.noop".to_owned(),
            ("checkpoint-redo-stale", "checkpoint-undo") => "checkpoint-undo.stale".to_owned(),
            (_, other) => format!("case.{case_id}.after.{other}.tick.{tick}"),
        }
    } else if matches!(
        case_id,
        "mission-proposal-accept"
            | "mission-resolve-input-ready"
            | "mission-resolve-input-still-blocked"
            | "mission-start"
            | "mission-launch-cancel"
            | "mission-blocked-cancel"
            | "mission-view-progress"
            | "mission-create-fix-task"
            | "mission-fix-working"
            | "mission-fix-validating"
            | "mission-validation-success"
            | "mission-validation-failure"
            | "mission-review-create-fix"
            | "mission-review-fix-validating"
            | "mission-completion-validation"
            | "mission-request-changes-open"
            | "mission-request-changes-submit"
            | "mission-request-changes-cancel"
            | "mission-approve-completion"
            | "mission-fix-loop-rerepair"
            | "mission-monitor-to-failure-bridge"
            | "mission-failure-to-review-bridge"
            | "mission-review-to-approved-bridge"
    ) {
        match (case_id, after.screen_id.as_str()) {
            ("mission-proposal-accept", "workflow-launch") => {
                "workflow-launch.preflight-running".to_owned()
            }
            ("mission-resolve-input-ready", "workflow-launch") => {
                "gate.mission.workflow-launch".to_owned()
            }
            ("mission-resolve-input-still-blocked", "workflow-launch-blocked") => {
                "gate.mission.workflow-launch-blocked".to_owned()
            }
            ("mission-start", "workflow-monitor") => "gate.mission.workflow-monitor".to_owned(),
            ("mission-launch-cancel", "chat-turn") | ("mission-blocked-cancel", "chat-turn") => {
                "case.mission.cancel.after.chat-turn.tick.1".to_owned()
            }
            ("mission-view-progress", "workflow-monitor") => {
                "gate.mission.workflow-monitor".to_owned()
            }
            ("mission-create-fix-task", "workflow-monitor")
            | ("mission-review-create-fix", "workflow-monitor")
            | ("mission-request-changes-submit", "workflow-monitor") => {
                "workflow-monitor.fix-planned".to_owned()
            }
            ("mission-fix-working", "workflow-monitor") => {
                "workflow-monitor.fix-working".to_owned()
            }
            ("mission-fix-validating", "workflow-monitor")
            | ("mission-review-fix-validating", "workflow-monitor") => {
                "workflow-monitor.validating".to_owned()
            }
            ("mission-validation-success", "reviewer-output") => {
                "gate.mission.reviewer-output".to_owned()
            }
            ("mission-validation-failure", "workflow-monitor-failure")
            | ("mission-fix-loop-rerepair", "workflow-monitor-failure") => {
                "gate.mission.workflow-monitor-failure".to_owned()
            }
            ("mission-completion-validation", "reviewer-approved")
            | ("mission-request-changes-cancel", "reviewer-approved") => {
                "gate.mission.reviewer-approved".to_owned()
            }
            ("mission-request-changes-open", "reviewer-approved") => {
                "reviewer-approved.request-changes".to_owned()
            }
            ("mission-approve-completion", "chat-turn") => {
                "case.mission.complete.after.chat-turn.tick.1".to_owned()
            }
            ("mission-monitor-to-failure-bridge", "workflow-monitor-failure") => {
                "gate.mission.workflow-monitor-failure".to_owned()
            }
            ("mission-failure-to-review-bridge", "reviewer-output") => {
                "gate.mission.reviewer-output".to_owned()
            }
            ("mission-review-to-approved-bridge", "reviewer-approved") => {
                "gate.mission.reviewer-approved".to_owned()
            }
            (_, other) => format!("case.{case_id}.after.{other}.tick.{tick}"),
        }
    } else {
        format!("case.{case_id}.tick.{tick}")
    };
    let audit_fixture = if matches!(
        case_id,
        "staging-apply-partial-failure"
            | "staging-apply-failure"
            | "staging-finish-failed"
            | "staging-partial-apply-failure"
            | "staging-partial-failure"
            | "apply-partial-failure"
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
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
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
        path: "the unavailable onboarding choice opens connection, then returns to model-picker without losing setup state"
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

fn apply_flow4_provider_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.provider-connection".to_owned();
    state.transcript_id = "transcript.provider-connection".to_owned();
    state.session_id = "session.provider-connection.1".to_owned();
    state.turn_id = "turn.provider-connection.1".to_owned();
    state.user_task = "Connect the model needed for this conversation.".to_owned();
    state.visible_origin = "Provider connection · blocked model action".to_owned();
    state.draft = "Continue the paused conversation.".to_owned();
    state.transcript = "The blocked model choice remains available.".to_owned();
    state.paused_activity_id = "pending.model.gpt-5-2-codex".to_owned();
    state.state_hash = state_hash(state);
}

fn apply_flow4_mcp_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.mcp-capabilities".to_owned();
    state.transcript_id = "transcript.mcp-capabilities".to_owned();
    state.session_id = "session.mcp-capabilities.1".to_owned();
    state.turn_id = "turn.mcp-capabilities.1".to_owned();
    state.user_task = "Fix the capabilities this conversation needs.".to_owned();
    state.visible_origin = "MCP · conversation capabilities".to_owned();
    state.draft = "Continue the paused conversation.".to_owned();
    state.transcript = "The conversation is waiting for MCP capabilities.".to_owned();
    // Separate capability branch: no pending model apply activity from provider.
    state.paused_activity_id = "pending.capability.browser-automation".to_owned();
    state.state_hash = state_hash(state);
}

const FLOW5_TASK: &str = "Finish the release notes without losing the current work.";
const FLOW5_ORIGIN: &str = "Recovery · paused conversation";
const FLOW5_PARTIAL: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const FLOW5_DRAFT: &str = "Keep the risk note short before we publish.";
const FLOW5_PENDING: &str = "pending.network.release-notes";
const FLOW5_TIMER_OWNER: &str = "timer.network.backoff";
const FLOW5_RATE_TIMER: &str = "timer.rate-limit.wait";
const FLOW5_CHECKPOINT: &str = "checkpoint.before-risk-note-expansion";
const FLOW5_QUEUE: &str = "queued.follow-up.risk-summary";
const FLOW5_FILE_RELEASE_BEFORE: &str = "# Release notes\n- auth race fix\n- retry window change\n- open risk: token refresh race\n";
const FLOW5_FILE_RELEASE_AFTER: &str = "# Release notes\n- auth race fix\n- retry window change\n";
const FLOW5_FILE_RISKS_BEFORE: &str = "# Risks\n- token refresh can race on reconnect\n";
const FLOW5_FILE_RISKS_AFTER: &str = "# Risks\n";
const FLOW5_FILE_UNAFFECTED: &str = "# Later note\nKeep the queue draft for the next turn.\n";
const FLOW5_FILE_RISKS_CONFLICT: &str = "# Risks\n- token refresh can race on reconnect\n- new draft edit after checkpoint\n";

fn apply_flow5_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.recovery".to_owned();
    state.transcript_id = "transcript.recovery".to_owned();
    state.session_id = "session.recovery.1".to_owned();
    state.turn_id = "turn.recovery.1".to_owned();
    state.user_task = FLOW5_TASK.to_owned();
    state.visible_origin = FLOW5_ORIGIN.to_owned();
    state.draft = FLOW5_DRAFT.to_owned();
    state.transcript = format!(
        "You · Summarize the remaining release notes and the still-open risks.\n{FLOW5_PARTIAL}"
    );
    state.partial_response = FLOW5_PARTIAL.to_owned();
    state.paused_activity_id = FLOW5_PENDING.to_owned();
    state.submitted_prompt =
        Some("Summarize the remaining release notes and the still-open risks.".to_owned());
    state.queue = FLOW5_QUEUE.to_owned();
    state.queued_follow_up = "Queue the short risk follow-up after restore.".to_owned();
    state.scroll_anchor = "turn-partial".to_owned();
    state.attachment = "rate-limit-notes.md".to_owned();
    state.state_hash = state_hash(state);
}

const FLOW6_TASK: &str = "Prepare the release safely and review the result.";
const FLOW6_ORIGIN: &str = "Mission · current conversation";
const FLOW6_DRAFT: &str = "Keep the risk note short before we publish.";
const FLOW6_PENDING: &str = "pending.mission.launch";
const FLOW6_SUBMITTED: &str = "Turn the release work into a mission and walk me through it.";
fn apply_flow6_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.mission".to_owned();
    state.transcript_id = "transcript.mission".to_owned();
    state.session_id = "session.mission.1".to_owned();
    state.turn_id = "turn.mission.1".to_owned();
    state.user_task = FLOW6_TASK.to_owned();
    state.visible_origin = FLOW6_ORIGIN.to_owned();
    state.draft = FLOW6_DRAFT.to_owned();
    state.transcript = format!(
        "You · {FLOW6_SUBMITTED}\nI can run that as a small mission with three connected tasks."
    );
    state.partial_response = String::new();
    state.paused_activity_id = FLOW6_PENDING.to_owned();
    state.submitted_prompt = Some(FLOW6_SUBMITTED.to_owned());
    state.queue = String::new();
    state.queued_follow_up = String::new();
    state.scroll_anchor = "mission-plan".to_owned();
    state.attachment = String::new();
    state.state_hash = state_hash(state);
}

fn copy_flow6_identity(from: &ScenarioState, to: &mut ScenarioState) {
    to.conversation_id = from.conversation_id.clone();
    to.transcript_id = from.transcript_id.clone();
    to.session_id = from.session_id.clone();
    to.turn_id = from.turn_id.clone();
    to.user_task = from.user_task.clone();
    to.visible_origin = from.visible_origin.clone();
    to.draft = from.draft.clone();
    to.transcript = from.transcript.clone();
    to.partial_response = from.partial_response.clone();
    to.paused_activity_id = from.paused_activity_id.clone();
    to.submitted_prompt = from.submitted_prompt.clone();
    to.queue = from.queue.clone();
    to.queued_follow_up = from.queued_follow_up.clone();
    to.scroll_anchor = from.scroll_anchor.clone();
    to.attachment = from.attachment.clone();
    to.return_policy = from.return_policy.clone();
    to.state_hash = state_hash(to);
}

fn mission_proposal_accept() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-launch",
        vec!["chat-turn", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "accept-plan",
        "mission-proposal",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    before.selection = "accept-plan".to_owned();
    before.focus = "mission-proposal".to_owned();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "workflow-launch",
        vec!["chat-turn", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "preflight-running",
        "mission-preflight",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "preflight-running".to_owned();
    after.focus = "mission-preflight".to_owned();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.mission.accept-plan".to_owned(),
        label: "Accept plan".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "accepting the proposal starts automatic preflight before launch".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.preflight.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "accepted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.preflight-start",
        "mission-preflight",
        "running",
        "accepting the plan begun automatic preflight without a manual check step",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_resolve_input_ready() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "workflow-launch-blocked"];
    let mut before = state(
        "workflow-launch-blocked",
        origin.clone(),
        FLOW6_DRAFT,
        "mission-plan",
        "missing-input",
        "mission-blocker",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-launch",
        vec!["chat-turn", "workflow-launch-blocked", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "start-mission",
        "mission-launch",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "start-mission".to_owned();
    after.focus = "mission-launch".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-launch-blocked".to_owned(),
        "workflow-launch".to_owned(),
    ];
    // origin_stack preservation check requires before == after stacks
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.mission.resolve-missing-input".to_owned(),
        label: "Add release checklist".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "resolving the missing input reruns automatic preflight before launch".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.preflight-rerun.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "launch-ready".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.preflight-success",
        "mission-preflight",
        "passed",
        "fresh automatic preflight succeeded after the missing input was supplied",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_resolve_input_still_blocked() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "workflow-launch-blocked"];
    let mut before = state(
        "workflow-launch-blocked",
        origin.clone(),
        FLOW6_DRAFT,
        "mission-plan",
        "missing-input",
        "mission-blocker",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.selection = "missing-input".to_owned();
    after.focus = "mission-blocker".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.resolve-missing-input".to_owned(),
        label: "Add release checklist".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "automatic preflight still reports the current concrete missing input".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.preflight-rerun.2".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "still-blocked".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.preflight-still-blocked",
        "mission-preflight",
        "blocked",
        "fresh automatic preflight failed again with the same concrete missing input",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_start() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-launch",
        vec!["chat-turn", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "start-mission",
        "mission-launch",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-launch", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-running",
        "view-progress",
        "mission-monitor",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "view-progress".to_owned();
    after.focus = "mission-monitor".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-launch".to_owned(),
        "workflow-monitor".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.start".to_owned(),
        label: "Start mission".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "explicit launch decision enters compact healthy monitoring".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.launch.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "started".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.started",
        "mission-execution",
        "running",
        "mission started for the release goal with compact monitoring",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_launch_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-launch",
        vec!["chat-turn", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "start-mission",
        "mission-launch",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "workflow-launch"],
        FLOW6_DRAFT,
        "mission-plan",
        "composer",
        "composer",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.pending_action = None;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.launch-cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel returns to the proposing conversation without starting the mission"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.launch-cancel.0".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 0,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.launch-cancel",
        "mission-launch",
        "no-start",
        "cancel preserved the conversation and started no mission work",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_blocked_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-launch-blocked",
        vec!["chat-turn", "workflow-launch-blocked"],
        FLOW6_DRAFT,
        "mission-plan",
        "missing-input",
        "mission-blocker",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "workflow-launch-blocked"],
        FLOW6_DRAFT,
        "mission-plan",
        "composer",
        "composer",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.pending_action = None;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.blocked-cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel returns to the proposing conversation without launching".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.blocked-cancel.0".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 0,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.blocked-cancel",
        "mission-launch",
        "no-start",
        "cancel left the blocked mission unstarted and restored the conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_view_progress() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-launch", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-running",
        "view-progress",
        "mission-monitor",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.focus = "mission-monitor".to_owned();
    after.selection = "view-progress".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.view-progress".to_owned(),
        label: "View progress".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "progress disclosure returns to the same compact monitor".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.view-progress.0".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 0,
        outcome: "progress-open".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.view-progress",
        "mission-monitor",
        "disclosed",
        "opening progress kept the mission identity and compact default state",
    );
    (before, after, action, transition, attempt, effect)
}

const FLOW6_FIX_TASK: &str = "Repair the safe publish path checks";
const FLOW6_VALIDATION_RUN: &str = "validation.release.2";
const FLOW6_COMPLETION_VALIDATION: &str = "validation.release.completion.1";
const FLOW6_FEEDBACK: &str =
    "Add the open risk and rollback order to the next-step note before completion.";

fn mission_create_fix_task() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor-failure",
        vec![
            "chat-turn",
            "workflow-monitor",
            "workflow-monitor-failure",
        ],
        FLOW6_DRAFT,
        "mission-failure",
        "create-fix-task",
        "mission-failure",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec![
            "chat-turn",
            "workflow-monitor",
            "workflow-monitor-failure",
            "workflow-monitor",
        ],
        FLOW6_DRAFT,
        "mission-fix-planned",
        "view-progress",
        "mission-fix-planned",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "fix-planned".to_owned();
    after.focus = "mission-fix-planned".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "workflow-monitor-failure".to_owned(),
        "workflow-monitor".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.create-fix-task".to_owned(),
        label: "Create a fix task".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "creating a fix task records the failure as planned repair work".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.fix-plan.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "fix-planned".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.fix-planned",
        "mission-fix",
        "planned",
        &format!(
            "fix task planned from publish path failure: {FLOW6_FIX_TASK}; completed work preserved"
        ),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_fix_working() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-fix-planned",
        "fix-planned",
        "mission-fix-planned",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-fix-working",
        "fix-working",
        "mission-fix-working",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "fix-working".to_owned();
    after.focus = "mission-fix-working".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.fix-start".to_owned(),
        label: "Start fix work".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "planned fix work becomes visible Working without mutating completed tasks"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.fix-work.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "working".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.fix-working",
        "mission-fix",
        "working",
        &format!("fix task {FLOW6_FIX_TASK} is working on the failed publish path"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_fix_validating() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-fix-working",
        "fix-working",
        "mission-fix-working",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-validating",
        "validating",
        "mission-validating",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "validating".to_owned();
    after.focus = "mission-validating".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.fix-complete-validate".to_owned(),
        label: "Enter validation".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "completing fix work always enters a visible fresh Validating stage".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: format!("attempt.{FLOW6_VALIDATION_RUN}.start"),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "validating".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.validation-start",
        "mission-validation",
        "running",
        &format!("fresh validation run {FLOW6_VALIDATION_RUN} started after repair"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_validation_success() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-validating",
        "validating",
        "mission-validating",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-output",
        vec!["chat-turn", "workflow-monitor", "reviewer-output"],
        FLOW6_DRAFT,
        "mission-review",
        "changes-requested",
        "mission-review",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "create-fix-from-review".to_owned();
    after.focus = "mission-review".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "reviewer-output".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.validation-finished".to_owned(),
        label: "Validation finished".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "successful fresh validation advances the mission to review".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: FLOW6_VALIDATION_RUN.to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "passed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.validation-passed",
        "mission-validation",
        "passed",
        &format!("validation run {FLOW6_VALIDATION_RUN} passed; review opened for remaining feedback"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_validation_failure() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-validating",
        "validating",
        "mission-validating",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor-failure",
        vec![
            "chat-turn",
            "workflow-monitor",
            "workflow-monitor-failure",
        ],
        FLOW6_DRAFT,
        "mission-failure",
        "create-fix-task",
        "mission-failure",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "create-fix-task".to_owned();
    after.focus = "mission-failure".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "workflow-monitor-failure".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.validation-finished".to_owned(),
        label: "Validation finished".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "failed fresh validation returns the mission to Needs attention".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: format!("{FLOW6_VALIDATION_RUN}.fail"),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "failed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.validation-failed",
        "mission-validation",
        "failed",
        &format!("validation run {FLOW6_VALIDATION_RUN} failed and reopened Needs attention"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_review_create_fix() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-output",
        vec!["chat-turn", "workflow-monitor", "reviewer-output"],
        FLOW6_DRAFT,
        "mission-review",
        "create-fix-from-review",
        "mission-review",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec![
            "chat-turn",
            "workflow-monitor",
            "reviewer-output",
            "workflow-monitor",
        ],
        FLOW6_DRAFT,
        "mission-fix-planned",
        "fix-planned",
        "mission-fix-planned",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "fix-planned".to_owned();
    after.focus = "mission-fix-planned".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "reviewer-output".to_owned(),
        "workflow-monitor".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.review-create-fix".to_owned(),
        label: "Create a fix task".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "requested changes become an explicit fix task followed by fresh validation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.review-fix.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "fix-planned".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.review-fix-planned",
        "mission-fix",
        "planned",
        &format!("reviewer feedback attached to fix task: {FLOW6_FIX_TASK}"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_review_fix_validating() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    // After review-created fix work, completing it re-enters Validating.
    mission_fix_validating()
}

fn mission_completion_validation() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-validating",
        "validating",
        "mission-validating",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "approve-completion",
        "mission-human-gate",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "approve-completion".to_owned();
    after.focus = "mission-human-gate".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "reviewer-approved".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.completion-validation".to_owned(),
        label: "Completion validation".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "fresh completion validation on the post-repair revision opens human approval"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: FLOW6_COMPLETION_VALIDATION.to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "passed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.completion-validation",
        "mission-validation",
        "passed",
        &format!(
            "completion validation {FLOW6_COMPLETION_VALIDATION} passed against the final post-repair state"
        ),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_request_changes_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "approve-completion",
        "mission-human-gate",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "request-changes",
        "mission-change-reason",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "request-changes".to_owned();
    after.focus = "mission-change-reason".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.request-changes".to_owned(),
        label: "Request changes".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "request changes opens an anchored reason dock on the same gate".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.request-changes.open".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 0,
        outcome: "reason-dock-open".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.request-changes-open",
        "mission-human-feedback",
        "open",
        "reason dock opened without completing the mission",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_request_changes_submit() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "request-changes",
        "mission-change-reason",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor",
        vec![
            "chat-turn",
            "workflow-monitor",
            "reviewer-approved",
            "workflow-monitor",
        ],
        FLOW6_DRAFT,
        "mission-fix-planned",
        "fix-planned",
        "mission-fix-planned",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "fix-planned".to_owned();
    after.focus = "mission-fix-planned".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "reviewer-approved".to_owned(),
        "workflow-monitor".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.request-changes-submit".to_owned(),
        label: "Submit reason".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "submitted human feedback creates or updates a fix task then revalidation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.request-changes.submit".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "fix-planned".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.human-feedback-fix",
        "mission-fix",
        "planned",
        &format!("human feedback retained on fix task: {FLOW6_FEEDBACK}"),
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_request_changes_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "request-changes",
        "mission-change-reason",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "approve-completion",
        "mission-human-gate",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "approve-completion".to_owned();
    after.focus = "mission-human-gate".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.request-changes-cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel restores the unchanged human approval gate".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.request-changes.cancel".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 0,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.request-changes-cancel",
        "mission-human-feedback",
        "cancelled",
        "reason dock cancelled; Approve completion remains required",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_approve_completion() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-approved",
        "approve-completion",
        "mission-human-gate",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        FLOW6_DRAFT,
        "mission-complete",
        "composer",
        "composer",
        None,
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "composer".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.transcript = format!(
        "You · {FLOW6_SUBMITTED}\nI can run that as a small mission with three connected tasks.\nMission complete · release is ready for the next conversation step."
    );
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.approve-completion".to_owned(),
        label: "Approve completion".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "explicit human approval marks the mission complete and returns to conversation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mission.approve-completion.1".to_owned(),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "completed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.completed",
        "mission-completion",
        "completed",
        "human approval finished the mission and restored the originating conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_fix_loop_rerepair() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    // Second validation re-failure keeps goal, session, and dependency identity.
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-monitor"],
        FLOW6_DRAFT,
        "mission-validating",
        "validating",
        "mission-validating",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor-failure",
        vec![
            "chat-turn",
            "workflow-monitor",
            "workflow-monitor-failure",
        ],
        FLOW6_DRAFT,
        "mission-failure",
        "create-fix-task",
        "mission-failure",
        Some(FLOW6_PENDING),
        "return-to-conversation",
        1,
    );
    copy_flow6_identity(&before, &mut after);
    after.selection = "create-fix-task".to_owned();
    after.focus = "mission-failure".to_owned();
    after.origin_stack = vec![
        "chat-turn".to_owned(),
        "workflow-monitor".to_owned(),
        "workflow-monitor-failure".to_owned(),
    ];
    before.origin_stack = after.origin_stack.clone();
    before.state_hash = state_hash(&before);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mission.validation-finished".to_owned(),
        label: "Validation finished".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "second failed validation reopens Needs attention with stable identities".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: format!("{FLOW6_VALIDATION_RUN}.retry.fail"),
        owner_id: FLOW6_PENDING.to_owned(),
        count: 1,
        outcome: "failed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mission.validation-refailed",
        "mission-validation",
        "failed",
        "revalidation failed again; goal, completed work, and fix identity remained stable",
    );
    (before, after, action, transition, attempt, effect)
}

fn copy_flow5_identity(from: &ScenarioState, to: &mut ScenarioState) {
    to.conversation_id = from.conversation_id.clone();
    to.transcript_id = from.transcript_id.clone();
    to.session_id = from.session_id.clone();
    to.turn_id = from.turn_id.clone();
    to.user_task = from.user_task.clone();
    to.visible_origin = from.visible_origin.clone();
    to.draft = from.draft.clone();
    to.transcript = from.transcript.clone();
    to.partial_response = from.partial_response.clone();
    to.paused_activity_id = from.paused_activity_id.clone();
    to.submitted_prompt = from.submitted_prompt.clone();
    to.queue = from.queue.clone();
    to.queued_follow_up = from.queued_follow_up.clone();
    to.scroll_anchor = from.scroll_anchor.clone();
    to.attachment = from.attachment.clone();
    to.selection = from.selection.clone();
    to.pending_action = from.pending_action.clone();
    to.return_policy = from.return_policy.clone();
    to.state_hash = state_hash(to);
}

fn flow5_file_hash(contents: &str) -> String {
    sha256_hex(contents.as_bytes())
}

fn insert_checkpoint_file_hashes(
    hashes: &mut BTreeMap<String, PreservedFieldHash>,
    release_before: &str,
    release_after: &str,
    risks_before: &str,
    risks_after: &str,
    checkpoint_before: &str,
    checkpoint_after: &str,
) {
    hashes.insert(
        "file.affected.notes/release.md".to_owned(),
        PreservedFieldHash {
            before: flow5_file_hash(release_before),
            after: flow5_file_hash(release_after),
        },
    );
    hashes.insert(
        "file.affected.notes/risks.md".to_owned(),
        PreservedFieldHash {
            before: flow5_file_hash(risks_before),
            after: flow5_file_hash(risks_after),
        },
    );
    hashes.insert(
        "file.unaffected.drafts/later-note.md".to_owned(),
        PreservedFieldHash {
            before: flow5_file_hash(FLOW5_FILE_UNAFFECTED),
            after: flow5_file_hash(FLOW5_FILE_UNAFFECTED),
        },
    );
    hashes.insert(
        "checkpoint".to_owned(),
        PreservedFieldHash {
            before: sha256_hex(checkpoint_before.as_bytes()),
            after: sha256_hex(checkpoint_after.as_bytes()),
        },
    );
}

fn turn_error_retry() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "turn-error",
        vec!["chat-turn", "turn-error"],
        FLOW5_DRAFT,
        "turn-partial",
        "retry-reply",
        "turn-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "turn-error"],
        FLOW5_DRAFT,
        "turn-partial",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = None;
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.turn-error.retry".to_owned(),
        label: "Retry reply".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "successful turn retry restores the same conversation with a new reply".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.turn.retry.1".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.turn.retry",
        "turn-recovery",
        "simulated-success",
        "one manual turn retry restored the conversation shell",
    );
    (before, after, action, transition, attempt, effect)
}

fn turn_error_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "turn-error",
        vec!["chat-turn", "turn-error"],
        FLOW5_DRAFT,
        "turn-partial",
        "cancel-keep-failure",
        "turn-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.focus = "turn-recovery".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.turn-error.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel keeps the partial reply and failure record in place".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.turn.cancel.1".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.turn.cancel",
        "turn-recovery",
        "cancelled-without-retry",
        "cancel preserved the interrupted turn without a second request",
    );
    (before, after, action, transition, attempt, effect)
}

fn turn_error_inspect_connection() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "turn-error", "error-network"];
    let mut before = state(
        "turn-error",
        origin.clone(),
        FLOW5_DRAFT,
        "turn-partial",
        "inspect-connection",
        "turn-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = state(
        "error-network",
        origin,
        FLOW5_DRAFT,
        "turn-partial",
        "retry-or-wait",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "network-recovery".to_owned();
    after.selection = "retry-or-wait".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.turn-error.inspect-connection".to_owned(),
        label: "Inspect the connection problem".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "inspecting the connection problem opens the network recovery state".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.turn.inspect.1".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.turn.inspect-connection",
        "network-diagnosis",
        "opened",
        "connection diagnosis opens without rewriting transcript, draft, or partial reply",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_offline_review_setting() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "error-network", "connect-provider"];
    let mut before = state(
        "error-network",
        origin.clone(),
        FLOW5_DRAFT,
        "network-offline",
        "review-setting",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.selection = "offline".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = state(
        "connect-provider",
        origin,
        FLOW5_DRAFT,
        "network-offline",
        "openai",
        "provider-catalogue",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "provider-catalogue".to_owned();
    after.selection = "openai".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.review-setting".to_owned(),
        label: "Review network setting".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "offline recovery opens connection review with the paused model action as origin"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.offline.0".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.offline.review",
        "connection-inspection",
        "no-submit",
        "review network setting opened without a provider attempt",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_offline_resume() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "error-network", "connect-provider"];
    let mut before = state(
        "connect-provider",
        origin.clone(),
        FLOW5_DRAFT,
        "network-offline",
        "openai",
        "provider-catalogue",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.state_hash = state_hash(&before);
    let mut after = state(
        "error-network",
        origin,
        FLOW5_DRAFT,
        "network-offline",
        "offline",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "network-recovery".to_owned();
    after.selection = "offline".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.connect-provider.back".to_owned(),
        label: "Back".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "provider resume policy returns to the exact offline recovery origin".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.offline.resume.0".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.offline.resume",
        "connection-inspection",
        "no-submit",
        "return restored offline recovery without submitting the paused request",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_backoff_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "automatic-backoff",
        "recovery-wait",
        Some(FLOW5_TIMER_OWNER),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_TIMER_OWNER.to_owned());
    before.state_hash = state_hash(&before);
    let mut after = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "manual-retry",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = Some(FLOW5_PENDING.to_owned());
    after.focus = "network-recovery".to_owned();
    after.selection = "manual-retry".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.cancel-backoff".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel stops the single backoff owner and returns a stable preserved origin"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.backoff.cancel".to_owned(),
        owner_id: FLOW5_TIMER_OWNER.to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.backoff.cancel",
        "provider-attempt",
        "cancelled",
        "cancel stopped future automatic network attempts",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_backoff_exhaust() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "automatic-backoff",
        "recovery-wait",
        Some(FLOW5_TIMER_OWNER),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_TIMER_OWNER.to_owned());
    before.state_hash = state_hash(&before);
    let mut after = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "manual-retry",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = Some(FLOW5_PENDING.to_owned());
    after.focus = "network-recovery".to_owned();
    after.selection = "manual-retry".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.backoff-tick".to_owned(),
        label: "Automatic backoff".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "the single backoff owner reaches the declared attempt limit and stabilizes"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.backoff.3".to_owned(),
        owner_id: FLOW5_TIMER_OWNER.to_owned(),
        count: 3,
        outcome: "exhausted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.backoff.exhaust",
        "provider-attempt",
        "exhausted",
        "bounded network backoff exhausted without further automatic activity",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_backoff_race_retry_wins() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "automatic-backoff",
        "recovery-wait",
        Some(FLOW5_TIMER_OWNER),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_TIMER_OWNER.to_owned());
    before.state_hash = state_hash(&before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = None;
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.retry-now".to_owned(),
        label: "Retry now".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "same-tick retry-now wins once; timer expiry becomes inert".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.race.retry".to_owned(),
        owner_id: FLOW5_TIMER_OWNER.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.race.retry",
        "provider-attempt",
        "one-winner",
        "retry-now and timer expiry raced; only retry-now produced a successor",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_backoff_race_cancel_wins() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "automatic-backoff",
        "recovery-wait",
        Some(FLOW5_TIMER_OWNER),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_TIMER_OWNER.to_owned());
    before.state_hash = state_hash(&before);
    let mut after = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "manual-retry",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = Some(FLOW5_PENDING.to_owned());
    after.focus = "network-recovery".to_owned();
    after.selection = "manual-retry".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.cancel-backoff".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "same-tick cancel wins once; timer expiry becomes inert".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.race.cancel".to_owned(),
        owner_id: FLOW5_TIMER_OWNER.to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.race.cancel",
        "provider-attempt",
        "one-winner",
        "cancel and timer expiry raced; only cancel produced a successor",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_backoff_snapshot() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    // Stable snapshot of automatic backoff for visual/case evidence (self-loop tick).
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-backoff",
        "automatic-backoff",
        "recovery-wait",
        Some(FLOW5_TIMER_OWNER),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_TIMER_OWNER.to_owned());
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.backoff-tick".to_owned(),
        label: "Automatic backoff".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "the single backoff owner remains active within the attempt limit".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.backoff.2".to_owned(),
        owner_id: FLOW5_TIMER_OWNER.to_owned(),
        count: 1,
        outcome: "waiting".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.backoff.snapshot",
        "provider-attempt",
        "waiting",
        "deterministic backoff snapshot without side-effect mutation",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_evidence_snapshot() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "error-network"],
        FLOW5_DRAFT,
        "network-evidence",
        "evidence",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.selection = "evidence".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.show-evidence".to_owned(),
        label: "Show evidence".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "evidence discloses attempts and endpoint detail on request".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.evidence.0".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.evidence.snapshot",
        "disclosure",
        "expanded",
        "technical evidence disclosed without resubmitting the paused request",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_check_connection() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "error-network", "connect-provider"];
    let mut before = state(
        "error-network",
        origin.clone(),
        FLOW5_DRAFT,
        "network-check",
        "check-connection",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = state(
        "connect-provider",
        origin,
        FLOW5_DRAFT,
        "network-check",
        "openai",
        "provider-catalogue",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "provider-catalogue".to_owned();
    after.selection = "openai".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.error-network.check-connection".to_owned(),
        label: "Check connection".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "connection inspection opens with the failed model action as origin".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.check.0".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.check-connection",
        "connection-inspection",
        "no-submit",
        "check connection did not submit the paused request",
    );
    (before, after, action, transition, attempt, effect)
}

fn network_check_connection_return() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "error-network", "connect-provider"];
    let mut before = state(
        "connect-provider",
        origin.clone(),
        FLOW5_DRAFT,
        "network-check",
        "openai",
        "provider-catalogue",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.state_hash = state_hash(&before);
    let mut after = state(
        "error-network",
        origin,
        FLOW5_DRAFT,
        "network-check",
        "check-connection",
        "network-recovery",
        Some(FLOW5_PENDING),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "network-recovery".to_owned();
    after.selection = "check-connection".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.connect-provider.back".to_owned(),
        label: "Back".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "connection inspection returns to the same paused network recovery without resubmit"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.network.check.return.0".to_owned(),
        owner_id: FLOW5_PENDING.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.network.check-connection.return",
        "connection-inspection",
        "no-submit",
        "return restored network recovery without submitting the paused request",
    );
    (before, after, action, transition, attempt, effect)
}

fn rate_limit_waiting_before(virtual_tick: u32) -> ScenarioState {
    let mut before = state(
        "rate-limit",
        vec!["chat-turn", "error-network", "rate-limit"],
        FLOW5_DRAFT,
        "turn-partial",
        "automatic-wait",
        "recovery-wait",
        Some(FLOW5_RATE_TIMER),
        "return-to-paused-turn",
        virtual_tick,
    );
    apply_flow5_identity(&mut before);
    before.pending_action = Some(FLOW5_RATE_TIMER.to_owned());
    before.state_hash = state_hash(&before);
    before
}

fn rate_limit_chat_after(from: &ScenarioState) -> ScenarioState {
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "error-network", "rate-limit"],
        FLOW5_DRAFT,
        "turn-partial",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(from, &mut after);
    after.pending_action = None;
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.state_hash = state_hash(&after);
    after
}

fn rate_limit_automatic_success() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = rate_limit_waiting_before(0);
    let after = rate_limit_chat_after(&before);
    let action = ActionEvidence {
        action_id: "action.rate-limit.timer-tick".to_owned(),
        label: "Automatic rate-limit wait".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "countdown completion triggers exactly one provider attempt and restores chat"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.rate-limit.timer.1".to_owned(),
        owner_id: FLOW5_RATE_TIMER.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.rate-limit.timer.success",
        "provider-attempt",
        "one-attempt",
        "automatic rate-limit wait completed with one provider resume",
    );
    (before, after, action, transition, attempt, effect)
}

fn rate_limit_retry_now() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = rate_limit_waiting_before(0);
    let after = rate_limit_chat_after(&before);
    let action = ActionEvidence {
        action_id: "action.rate-limit.retry-now".to_owned(),
        label: "Retry now".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "optional manual retry interrupts the countdown once and restores chat".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.rate-limit.retry-now.1".to_owned(),
        owner_id: FLOW5_RATE_TIMER.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.rate-limit.retry-now",
        "provider-attempt",
        "one-attempt",
        "optional Retry now produced the only provider attempt",
    );
    (before, after, action, transition, attempt, effect)
}

fn rate_limit_race_retry_wins() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = rate_limit_waiting_before(0);
    let after = rate_limit_chat_after(&before);
    let action = ActionEvidence {
        action_id: "action.rate-limit.retry-now".to_owned(),
        label: "Retry now".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "same-tick retry-now wins once; timer expiry becomes inert".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.rate-limit.race.retry".to_owned(),
        owner_id: FLOW5_RATE_TIMER.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.rate-limit.race.retry",
        "provider-attempt",
        "one-winner",
        "retry-now and timer expiry raced; only retry-now produced a successor",
    );
    (before, after, action, transition, attempt, effect)
}

fn rate_limit_race_cancel_wins() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let before = rate_limit_waiting_before(0);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.pending_action = None;
    after.focus = "rate-limit-recovery".to_owned();
    after.selection = "cancelled-wait".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.rate-limit.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "same-tick cancel wins once; timer expiry becomes inert".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.rate-limit.race.cancel".to_owned(),
        owner_id: FLOW5_RATE_TIMER.to_owned(),
        count: 0,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.rate-limit.race.cancel",
        "provider-attempt",
        "one-winner",
        "cancel and timer expiry raced; only cancel produced a successor",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_undo_success() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "chat-turn",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "checkpoint-menu",
        "composer",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "composer".to_owned();
    before.selection = "checkpoint-menu".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "redo-change",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "checkpoint-confirmation".to_owned();
    after.selection = "redo-change".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.restore".to_owned(),
        label: "Restore checkpoint".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "restore rewrites only the two checkpoint files and opens one inline confirmation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.restore.1".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.restore",
        "file-restore",
        "restored",
        "exact restore of notes/release.md and notes/risks.md; later-note untouched",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_redo_success() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "redo-change",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "checkpoint-confirmation".to_owned();
    before.selection = "redo-change".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = None;
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.redo".to_owned(),
        label: "Redo change".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "redo reapplies the same checkpoint and restores the chat composer".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.redo.1".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 1,
        outcome: "redone".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.redo",
        "file-restore",
        "redone",
        "symmetric redo rewrote only the same two affected files",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_undo_conflict() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "restore-conflict",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "checkpoint-confirmation".to_owned();
    before.selection = "restore-conflict".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.selection = "review-conflict".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.restore".to_owned(),
        label: "Restore checkpoint".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "conflict refuses restoration without writing any file and keeps the checkpoint"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.conflict.0".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 0,
        outcome: "conflict".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.conflict",
        "file-restore",
        "conflict-refused",
        "changed target files blocked restore; zero filesystem writes",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_undo_noop() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "already-restored",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "checkpoint-confirmation".to_owned();
    before.selection = "already-restored".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.restore".to_owned(),
        label: "Restore checkpoint".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "repeating an already completed undo is an explicit no-op without rewrite".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.noop.0".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 0,
        outcome: "no-op".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.noop",
        "file-restore",
        "no-op",
        "already restored files and confirmation produced no further writes",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_redo_stale() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "stale-redo",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "checkpoint-confirmation".to_owned();
    before.selection = "stale-redo".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.virtual_tick = 1;
    after.selection = "continue-chat".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.invalidate-redo".to_owned(),
        label: "Fresh mutation".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "a fresh mutation invalidates redo and removes the redo action".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.stale.0".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 0,
        outcome: "stale".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.stale-redo",
        "file-restore",
        "stale",
        "later file mutation removed redo without changing unrelated work",
    );
    (before, after, action, transition, attempt, effect)
}

fn checkpoint_continue_chat() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "checkpoint-undo",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "continue-chat",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "checkpoint-confirmation".to_owned();
    before.selection = "continue-chat".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = state(
        "chat-turn",
        vec!["chat-turn", "checkpoint-undo"],
        FLOW5_DRAFT,
        "turn-partial",
        "composer",
        "composer",
        None,
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.pending_action = None;
    after.focus = "composer".to_owned();
    after.selection = "composer".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.checkpoint-undo.continue".to_owned(),
        label: "Continue in conversation".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "continue returns to the exact prior chat position with a restored composer"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.continue.0".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.continue",
        "navigation",
        "returned",
        "return restored the ordinary composer without rewriting files",
    );
    (before, after, action, transition, attempt, effect)
}

fn chat_turn_open_checkpoint_undo() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "checkpoint-undo"];
    let mut before = state(
        "chat-turn",
        origin.clone(),
        FLOW5_DRAFT,
        "turn-partial",
        "open-checkpoint-undo",
        "composer",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    before.focus = "composer".to_owned();
    before.selection = "open-checkpoint-undo".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = state(
        "checkpoint-undo",
        origin,
        FLOW5_DRAFT,
        "turn-partial",
        "redo-change",
        "checkpoint-confirmation",
        Some(FLOW5_CHECKPOINT),
        "return-to-paused-turn",
        1,
    );
    copy_flow5_identity(&before, &mut after);
    after.focus = "checkpoint-confirmation".to_owned();
    after.selection = "redo-change".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.chat-turn.open-checkpoint-undo".to_owned(),
        label: "Undo last checkpoint".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the reversible checkpoint action opens beside the same recovered conversation"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.checkpoint.open.0".to_owned(),
        owner_id: FLOW5_CHECKPOINT.to_owned(),
        count: 0,
        outcome: "none".to_owned(),
    };
    let effect = simulated_effect(
        "effect.checkpoint.open",
        "navigation",
        "opened",
        "opening checkpoint undo preserves draft, transcript, and completed work",
    );
    (before, after, action, transition, attempt, effect)
}

fn login_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "login",
        vec!["model-picker", "login"],
        "Continue the paused conversation.",
        "blocked-model",
        "gpt-5-2-codex",
        "login-decision",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "model-picker",
        vec!["model-picker", "login"],
        "Continue the paused conversation.",
        "blocked-model",
        "gpt-5-2-codex",
        "model-selection",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-chat-turn",
        1,
    );
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.selection = before.selection.clone();
    after.paused_activity_id = before.paused_activity_id.clone();
    after.submitted_prompt = before.submitted_prompt.clone();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.login.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel restores the blocked model origin without starting authentication".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.login.cancel.1".to_owned(),
        owner_id: "pending.model.gpt-5-2-codex".to_owned(),
        count: 0,
        outcome: "cancelled-without-connection".to_owned(),
    };
    let effect = simulated_effect(
        "effect.login.cancel",
        "provider-connection",
        "no-change",
        "login cancel changed no provider credentials",
    );
    (before, after, action, transition, attempt, effect)
}

fn connect_provider_select() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["model-picker", "login", "connect-provider"];
    let mut before = state(
        "connect-provider",
        origin.clone(),
        "Continue the paused conversation.",
        "blocked-model",
        "openai",
        "provider-choice",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "connect-method",
        origin,
        "Continue the paused conversation.",
        "blocked-model",
        "browser-sign-in",
        "method-choice",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        1,
    );
    apply_flow4_provider_identity(&mut after);
    let action = ActionEvidence {
        action_id: "action.connect-provider.connect".to_owned(),
        label: "Connect OpenAI".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the chosen provider exposes its applicable methods".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.connect-provider.select.1".to_owned(),
        owner_id: "pending.provider.openai".to_owned(),
        count: 0,
        outcome: "method-choice-opened".to_owned(),
    };
    let effect = simulated_effect(
        "effect.connect-provider.select",
        "provider-connection",
        "provider-selected",
        "OpenAI was selected without connecting yet",
    );
    (before, after, action, transition, attempt, effect)
}

fn connect_method_back() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["model-picker", "login", "connect-provider", "connect-method"];
    let mut before = state(
        "connect-method",
        origin.clone(),
        "Continue the paused conversation.",
        "blocked-model",
        "browser-sign-in",
        "method-choice",
        Some("pending.provider.openai"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "connect-provider",
        origin,
        "Continue the paused conversation.",
        "blocked-model",
        "openai",
        "provider-choice",
        Some("pending.provider.openai"),
        "return-to-model-picker",
        1,
    );
    apply_flow4_provider_identity(&mut after);
    let action = ActionEvidence {
        action_id: "action.connect-method.back".to_owned(),
        label: "Back".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "back preserves the chosen OpenAI provider without discarding the origin".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.connect-method.back.1".to_owned(),
        owner_id: "pending.provider.openai".to_owned(),
        count: 0,
        outcome: "returned-to-provider-catalogue".to_owned(),
    };
    let effect = simulated_effect(
        "effect.connect-method.back",
        "provider-connection",
        "no-change",
        "method back kept provider selection and credentials unchanged",
    );
    (before, after, action, transition, attempt, effect)
}

fn connect_connected_return() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec![
        "model-picker",
        "login",
        "connect-provider",
        "connect-method",
        "connect-oauth",
        "connect-connected",
    ];
    let mut before = state(
        "connect-connected",
        origin.clone(),
        "Continue the paused conversation.",
        "blocked-model",
        "openai-connected",
        "connection-success",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "model-picker",
        origin,
        "Continue the paused conversation.",
        "blocked-model",
        "gpt-5-2-codex",
        "model-selection",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-chat-turn",
        1,
    );
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.paused_activity_id = before.paused_activity_id.clone();
    after.submitted_prompt = before.submitted_prompt.clone();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.connect-connected.return".to_owned(),
        label: "Return to model choices".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "provider success returns to its blocked model origin before any MCP action"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.connect-connected.return.1".to_owned(),
        owner_id: "pending.model.gpt-5-2-codex".to_owned(),
        count: 1,
        outcome: "returned-to-blocked-model".to_owned(),
    };
    let effect = simulated_effect(
        "effect.connect-connected.return",
        "provider-connection",
        "provider-connected",
        "success restored the blocked model action without opening MCP",
    );
    (before, after, action, transition, attempt, effect)
}

fn mcp_return() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["chat-turn", "mcp"];
    let mut before = state(
        "mcp",
        origin.clone(),
        "Continue the paused conversation.",
        "conversation-capabilities",
        "browser-automation",
        "capability-list",
        Some("pending.capability.browser-automation"),
        "return-to-chat-turn",
        0,
    );
    apply_flow4_mcp_identity(&mut before);
    let mut after = state(
        "chat-turn",
        origin,
        "Continue the paused conversation.",
        "conversation-capabilities",
        "composer",
        "composer",
        None,
        "return-to-chat-turn",
        1,
    );
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.paused_activity_id = before.paused_activity_id.clone();
    after.submitted_prompt = before.submitted_prompt.clone();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.mcp.return".to_owned(),
        label: "Return to conversation".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the capability view closes to its recorded conversation origin".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.mcp.return.1".to_owned(),
        owner_id: "pending.capability.browser-automation".to_owned(),
        count: 0,
        outcome: "returned-to-conversation".to_owned(),
    };
    let effect = simulated_effect(
        "effect.mcp.return",
        "mcp-capability",
        "no-change",
        "closing MCP restored the conversation without provider adjacency",
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
        path: "the unavailable model opens connection and returns to model-picker while retaining the pending model and draft"
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

fn model_to_work_style_bridge() -> (
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
        "Start the first conversation after configuration.",
        "first-run",
        "claude-opus-4-8",
        "model-selection",
        Some("pending.model.claude-opus-4-8"),
        "return-to-onboarding",
        0,
    );
    apply_flow3_onboarding_identity(&mut before);
    let mut after = state(
        "agent-picker",
        vec!["onboarding", "model-picker"],
        "Start the first conversation after configuration.",
        "first-run",
        "autonomous-changes",
        "work-style-selection",
        Some("pending.work-style.autonomous"),
        "return-to-configuration",
        1,
    );
    apply_flow3_current_identity(&mut after);
    let action = ActionEvidence {
        action_id: "action.storyboard.open-work-style-after-model".to_owned(),
        label: "Review how Code should work".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "illustrative_detour".to_owned(),
        path: "after the model applies and returns to onboarding, the explicit next configuration action opens How Code should work for the current conversation chapter"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.open-work-style.1".to_owned(),
        owner_id: "onboarding.configuration".to_owned(),
        count: 1,
        outcome: "work-style-opened-after-model-return".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.open-work-style",
        "storyboard-navigation",
        "illustrative-detour",
        "the applied model returned to onboarding before the user opened the work-style choice",
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
    if state.return_policy != "return-to-model-picker" {
        return Err(format!(
            "blocked-model recovery must return-to-model-picker, got {}",
            state.return_policy
        ));
    }
    crate::audit::blocked_model_recovery_audit(
        &fixture_id,
        &state.visible_origin,
        origin,
        "Return to model choices",
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
        vec!["onboarding", "agent-picker", "mode-switch"],
        "Start the first conversation after configuration.",
        "first-run",
        "autonomous-changes",
        "access-confirmation",
        Some("pending.work-style.autonomous"),
        "return-to-configuration",
        0,
    );
    before.pending_work_style = Some("Autonomous changes".to_owned());
    apply_flow3_current_identity(&mut before);
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
            "cancel discards the pending access change and returns to the work-style choice with Careful edits still applied while preserving the current conversation origin"
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
        action_id: REFUSAL_UNCHANGED_ACTION_ID.to_owned(),
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

fn mode_switch_to_session_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "mode-switch",
        vec!["onboarding", "agent-picker", "mode-switch"],
        "Start the first conversation after configuration.",
        "first-run",
        "autonomous-changes",
        "access-confirmation",
        Some("pending.work-style.autonomous"),
        "return-to-configuration",
        0,
    );
    before.pending_work_style = Some("Autonomous changes".to_owned());
    apply_flow3_current_identity(&mut before);
    before.state_hash = state_hash(&before);
    let mut after = session_state(
        "session-picker",
        "all",
        "saved-conversations",
        Some("pending.session-picker"),
        "return-to-configuration",
        1,
    );
    apply_flow3_resume_identity(&mut after);
    after.origin_stack = before.origin_stack.clone();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.storyboard.open-sessions-after-work-style".to_owned(),
        label: "Choose a saved conversation".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "illustrative_detour".to_owned(),
        path: "after the confirmed work style returns to configuration, the explicit session action opens saved conversations for the current conversation chapter"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.open-sessions.1".to_owned(),
        owner_id: "onboarding.configuration".to_owned(),
        count: 1,
        outcome: "session-picker-opened-after-work-style-return".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.open-sessions",
        "storyboard-navigation",
        "illustrative-detour",
        "the applied work style returned to onboarding before the user opened saved conversations",
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
        vec!["configuration", "session-picker"],
        "Continue the current draft while choosing a saved conversation.",
        "resume-list",
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
    apply_flow3_resume_identity(&mut state);
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
    before.requested_command =
        String::from_utf8(crate::screens::input_and_approval::LONG_SHELL_COMMAND.to_vec())
            .expect("long permission command must be UTF-8");
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = format!("long-command-page-{after_offset}");
    after.focus = "command-detail".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let opens_final_page = after_offset + PERMISSION_PAGE_SIZE
        >= crate::screens::input_and_approval::LONG_SHELL_COMMAND.len();
    let action = ActionEvidence {
        action_id: if opens_final_page {
            "action.permission.command-next-final"
        } else {
            "action.permission.command-next"
        }
        .to_owned(),
        label: if opens_final_page {
            "Open final page"
        } else {
            "Next page"
        }
        .to_owned(),
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

fn permission_shell_long_final() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let (mut before, _, _, _, _, _) = permission_shell_long_page(44, 88);
    before.selection = "long-command-page-44".to_owned();
    before.focus = "command-detail".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "long-command-page-88".to_owned();
    after.focus = "command-detail".to_owned();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.permission.command-next-final".to_owned(),
        label: "Open final page".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the final-page transition opens bytes 88 through the exact command boundary and disables Next page"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.permission.long-page.final.1".to_owned(),
        owner_id: "pending.shell.project-check".to_owned(),
        count: 1,
        outcome: "final-page-inspected".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.long-page.final",
        "permission-disclosure",
        "no-execution",
        "final command page navigation did not execute the command",
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
    let (mut before, _, _, _, _, _) = permission_shell_long_final();
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
    let mut before = before;
    before.requested_command =
        String::from_utf8(crate::screens::input_and_approval::UNSAFE_SHELL_COMMAND.to_vec())
            .expect("unsafe permission fixture bytes must remain valid UTF-8");
    before.state_hash = state_hash(&before);
    let mut after = after;
    after.requested_command = before.requested_command.clone();
    after.state_hash = state_hash(&after);
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

fn disclosure_base(
    screen_id: &str,
    draft: &str,
    scroll_anchor: &str,
    selection: &str,
    focus: &str,
    pending: Option<&str>,
    tick: u32,
) -> ScenarioState {
    let mut s = state(
        screen_id,
        vec!["home", "chat-turn", "streaming", "tool-running"],
        draft,
        scroll_anchor,
        selection,
        focus,
        pending,
        "return-to-same-turn",
        tick,
    );
    s.transcript =
        "User asked for the rate-limit path. Code found the shared request guard.".to_owned();
    s.queued_follow_up = "Also list the tests that should cover the search.".to_owned();
    s.draft = draft.to_owned();
    s.state_hash = state_hash(&s);
    s
}

fn reasoning_expand() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "reasoning-collapsed",
        "Follow up after reading the rationale.",
        "turn-reasoning-rate-limit",
        "reasoning-summary",
        "reasoning-expand",
        Some("disclosure.reasoning"),
        0,
    );
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "reasoning".to_owned();
    after.selection = "reasoning-detail".to_owned();
    after.focus = "reasoning-collapse".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.reasoning.expand".to_owned(),
        label: "Expand rationale".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "expanded rationale opens beside the same turn".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.reasoning.expand.1".to_owned(),
        owner_id: "disclosure.reasoning".to_owned(),
        count: 1,
        outcome: "expanded".to_owned(),
    };
    let effect = simulated_effect(
        "effect.reasoning.expand",
        "disclosure",
        "expanded",
        "concise rationale became visible without private traces",
    );
    (before, after, action, transition, attempt, effect)
}

fn reasoning_collapse() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "reasoning",
        "Follow up after reading the rationale.",
        "turn-reasoning-rate-limit",
        "reasoning-detail",
        "reasoning-collapse",
        Some("disclosure.reasoning"),
        0,
    );
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "reasoning-collapsed".to_owned();
    after.selection = "reasoning-summary".to_owned();
    after.focus = "reasoning-expand".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.reasoning.collapse".to_owned(),
        label: "Collapse".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "collapse restores the same transcript position and draft".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.reasoning.collapse.1".to_owned(),
        owner_id: "disclosure.reasoning".to_owned(),
        count: 1,
        outcome: "collapsed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.reasoning.collapse",
        "disclosure",
        "collapsed",
        "rationale collapsed without mutating the answer or draft",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_success_expand() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "tool-success",
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "tool-result-summary",
        "tool-success-expand",
        Some("activity.tool.search"),
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code completed with 3 matches in 400 files."
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "tool-success-expanded".to_owned();
    after.selection = "tool-result-detail".to_owned();
    after.focus = "tool-success-collapse".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-success.expand".to_owned(),
        label: "Expand".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "focused matches open under the same completed search".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-success.expand.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "expanded".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-success.expand",
        "disclosure",
        "expanded",
        "three focused matches were disclosed under the search row",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_success_collapse() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "tool-success-expanded",
        "Also list the tests that should cover the search.",
        "tool-search-rate-limit",
        "tool-result-detail",
        "tool-success-collapse",
        Some("activity.tool.search"),
        0,
    );
    before.transcript =
        "Checked session rules completed. Search project code completed with 3 matches in 400 files."
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "tool-success".to_owned();
    after.selection = "tool-result-summary".to_owned();
    after.focus = "tool-success-expand".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-success.collapse".to_owned(),
        label: "Collapse".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "collapse restores the compact terminal result".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-success.collapse.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "collapsed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-success.collapse",
        "disclosure",
        "collapsed",
        "focused matches collapsed without mutating the terminal summary",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_task_expand() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "tool-task",
        "Follow up after the delegated audit finishes.",
        "task-audit-auth",
        "task-audit-summary",
        "tool-task-expand",
        Some("activity.task.audit-auth"),
        0,
    );
    before.user_task = "Audit the auth module and add the rate-limit tests.".to_owned();
    before.transcript =
        "Delegated audit complete with 6 findings. Rate-limit tests still running.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "tool-task-expanded".to_owned();
    after.selection = "task-audit-findings".to_owned();
    after.focus = "tool-task-collapse".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-task.expand".to_owned(),
        label: "Expand".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "findings open under the completed audit task".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-task.expand.1".to_owned(),
        owner_id: "activity.task.audit-auth".to_owned(),
        count: 1,
        outcome: "expanded".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-task.expand",
        "disclosure",
        "expanded",
        "audit findings opened while sibling task states remained visible",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_task_collapse() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "tool-task-expanded",
        "Follow up after the delegated audit finishes.",
        "task-audit-auth",
        "task-audit-findings",
        "tool-task-collapse",
        Some("activity.task.audit-auth"),
        0,
    );
    before.user_task = "Audit the auth module and add the rate-limit tests.".to_owned();
    before.transcript =
        "Delegated audit complete with 6 findings. Rate-limit tests still running.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "tool-task".to_owned();
    after.selection = "task-audit-summary".to_owned();
    after.focus = "tool-task-expand".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-task.collapse".to_owned(),
        label: "Collapse".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "collapse keeps the audit findings available under the same task".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-task.collapse.1".to_owned(),
        owner_id: "activity.task.audit-auth".to_owned(),
        count: 1,
        outcome: "collapsed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-task.collapse",
        "disclosure",
        "collapsed",
        "expanded findings collapsed without changing sibling state",
    );
    (before, after, action, transition, attempt, effect)
}

fn shell_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list the tests that should cover the search.",
        "shell-cargo-test-running",
        "shell-cargo-test-running",
        "shell-interrupt",
        Some("activity.shell.cargo-test"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.transcript =
        "git status completed with exit 0. cargo test is still running with captured output."
            .to_owned();
    before.partial_response =
        "Compiling cortex-server v0.1.0\nFinished test profile in 3.1s\ntest routes::login::rate_limited ... ok"
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-cargo-test-interrupted".to_owned();
    after.focus = "shell-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "git status completed with exit 0. cargo test was interrupted; captured output kept."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.interrupt".to_owned(),
        label: "Stop command".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "interrupt stops the live command, keeps captured output, and restores the composer"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.interrupt.1".to_owned(),
        owner_id: "activity.shell.cargo-test".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.interrupt",
        "shell-command",
        "interrupted",
        "running command stopped with captured output and exit status retained",
    );
    (before, after, action, transition, attempt, effect)
}

fn shell_complete() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list the tests that should cover the search.",
        "shell-cargo-test-running",
        "shell-cargo-test-running",
        "shell-complete",
        Some("activity.shell.cargo-test"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.transcript =
        "git status completed with exit 0. cargo test is still running with captured output."
            .to_owned();
    before.partial_response =
        "Compiling cortex-server v0.1.0\nFinished test profile in 3.1s\ntest routes::login::rate_limited ... ok"
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-cargo-test-complete".to_owned();
    after.focus = "shell-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "git status completed with exit 0. cargo test completed with exit 0; output preserved."
            .to_owned();
    after.partial_response = String::new();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.complete".to_owned(),
        label: "Command finished".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "completion replaces the running row with exit 0 and restores the composer"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.complete.1".to_owned(),
        owner_id: "activity.shell.cargo-test".to_owned(),
        count: 1,
        outcome: "completed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.complete",
        "shell-command",
        "completed",
        "running command finished with exit 0 and preserved captured output",
    );
    (before, after, action, transition, attempt, effect)
}

fn todo_progress() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "todo-list",
        "Type a follow-up, it runs after this turn…",
        "todo-current-login-tests",
        "todo-progress-baseline",
        "todo-advance",
        Some("activity.todo.plan"),
        0,
    );
    before.user_task = "Implement rate limiting on the login endpoint.".to_owned();
    before.transcript =
        "Plan: 3 of 5 done, 1 in progress (login tests), 1 pending (verify suite).".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "todo-progress-advanced".to_owned();
    after.focus = "todo-current-verify-suite".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Plan: 4 of 5 done, 1 in progress (verify suite), 0 pending. Origin request unchanged."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.todo.advance".to_owned(),
        label: "Mark current done".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "only affected task states and the progress summary change while the origin turn and composer stay put"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.todo.advance.1".to_owned(),
        owner_id: "activity.todo.plan".to_owned(),
        count: 1,
        outcome: "advanced".to_owned(),
    };
    let effect = simulated_effect(
        "effect.todo.advance",
        "todo-progress",
        "advanced",
        "current item completed; summary moved from 3 of 5 to 4 of 5 without leaving conversation",
    );
    (before, after, action, transition, attempt, effect)
}

fn chat_long_scroll_return_latest() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-long-scroll",
        "Add a short note about the remaining risk.",
        "earlier-turn-band",
        "earlier-visible-turns",
        "return-to-latest",
        Some("scroll.history.earlier"),
        0,
    );
    before.user_task = "Review earlier turns, then return to the latest work.".to_owned();
    before.transcript =
        "Reading earlier turns. 12 earlier turns exist above the viewport. Draft kept.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "latest-visible-turn".to_owned();
    after.focus = "latest-composer".to_owned();
    after.scroll_anchor = "latest-turn".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Returned to the latest turn and composer. Draft preserved at newest work.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.chat-long-scroll.return-latest".to_owned(),
        label: "Jump to latest".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "return to latest restores the prior draft and lands near the exact latest activity"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.chat-long-scroll.return-latest.1".to_owned(),
        owner_id: "scroll.history.position".to_owned(),
        count: 1,
        outcome: "at-latest".to_owned(),
    };
    let effect = simulated_effect(
        "effect.chat-long-scroll.return-latest",
        "history-navigation",
        "returned",
        "scroll returned to the latest turn with draft preserved",
    );
    (before, after, action, transition, attempt, effect)
}

fn chat_long_scroll_jump_earlier() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-long-scroll",
        "Add a short note about the remaining risk.",
        "latest-turn",
        "latest-visible-turn",
        "jump-earlier",
        Some("scroll.history.latest"),
        0,
    );
    before.user_task = "Review earlier turns, then return to the latest work.".to_owned();
    before.transcript =
        "Near the latest activity. 12 earlier turns remain available above.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "earlier-visible-turns".to_owned();
    after.focus = "return-to-latest".to_owned();
    after.scroll_anchor = "earlier-turn-band".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Jumped to earlier turns. Draft preserved. Jump to latest remains available.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.chat-long-scroll.jump-earlier".to_owned(),
        label: "Jump to earlier".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "jump earlier preserves the draft and lands at the earlier-turn-band anchor"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.chat-long-scroll.jump-earlier.1".to_owned(),
        owner_id: "scroll.history.position".to_owned(),
        count: 1,
        outcome: "at-earlier".to_owned(),
    };
    let effect = simulated_effect(
        "effect.chat-long-scroll.jump-earlier",
        "history-navigation",
        "jumped",
        "scroll moved to earlier turns with draft preserved",
    );
    (before, after, action, transition, attempt, effect)
}

const TWO_QUEUE_ITEMS: &str = "1:queue-item-1:rev=1:hash=qa11 Also list unit tests. [attachment: queue-notes-a.md] | 2:queue-item-2:rev=2:hash=qb22 Then summarize risks. [attachment: queue-notes-b.md]";

fn queued_interact_base(
    screen_id: &str,
    draft: &str,
    scroll_anchor: &str,
    selection: &str,
    focus: &str,
    pending: Option<&str>,
    tick: u32,
) -> ScenarioState {
    let mut s = disclosure_base(
        screen_id,
        draft,
        scroll_anchor,
        selection,
        focus,
        pending,
        tick,
    );
    s.queue = TWO_QUEUE_ITEMS.to_owned();
    s.queued_follow_up = TWO_QUEUE_ITEMS.to_owned();
    s.attachment = "queue-notes-a.md".to_owned();
    s.partial_response =
        "The shared request guard applies the limit before route work begins.".to_owned();
    s.state_hash = state_hash(&s);
    s
}

fn queued_interrupt_streaming() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = queued_interact_base(
        "streaming",
        "Hold these two follow-ups until streaming ends.",
        "partial-answer-anchor",
        "active-reply",
        "streaming-interrupt",
        Some("active.reply.auth-refactor"),
        0,
    );
    before.transcript = before.partial_response.clone();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.selection = "interrupted-partial-answer".to_owned();
    after.focus = "queued-composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript = format!(
        "{} [interrupted] Queue retained: {}",
        before.partial_response, TWO_QUEUE_ITEMS
    );
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
        path: "interrupt keeps both queued prompts and attachments in original order".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.streaming.queued-interrupt.1".to_owned(),
        owner_id: "active.reply.auth-refactor".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.streaming.queued-interrupt",
        "generation",
        "queue-preserved",
        "two queued prompts retained identity, revision, hash, and order",
    );
    (before, after, action, transition, attempt, effect)
}

fn queued_interrupt_tool() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = queued_interact_base(
        "tool-running",
        "Keep the two queued prompts while the search runs.",
        "tool-search-row",
        "tool-search-running",
        "tool-interrupt",
        Some("activity.tool.search"),
        0,
    );
    before.transcript = "Search project code is running. Queue is waiting.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.selection = "tool-search-interrupted".to_owned();
    after.focus = "queued-composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript = format!(
        "Search stopped. Queue retained in order: {}",
        TWO_QUEUE_ITEMS
    );
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
        path: "tool interrupt preserves both queued prompts without reordering".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool.queued-interrupt.1".to_owned(),
        owner_id: "activity.tool.search".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool.queued-interrupt",
        "tool-execution",
        "queue-preserved",
        "tool stop retained queue item revisions and attachment hashes",
    );
    (before, after, action, transition, attempt, effect)
}

fn queued_interrupt_shell() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = queued_interact_base(
        "shell",
        "Keep the two queued prompts while the command runs.",
        "shell-cargo-test-running",
        "shell-cargo-test-running",
        "shell-interrupt",
        Some("activity.shell.cargo-test"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript =
        "Foreground cargo test is running. Queue waits with both attachments.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-cargo-test-interrupted".to_owned();
    after.focus = "queued-composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript = format!(
        "Command interrupted. Captured output kept. Queue retained: {}",
        TWO_QUEUE_ITEMS
    );
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.interrupt".to_owned(),
        label: "Stop command".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "shell interrupt preserves queued prompts, attachments, and captured output"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.queued-interrupt.1".to_owned(),
        owner_id: "activity.shell.cargo-test".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.queued-interrupt",
        "shell-command",
        "queue-preserved",
        "foreground interrupt retained the ordered two-item queue",
    );
    (before, after, action, transition, attempt, effect)
}

fn agent_question_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "streaming",
        "Keep the risk note short before we publish.",
        "partial-answer-anchor",
        "active-reply",
        "streaming-live",
        Some("active.reply.auth-refactor"),
        0,
    );
    before.partial_response = "I need one choice before continuing the recovery note.".to_owned();
    before.transcript = before.partial_response.clone();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "agent-question-option-a".to_owned();
    after.focus = "agent-question-submit".to_owned();
    after.pending_action = Some("activity.agent-question".to_owned());
    after.virtual_tick = 1;
    after.transcript = "Paused for one decision: which recovery path should we keep?"
        .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.agent-question.open".to_owned(),
        label: "Ask inline".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "question opens inline with the paused turn and an anchored decision dock"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.agent-question.open.1".to_owned(),
        owner_id: "activity.agent-question".to_owned(),
        count: 1,
        outcome: "awaiting-answer".to_owned(),
    };
    let effect = simulated_effect(
        "effect.agent-question.open",
        "decision",
        "dock-open",
        "one focused response and Submit answer wait on the bottom dock",
    );
    (before, after, action, transition, attempt, effect)
}

fn agent_question_submit() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "streaming",
        "Keep the risk note short before we publish.",
        "partial-answer-anchor",
        "agent-question-option-a",
        "agent-question-submit",
        Some("activity.agent-question"),
        0,
    );
    before.partial_response = "Paused for one decision: which recovery path should we keep?"
        .to_owned();
    before.transcript = before.partial_response.clone();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "chat-turn".to_owned();
    after.selection = "composer".to_owned();
    after.focus = "composer".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Answer submitted once. Activity resumed and the prior composer draft returned."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.agent-question.submit".to_owned(),
        label: "Submit answer".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "submit resumes that exact activity once and restores the prior composer draft"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.agent-question.submit.1".to_owned(),
        owner_id: "activity.agent-question".to_owned(),
        count: 1,
        outcome: "resumed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.agent-question.submit",
        "decision",
        "resumed-once",
        "answer submitted once without re-invoking the paused activity",
    );
    (before, after, action, transition, attempt, effect)
}

fn agent_question_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "streaming",
        "Keep the risk note short before we publish.",
        "partial-answer-anchor",
        "agent-question-option-a",
        "agent-question-submit",
        Some("activity.agent-question"),
        0,
    );
    before.partial_response = "Paused for one decision: which recovery path should we keep?"
        .to_owned();
    before.transcript = before.partial_response.clone();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "agent-question-option-a".to_owned();
    after.focus = "agent-question-submit".to_owned();
    after.pending_action = Some("activity.agent-question".to_owned());
    after.virtual_tick = 1;
    after.transcript =
        "Cancel kept the activity paused. Submit answer remains on the dock.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.agent-question.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "cancel keeps the activity paused with the prior draft intact".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.agent-question.cancel.1".to_owned(),
        owner_id: "activity.agent-question".to_owned(),
        count: 1,
        outcome: "still-paused".to_owned(),
    };
    let effect = simulated_effect(
        "effect.agent-question.cancel",
        "decision",
        "still-paused",
        "cancel did not resume or drop the pending answer",
    );
    (before, after, action, transition, attempt, effect)
}

fn reading_anchor_output_arrive() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "streaming",
        "Note the remaining risk after this section.",
        "earlier-reading-anchor",
        "earlier-rate-limit-guard",
        "reading-earlier",
        Some("active.reply.auth-refactor"),
        0,
    );
    before.partial_response = "Earlier section about the rate-limit guard stays in view.".to_owned();
    before.transcript = before.partial_response.clone();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "new-output-affordance".to_owned();
    after.focus = "new-output".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "New lines arrived below. Reading anchor unchanged. New output affordance shown."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.streaming.output-arrive".to_owned(),
        label: "Output arrived".to_owned(),
        enabled: true,
        automatic: true,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "automatic".to_owned(),
        path: "live output never forces scroll while the user reads earlier content".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.streaming.output-arrive.1".to_owned(),
        owner_id: "active.reply.auth-refactor".to_owned(),
        count: 1,
        outcome: "anchor-preserved".to_owned(),
    };
    let effect = simulated_effect(
        "effect.streaming.output-arrive",
        "live-output",
        "anchor-preserved",
        "new output stayed below until Jump to latest",
    );
    (before, after, action, transition, attempt, effect)
}

fn reading_anchor_jump_latest() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "streaming",
        "Note the remaining risk after this section.",
        "earlier-reading-anchor",
        "new-output-affordance",
        "new-output",
        Some("active.reply.auth-refactor"),
        0,
    );
    before.transcript =
        "Reading earlier rate-limit guard. New output is waiting below.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "latest-output".to_owned();
    after.focus = "latest-composer".to_owned();
    after.scroll_anchor = "latest-output".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Jumped to the newest output below. Draft unchanged. Activity still running.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.streaming.new-output".to_owned(),
        label: "New output".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "New output jumps to the newest lines without changing draft or activity state"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.streaming.new-output.1".to_owned(),
        owner_id: "scroll.reading-anchor".to_owned(),
        count: 1,
        outcome: "at-latest".to_owned(),
    };
    let effect = simulated_effect(
        "effect.streaming.new-output",
        "live-output",
        "jumped-latest",
        "reading jumped to newest output while draft and activity stayed put",
    );
    (before, after, action, transition, attempt, effect)
}

fn transcript_search_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-turn",
        "Add a short follow-up about the guard.",
        "latest-turn",
        "completed-answer",
        "composer",
        None,
        0,
    );
    before.transcript =
        "You asked for the rate-limit path. The shared request guard applies the limit."
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "match-1".to_owned();
    after.focus = "search-next".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Search open: 3 matches for \"guard\". Draft and reading position preserved.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.transcript.search-open".to_owned(),
        label: "Search transcript".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "search opens with deterministic matches while draft and position stay put"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.transcript.search-open.1".to_owned(),
        owner_id: "transcript.search".to_owned(),
        count: 1,
        outcome: "3-matches".to_owned(),
    };
    let effect = simulated_effect(
        "effect.transcript.search-open",
        "search",
        "matches",
        "transcript search reported three deterministic matches",
    );
    (before, after, action, transition, attempt, effect)
}

fn transcript_search_next() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-turn",
        "Add a short follow-up about the guard.",
        "match-1",
        "match-1",
        "search-next",
        Some("transcript.search"),
        0,
    );
    before.transcript = "Search: 3 matches for \"guard\". Currently match 1.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "match-2".to_owned();
    after.scroll_anchor = "match-2".to_owned();
    after.virtual_tick = 1;
    after.transcript = "Search: advanced to match 2 of 3. Draft unchanged.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.transcript.search-next".to_owned(),
        label: "Next match".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "next match moves selection without mutating draft or adding turns".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.transcript.search-next.1".to_owned(),
        owner_id: "transcript.search".to_owned(),
        count: 1,
        outcome: "match-2".to_owned(),
    };
    let effect = simulated_effect(
        "effect.transcript.search-next",
        "search",
        "advanced",
        "selection advanced to match-2",
    );
    (before, after, action, transition, attempt, effect)
}

fn transcript_search_no_results() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-turn",
        "Add a short follow-up about the guard.",
        "latest-turn",
        "completed-answer",
        "composer",
        None,
        0,
    );
    before.transcript = "The shared request guard applies the limit.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "search-no-results".to_owned();
    after.focus = "search-close".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Search open: No matches for \"xyzzy-not-found\". Draft and position preserved."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.transcript.search-no-results".to_owned(),
        label: "Search transcript".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "no matches stays recoverable without leaving the conversation".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.transcript.search-no-results.1".to_owned(),
        owner_id: "transcript.search".to_owned(),
        count: 1,
        outcome: "no-matches".to_owned(),
    };
    let effect = simulated_effect(
        "effect.transcript.search-no-results",
        "search",
        "no-matches",
        "recoverable empty result with draft preserved",
    );
    (before, after, action, transition, attempt, effect)
}

fn transcript_search_close() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-turn",
        "Add a short follow-up about the guard.",
        "match-2",
        "match-2",
        "search-close",
        Some("transcript.search"),
        0,
    );
    before.transcript = "Search open at match 2 of 3.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "completed-answer".to_owned();
    after.focus = "composer".to_owned();
    after.scroll_anchor = "match-2".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Search closed. Returned to the originating position with draft intact.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.transcript.search-close".to_owned(),
        label: "Close search".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "closing search restores the originating reading position".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.transcript.search-close.1".to_owned(),
        owner_id: "transcript.search".to_owned(),
        count: 1,
        outcome: "closed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.transcript.search-close",
        "search",
        "closed",
        "search closed with origin reading position restored",
    );
    (before, after, action, transition, attempt, effect)
}

fn transcript_copy_message() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-turn",
        "Add a short follow-up about the guard.",
        "latest-turn",
        "message-answer-public",
        "composer",
        None,
        0,
    );
    before.transcript =
        "The shared request guard applies the limit before route work begins.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "message-answer-public".to_owned();
    after.focus = "copy-confirmed".to_owned();
    after.virtual_tick = 1;
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.transcript.copy-message".to_owned(),
        label: "Copy message".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "copy acts on the selected public message only and never adds a turn".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.transcript.copy.1".to_owned(),
        owner_id: "message.answer-public".to_owned(),
        count: 1,
        outcome: "copied".to_owned(),
    };
    let effect = simulated_effect(
        "effect.transcript.copy",
        "clipboard",
        "copied",
        "copied only the public answer text; no hidden rationale or secrets",
    );
    (before, after, action, transition, attempt, effect)
}

fn shell_foreground_interrupt() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list remaining risks after commands finish.",
        "shell-mixed-foreground",
        "shell-foreground-running",
        "shell-foreground-interrupt",
        Some("activity.shell.foreground-cargo"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript =
        "Foreground cargo test running. Background watch continues.".to_owned();
    before.partial_response =
        "Compiling cortex-server v0.1.0\nFinished test profile in 3.1s".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-foreground-interrupted".to_owned();
    after.focus = "shell-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Foreground interrupted. Background still running. Captured output kept.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.foreground-interrupt".to_owned(),
        label: "Stop command".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "interrupt stops only the selected foreground process".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.foreground-interrupt.1".to_owned(),
        owner_id: "activity.shell.foreground-cargo".to_owned(),
        count: 1,
        outcome: "interrupted".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.foreground-interrupt",
        "shell-command",
        "foreground-stopped",
        "foreground stopped once; background and captured output preserved",
    );
    (before, after, action, transition, attempt, effect)
}

fn shell_background_stop() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list remaining risks after commands finish.",
        "shell-mixed-background",
        "shell-background-running",
        "shell-background-stop",
        Some("activity.shell.background-watch"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript =
        "Background watch is running while the completed foreground command stays visible."
            .to_owned();
    before.partial_response = "watch: file .../session.rs changed".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-background-stopped".to_owned();
    after.focus = "shell-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Background stopped by scoped action. Foreground output and captures preserved."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.background-stop".to_owned(),
        label: "Stop background".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "Stop background command terminates only that process and keeps captures".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.background-stop.1".to_owned(),
        owner_id: "activity.shell.background-watch".to_owned(),
        count: 1,
        outcome: "stopped".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.background-stop",
        "shell-command",
        "background-stopped",
        "background stopped once without touching unrelated activities",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_task_control_base(selection: &str, focus: &str) -> ScenarioState {
    let mut s = disclosure_base(
        "tool-task",
        "Follow up after the delegated audit finishes.",
        "task-audit-auth",
        selection,
        focus,
        Some("activity.task.selected"),
        0,
    );
    s.user_task = "Audit the auth module and add the rate-limit tests.".to_owned();
    s.turn_id = "turn.daily-work.task-audit.22".to_owned();
    s.transcript =
        "Delegated tasks: audit complete, tests running, build queued, refresh blocked."
            .to_owned();
    s.state_hash = state_hash(&s);
    s
}

fn tool_task_stop_selected() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = tool_task_control_base("task-add-tests", "task-stop");
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "task-add-tests-stopped".to_owned();
    after.focus = "task-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Selected test task stopped. Sibling audit, build, and refresh states unchanged."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-task.stop-selected".to_owned(),
        label: "Stop this task · ends only the selected test work".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "stop applies only to the selected task and states the consequence first".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-task.stop.1".to_owned(),
        owner_id: "activity.task.add-tests".to_owned(),
        count: 1,
        outcome: "stopped".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-task.stop",
        "delegated-task",
        "selected-stopped",
        "only the selected running task stopped; siblings kept identity and order",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_task_retry_selected() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = tool_task_control_base("task-refresh-risk", "task-retry");
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "task-refresh-risk-retrying".to_owned();
    after.focus = "task-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Selected blocked refresh task retrying once. Siblings unchanged.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-task.retry-selected".to_owned(),
        label: "Retry this task · restarts only the blocked refresh review".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "retry applies only to the selected blocked task".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-task.retry.1".to_owned(),
        owner_id: "activity.task.refresh-risk".to_owned(),
        count: 1,
        outcome: "retrying".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-task.retry",
        "delegated-task",
        "selected-retrying",
        "only the selected blocked task entered retry; siblings preserved",
    );
    (before, after, action, transition, attempt, effect)
}

fn tool_task_cancel_selected() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = tool_task_control_base("task-build-suite", "task-cancel");
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "task-build-suite-cancelled".to_owned();
    after.focus = "task-continue".to_owned();
    after.pending_action = None;
    after.virtual_tick = 1;
    after.transcript =
        "Selected queued build task cancelled. Siblings and findings unchanged.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.tool-task.cancel-selected".to_owned(),
        label: "Cancel this task · drops only the queued build".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "cancel removes only the selected queued task".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.tool-task.cancel.1".to_owned(),
        owner_id: "activity.task.build-suite".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.tool-task.cancel",
        "delegated-task",
        "selected-cancelled",
        "only the selected queued task cancelled; sibling order retained",
    );
    (before, after, action, transition, attempt, effect)
}

fn large_output_open() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list tests after this suite output.",
        "shell-large-compact",
        "shell-large-summary",
        "large-output-open",
        Some("activity.shell.large-cargo"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript =
        "Large cargo output compact: showing 12 of 240 lines · 228 omitted.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-large-full".to_owned();
    after.focus = "large-output-search".to_owned();
    after.scroll_anchor = "shell-large-full".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Full detail open for 240 lines. Search and copy available. Return restores compact."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.large-output-open".to_owned(),
        label: "Full detail".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "full detail opens searchable complete output without clipping silently".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.large-output-open.1".to_owned(),
        owner_id: "activity.shell.large-cargo".to_owned(),
        count: 1,
        outcome: "full-open".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.large-output-open",
        "large-output",
        "opened",
        "bounded summary opened lossless full detail with search and copy",
    );
    (before, after, action, transition, attempt, effect)
}

fn large_output_search() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list tests after this suite output.",
        "shell-large-full",
        "shell-large-full",
        "large-output-search",
        Some("activity.shell.large-cargo"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript = "Full cargo output open. Search idle.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "output-match-1".to_owned();
    after.focus = "large-output-copy".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Search output: match 1 for rate_limited. Complete output hash unchanged.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.large-output-search".to_owned(),
        label: "Search output".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "search inside complete output preserves the lossless payload".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.large-output-search.1".to_owned(),
        owner_id: "activity.shell.large-cargo".to_owned(),
        count: 1,
        outcome: "match".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.large-output-search",
        "large-output",
        "matched",
        "searching complete output selected output-match-1 without clipping",
    );
    (before, after, action, transition, attempt, effect)
}

fn large_output_close() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "shell",
        "Also list tests after this suite output.",
        "shell-large-full",
        "output-match-1",
        "large-output-return",
        Some("activity.shell.large-cargo"),
        0,
    );
    before.user_task = "Check the branch, then run the focused tests.".to_owned();
    before.turn_id = "turn.daily-work.shell.21".to_owned();
    before.transcript = "Full cargo output open at a search match.".to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.selection = "shell-large-summary".to_owned();
    after.focus = "large-output-open".to_owned();
    after.scroll_anchor = "shell-large-compact".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Returned to compact: showing 12 of 240 lines · 228 omitted.".to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.shell.large-output-close".to_owned(),
        label: "Return".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "return restores the compact activity with bounded omitted count".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.shell.large-output-close.1".to_owned(),
        owner_id: "activity.shell.large-cargo".to_owned(),
        count: 1,
        outcome: "compact".to_owned(),
    };
    let effect = simulated_effect(
        "effect.shell.large-output-close",
        "large-output",
        "returned",
        "exact return to compact bounded summary with complete output still available",
    );
    (before, after, action, transition, attempt, effect)
}

fn compact_expand_history() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "compact",
        "Resume from the compacted summary",
        "compact-summary",
        "compact-summary",
        "compact-expand",
        Some("disclosure.compact-history"),
        0,
    );
    before.transcript =
        "Conversation compacted. Found the shared rate-limit guard. Full history preserved."
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "chat-long-scroll".to_owned();
    after.selection = "restored-history".to_owned();
    after.focus = "compact-collapse".to_owned();
    after.pending_action = None;
    after.scroll_anchor = "turn-1".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Turn 1 through turn 18 restored in original order. Draft and reading position kept."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.compact.expand-history".to_owned(),
        label: "Expand full history".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "full history restores earlier turns in order with the same draft".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.compact.expand-history.1".to_owned(),
        owner_id: "history.session.order".to_owned(),
        count: 1,
        outcome: "restored".to_owned(),
    };
    let effect = simulated_effect(
        "effect.compact.expand-history",
        "history-restore",
        "restored",
        "18 earlier turns restored in order while the draft stayed stable",
    );
    (before, after, action, transition, attempt, effect)
}

fn compact_collapse_history() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = disclosure_base(
        "chat-long-scroll",
        "Resume from the compacted summary",
        "turn-1",
        "restored-history",
        "compact-collapse",
        Some("disclosure.compact-history"),
        0,
    );
    before.transcript =
        "Turn 1 through turn 18 restored in original order. Draft and reading position kept."
            .to_owned();
    before.state_hash = state_hash(&before);
    let mut after = before.clone();
    after.screen_id = "compact".to_owned();
    after.selection = "compact-summary".to_owned();
    after.focus = "compact-expand".to_owned();
    after.pending_action = None;
    after.scroll_anchor = "compact-summary".to_owned();
    after.virtual_tick = 1;
    after.transcript =
        "Conversation compacted. Found the shared rate-limit guard. Full history preserved."
            .to_owned();
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.compact.collapse-history".to_owned(),
        label: "Return to compact summary".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "return restores the same compact summary and draft".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.compact.collapse-history.1".to_owned(),
        owner_id: "history.session.order".to_owned(),
        count: 1,
        outcome: "collapsed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.compact.collapse-history",
        "history-restore",
        "collapsed",
        "compact summary returned without reordering turns or losing the draft",
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
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
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
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
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
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
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

fn slash_confirm_export() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let confirmed_draft = SLASH_OPEN_DRAFT.replacen(SLASH_ACTIVE_SPAN, SLASH_CONFIRMED_EXPORT, 1);
    let mut before = state(
        "slash-menu",
        vec!["chat-turn", "composer-multiline", "slash-menu"],
        SLASH_OPEN_DRAFT,
        "turn-retry-review",
        "active-command-span-0..1",
        "draft-cursor-1",
        Some("slash.query.empty"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "slash-menu"],
        &confirmed_draft,
        "turn-retry-review",
        "no-selection",
        "draft-cursor-7",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.slash-menu.confirm-selected".to_owned(),
        label: "Confirm command".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "confirm replaced only the active / span with /export; surrounding authored lines remain byte-for-byte stable"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.slash-menu.confirm.1".to_owned(),
        owner_id: "slash.span.0..1".to_owned(),
        count: 1,
        outcome: "confirmed".to_owned(),
    };
    let effect = simulated_effect(
        "effect.slash-menu.confirm",
        "draft-edit",
        "simulated-span-replacement",
        "only the active command span became /export",
    );
    (before, after, action, transition, attempt, effect)
}

fn slash_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "slash-menu",
        vec!["chat-turn", "composer-multiline", "slash-menu"],
        SLASH_OPEN_DRAFT,
        "turn-retry-review",
        "active-command-span-0..1",
        "draft-cursor-1",
        Some("slash.query.empty"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    // Cancel restores the pre-open draft snapshot (draft before `/` was typed).
    before.draft = SLASH_PRE_OPEN_DRAFT.to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "slash-menu"],
        SLASH_PRE_OPEN_DRAFT,
        "turn-retry-review",
        "active-command-span-0..1",
        "draft-cursor-0",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.focus = before.focus.clone();
    after.selection = before.selection.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.slash-menu.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel restores the exact draft, cursor, height, selection, and scroll from before / opened"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.slash-menu.cancel.1".to_owned(),
        owner_id: "slash.span.0..1".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.slash-menu.cancel",
        "draft-edit",
        "simulated-restore",
        "cancel restored the pre-open draft snapshot byte for byte",
    );
    (before, after, action, transition, attempt, effect)
}

fn slash_invalid_trigger() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let embedded = "See https://example.com/path for details\n\
Then explain the failure path that still needs coverage before the merge.\n\
Keep the summary short before we merge.";
    let mut before = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline"],
        embedded,
        "turn-retry-review",
        "no-menu",
        "draft-cursor-24",
        None,
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = before.clone();
    after.virtual_tick = 1;
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.composer-multiline.slash-invalid-trigger".to_owned(),
        label: "Type embedded slash".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "a slash embedded in a URL or word does not open the command menu".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.slash-menu.invalid-trigger.1".to_owned(),
        owner_id: "composer.embedded-slash".to_owned(),
        count: 1,
        outcome: "no-menu".to_owned(),
    };
    let effect = simulated_effect(
        "effect.slash-menu.invalid-trigger",
        "filter",
        "simulated-no-menu",
        "command-valid positions only; embedded / kept the composer unchanged",
    );
    (before, after, action, transition, attempt, effect)
}

fn attachment_confirm() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    // Insert the attachment token immediately after the first authored line so the
    // surrounding draft text remains byte-stable when the token is stripped.
    // Use a leading space that is part of the insertion unit so stripping the
    // full unit restores the origin draft byte for byte.
    let insertion = format!(" {ATTACHMENT_TOKEN}");
    let mut confirmed_draft = ATTACHMENT_OPEN_DRAFT.to_owned();
    if let Some(end_of_first_line) = confirmed_draft.find('\n') {
        confirmed_draft.insert_str(end_of_first_line, &insertion);
    } else {
        confirmed_draft.push_str(&insertion);
    }

    let mut before = state(
        "attachment-picker",
        vec!["chat-turn", "composer-multiline", "attachment-picker"],
        ATTACHMENT_OPEN_DRAFT,
        "turn-retry-review",
        "eligible-assets-retry-diagram-png",
        "draft-cursor-30",
        Some("attachment.query.empty"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.attachment = String::new();
    apply_flow2_identity(&mut before);
    before.attachment = String::new();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "attachment-picker"],
        &confirmed_draft,
        "turn-retry-review",
        "no-selection",
        "draft-cursor-67",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.attachment = ATTACHMENT_SELECTED_PATH.to_owned();
    apply_flow2_identity(&mut after);
    after.draft = confirmed_draft;
    after.attachment = ATTACHMENT_SELECTED_PATH.to_owned();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.attachment-picker.confirm-selected".to_owned(),
        label: "Attach".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "confirm inserts exactly one [attachment: assets/retry-diagram.png] token at the cursor while surrounding authored lines stay byte-stable"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.attachment-picker.confirm.1".to_owned(),
        owner_id: "attachment.assets/retry-diagram.png".to_owned(),
        count: 1,
        outcome: "attached".to_owned(),
    };
    let effect = simulated_effect(
        "effect.attachment-picker.confirm",
        "draft-edit",
        "simulated-attachment-insert",
        "one eligible project-relative attachment became a single draft token/chip",
    );
    (before, after, action, transition, attempt, effect)
}

fn attachment_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "attachment-picker",
        vec!["chat-turn", "composer-multiline", "attachment-picker"],
        ATTACHMENT_OPEN_DRAFT,
        "turn-retry-review",
        "eligible-assets-retry-diagram-png",
        "draft-cursor-30",
        Some("attachment.query.empty"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.attachment = String::new();
    apply_flow2_identity(&mut before);
    before.attachment = String::new();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "attachment-picker"],
        ATTACHMENT_OPEN_DRAFT,
        "turn-retry-review",
        "eligible-assets-retry-diagram-png",
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.attachment = String::new();
    apply_flow2_identity(&mut after);
    after.draft = ATTACHMENT_OPEN_DRAFT.to_owned();
    after.attachment = String::new();
    after.selection = before.selection.clone();
    after.focus = before.focus.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.attachment-picker.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel adds no attachment token and restores the original draft byte for byte"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.attachment-picker.cancel.1".to_owned(),
        owner_id: "attachment.assets/retry-diagram.png".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.attachment-picker.cancel",
        "draft-edit",
        "simulated-no-change",
        "cancel left the exact multiline draft and attachment set untouched",
    );
    (before, after, action, transition, attempt, effect)
}

fn export_confirm() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let open_transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    let success_transcript = format!("{open_transcript} {EXPORT_SUCCESS_SUMMARY}");

    let mut before = state(
        "export-picker",
        vec!["chat-turn", "composer-multiline", "export-picker"],
        EXPORT_OPEN_DRAFT,
        "turn-retry-review",
        EXPORT_DEFAULT_FORMAT,
        "draft-cursor-30",
        Some("export.format.markdown"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript = open_transcript.clone();
    before.attachment = String::new();
    apply_flow2_identity(&mut before);
    before.attachment = String::new();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "export-picker"],
        EXPORT_OPEN_DRAFT,
        "turn-retry-review",
        "no-selection",
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = success_transcript;
    after.attachment = String::new();
    apply_flow2_identity(&mut after);
    after.draft = EXPORT_OPEN_DRAFT.to_owned();
    after.attachment = String::new();
    after.focus = before.focus.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.export-picker.confirm-selected".to_owned(),
        label: "Export".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: format!(
            "confirm creates exactly the selected sandboxed Markdown file at {EXPORT_ARTIFACT_MARKDOWN} and restores the origin draft and geometry with one inline success summary"
        ),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.export-picker.confirm.1".to_owned(),
        owner_id: format!("export.{EXPORT_DEFAULT_FORMAT}"),
        count: 1,
        outcome: "exported".to_owned(),
    };
    let effect = simulated_effect(
        "effect.export-picker.confirm",
        "export-artifact",
        "simulated-export-create",
        &format!(
            "one sandboxed {EXPORT_DEFAULT_FORMAT} artifact was created at {EXPORT_ARTIFACT_MARKDOWN}"
        ),
    );
    (before, after, action, transition, attempt, effect)
}

fn export_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "export-picker",
        vec!["chat-turn", "composer-multiline", "export-picker"],
        EXPORT_OPEN_DRAFT,
        "turn-retry-review",
        EXPORT_DEFAULT_FORMAT,
        "draft-cursor-30",
        Some("export.format.markdown"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    before.attachment = String::new();
    apply_flow2_identity(&mut before);
    before.attachment = String::new();
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec!["chat-turn", "composer-multiline", "export-picker"],
        EXPORT_OPEN_DRAFT,
        "turn-retry-review",
        EXPORT_DEFAULT_FORMAT,
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    after.attachment = String::new();
    apply_flow2_identity(&mut after);
    after.draft = EXPORT_OPEN_DRAFT.to_owned();
    after.attachment = String::new();
    after.selection = before.selection.clone();
    after.focus = before.focus.clone();
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.export-picker.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel creates no export artifact, shows no success claim, and restores the original draft and geometry"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.export-picker.cancel.1".to_owned(),
        owner_id: format!("export.{EXPORT_DEFAULT_FORMAT}"),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.export-picker.cancel",
        "export-artifact",
        "simulated-no-change",
        "cancel left the conversation, draft, and sandbox export set untouched",
    );
    (before, after, action, transition, attempt, effect)
}

fn palette_open_submenu() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "command-palette",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "root-action-queued-prompts",
        "root-query-",
        Some("palette.root"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "command-palette-submenu",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "submenu-action-add-buffer",
        "submenu-scope-queued-prompts",
        Some("palette.submenu.queued-prompts"),
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.command-palette.open-submenu".to_owned(),
        label: "Open scope".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "opening Queued prompts enters the named submenu without mutating the draft"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.command-palette.open-submenu.1".to_owned(),
        owner_id: "palette.root.queued-prompts".to_owned(),
        count: 1,
        outcome: "opened".to_owned(),
    };
    let effect = simulated_effect(
        "effect.command-palette.open-submenu",
        "navigation",
        "simulated-scope-enter",
        "submenu opened with explicit queued-prompts scope",
    );
    (before, after, action, transition, attempt, effect)
}

fn palette_submenu_back() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "command-palette-submenu",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "submenu-action-add-buffer",
        "submenu-scope-queued-prompts",
        Some("palette.submenu.queued-prompts"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "command-palette",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "root-action-queued-prompts",
        "root-query-",
        Some("palette.root"),
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.command-palette-submenu.back".to_owned(),
        label: "Back to root".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "back restores the root palette with the prior root query, selected row, and list position"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.command-palette-submenu.back.1".to_owned(),
        owner_id: "palette.submenu.queued-prompts".to_owned(),
        count: 1,
        outcome: "back".to_owned(),
    };
    let effect = simulated_effect(
        "effect.command-palette-submenu.back",
        "navigation",
        "simulated-scope-exit",
        "back restored root query and selected queued-prompts row",
    );
    (before, after, action, transition, attempt, effect)
}

fn palette_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "command-palette",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "root-action-recall",
        "draft-cursor-30",
        Some("palette.root"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "root-action-recall",
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.command-palette.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel from the root palette restores the original multiline draft and transcript position"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.command-palette.cancel.1".to_owned(),
        owner_id: "palette.root".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.command-palette.cancel",
        "draft-edit",
        "simulated-no-change",
        "root cancel restored the origin composer without mutating the draft",
    );
    (before, after, action, transition, attempt, effect)
}

fn palette_submenu_cancel() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "command-palette-submenu",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "submenu-action-add-buffer",
        "draft-cursor-30",
        Some("palette.submenu.queued-prompts"),
        "return-to-multiline-draft",
        0,
    );
    before.transcript =
        "Check the retry path before we merge. Code is paused beside the draft.".to_owned();
    apply_flow2_identity(&mut before);
    before.state_hash = state_hash(&before);

    let mut after = state(
        "composer-multiline",
        vec![
            "chat-turn",
            "composer-multiline",
            "command-palette",
        ],
        PALETTE_PRESERVED_DRAFT,
        "turn-retry-review",
        "submenu-action-add-buffer",
        "draft-cursor-30",
        None,
        "return-to-multiline-draft",
        1,
    );
    after.transcript = before.transcript.clone();
    apply_flow2_identity(&mut after);
    after.state_hash = state_hash(&after);

    let action = ActionEvidence {
        action_id: "action.command-palette-submenu.cancel".to_owned(),
        label: "Cancel".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "cancel".to_owned(),
        path: "cancel from the submenu returns directly to the origin composer with the exact original multiline draft"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.command-palette-submenu.cancel.1".to_owned(),
        owner_id: "palette.submenu.queued-prompts".to_owned(),
        count: 1,
        outcome: "cancelled".to_owned(),
    };
    let effect = simulated_effect(
        "effect.command-palette-submenu.cancel",
        "draft-edit",
        "simulated-no-change",
        "submenu cancel restored the origin composer without mutating the draft",
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

fn apply_flow3_current_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.configuration".to_owned();
    state.transcript_id = "transcript.configuration".to_owned();
    state.session_id = "session.configuration.1".to_owned();
    state.turn_id = "turn.configuration.1".to_owned();
    state.user_task = "Choose a model and how Code should work.".to_owned();
    state.visible_origin = "Work configuration · current conversation".to_owned();
    state.state_hash = state_hash(state);
}

fn apply_flow3_resume_identity(state: &mut ScenarioState) {
    state.conversation_id = "conversation.configuration".to_owned();
    state.transcript_id = "transcript.configuration".to_owned();
    state.session_id = "session.configuration.1".to_owned();
    state.turn_id = "turn.configuration.1".to_owned();
    state.user_task = "Resume a conversation or inspect its history.".to_owned();
    state.visible_origin = "Work configuration · current conversation".to_owned();
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
        || diff.hunks[0].hunk_id != "hunk.retry-guard"
        || diff.hunks[1].hunk_id != "hunk.retry-test"
        || diff.focused_hunk.trim().is_empty()
        || diff.git_index_before != diff.git_index_after
        || diff.hunks.iter().any(|hunk| {
            hunk.file_path != diff.file_path
                || hunk.hunk_id.trim().is_empty()
                || !matches!(hunk.state.as_str(), "Included" | "Excluded" | "Pending")
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
    let before_bytes = diff_fixture_file_bytes();
    validate_diff_hunk_contract(&before_bytes, &diff.hunks)?;
    let index_bytes = diff_index_fixture_bytes();
    if diff.working_tree_before != sha256_hex(&before_bytes)
        || diff.git_index_before != sha256_hex(index_bytes)
        || diff.git_index_after != sha256_hex(index_bytes)
    {
        return Err(
            "diff evidence must hash the exact working-tree and Git-index fixture bytes".to_owned(),
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
    let expected_excluded = diff
        .hunks
        .iter()
        .filter(|hunk| hunk.state != "Included")
        .map(|hunk| hunk.hunk_id.clone())
        .collect::<Vec<_>>();
    if diff.excluded_hunks != expected_excluded {
        return Err("diff evidence does not reconcile pending/excluded hunks".to_owned());
    }
    if diff.write_count > 1 {
        return Err("diff evidence exceeds one atomic write".to_owned());
    }
    let applied_bytes = apply_validated_diff_hunks(&before_bytes, &diff.hunks, &diff.applied_hunks)
        .map_err(|error| format!("diff evidence cannot apply disclosed hunks: {error}"))?;
    let expected_after = if diff.write_count == 1 {
        sha256_hex(&applied_bytes)
    } else {
        sha256_hex(&before_bytes)
    };
    if diff.working_tree_after != expected_after {
        return Err(
            "diff evidence after hash does not match the disclosed applied hunks".to_owned(),
        );
    }
    if diff.write_count == 0 && !diff.applied_hunks.is_empty() {
        return Err("diff evidence reports applied hunks without a write".to_owned());
    }
    if diff.write_count == 1 && diff.applied_hunks.is_empty() {
        return Err("diff evidence reports a write without applied hunks".to_owned());
    }
    if diff.write_count == 1 {
        let expected_applied = diff
            .hunks
            .iter()
            .filter(|hunk| hunk.state == "Included")
            .map(|hunk| hunk.hunk_id.clone())
            .collect::<Vec<_>>();
        if diff.applied_hunks != expected_applied {
            return Err("diff evidence does not reconcile applied hunks".to_owned());
        }
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

pub(crate) fn diff_hunks(states: [String; 2]) -> Vec<DiffHunkEvidence> {
    derive_diff_hunks_from_bytes(&diff_fixture_file_bytes(), states)
        .expect("canonical diff payloads must derive exact ordered coordinates")
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
            | "staging-partial-failure"
            | "apply-partial-failure"
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
    let before_bytes = diff_fixture_file_bytes();
    let working_tree_before = sha256_hex(&before_bytes);
    let hunks = diff_hunks(states.clone());
    let applied_bytes = apply_validated_diff_hunks(&before_bytes, &hunks, &applied_hunks)
        .expect("public diff fixtures must apply their disclosed hunks");
    let working_tree_after = if write_count == 1 && !apply_failed {
        sha256_hex(&applied_bytes)
    } else {
        working_tree_before.clone()
    };
    let git_index = sha256_hex(diff_index_fixture_bytes());
    Some(DiffEvidence {
        patch_revision: "patch-retry-18".to_owned(),
        file_path: "src/net/retry_policy.rs".to_owned(),
        hunks,
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
    let byte_offset = if case_id.contains("page-88")
        || case_id.contains("long-final")
        || case_id == "permission-shell-long-return"
        || case_id.contains("middle-to-final")
        || case_id == "permission-shell-long-middle"
        || case_id.contains("final-to-return")
        || case_id == "permission-shell-long-page-44"
    {
        88
    } else if case_id == "permission-shell-long" || case_id == "permission-shell-long-page-0" {
        44
    } else {
        0
    };
    permission_evidence_at_offset(case_id, byte_offset)
}

fn permission_evidence_at_offset(case_id: &str, byte_offset: usize) -> Option<PermissionEvidence> {
    if case_id.starts_with("session-")
        || case_id.starts_with("shell-")
        || case_id.starts_with("todo-")
        || case_id.starts_with("chat-long-scroll-")
    {
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
    } else if case_id.contains("long") || case_id.starts_with("canonical-page-") {
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
    let page_end = (byte_offset + PERMISSION_PAGE_SIZE).min(visible_total_length);
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
        previous_offset: if byte_offset > 0 {
            Some(byte_offset.saturating_sub(PERMISSION_PAGE_SIZE))
        } else {
            None
        },
        next_offset: if page_end < visible_total_length {
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
            if *byte >= 0x80 || (escape_unsafe && (*byte < 0x20 || *byte == 0x7f)) {
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
        string_array(output, "old_payload", &hunk.old_payload);
        string_array(output, "new_payload", &hunk.new_payload);
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
/// Geometry is fixed, fully opaque, and sized for the 10-by-21 terminal cell.
/// Even 2px strokes form a left-rail vertical that meets a rightward arm at a
/// solid filled corner — the classic mono U+23BF ⎿ silhouette used for nest
/// markers, with stroke weight solid enough to stay readable at PNG scale.
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
    // Even 2px solid stroke. Tall left-rail stem mirrors mono U+23BF proportions
    // (much taller than a mid-cell capital letter foot) and the horizontal arm
    // springs from the solid bottom-left join to the right cell edge.
    let stem_left = cell_x + 3;
    let stem_right = cell_x + 4;
    let stem_top = cell_y + 3;
    let arm_top = cell_y + 14;
    let arm_bottom = cell_y + 15;
    let arm_right = cell_x + 9;

    for y in stem_top..=arm_bottom {
        for x in stem_left..=stem_right {
            put(image, x, y);
        }
    }
    for y in arm_top..=arm_bottom {
        for x in stem_left..=arm_right {
            put(image, x, y);
        }
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


fn login_choose_provider() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "login",
        vec!["model-picker", "login"],
        "Continue the paused conversation.",
        "blocked-model",
        "gpt-5-2-codex",
        "login-decision",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "connect-provider",
        vec!["model-picker", "login", "connect-provider"],
        "Continue the paused conversation.",
        "blocked-model",
        "openai",
        "provider-choice",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        1,
    );
    apply_flow4_provider_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.login.choose-provider".to_owned(),
        label: "Choose a provider".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the blocked model action opens provider selection".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.login.choose-provider.1".to_owned(),
        owner_id: "pending.model.gpt-5-2-codex".to_owned(),
        count: 0,
        outcome: "provider-catalogue-opened".to_owned(),
    };
    let effect = simulated_effect(
        "effect.login.choose-provider",
        "provider-connection",
        "provider-choice-opened",
        "login advanced to the provider catalogue without authentication",
    );
    (before, after, action, transition, attempt, effect)
}

fn connect_method_oauth() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec!["model-picker", "login", "connect-provider", "connect-method"];
    let mut before = state(
        "connect-method",
        origin.clone(),
        "Continue the paused conversation.",
        "blocked-model",
        "browser-sign-in",
        "method-choice",
        Some("pending.provider.openai"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "connect-oauth",
        origin,
        "Continue the paused conversation.",
        "blocked-model",
        "oauth-waiting",
        "oauth-wait",
        Some("pending.provider.openai"),
        "return-to-model-picker",
        1,
    );
    apply_flow4_provider_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.connect-method.oauth".to_owned(),
        label: "Continue with browser sign-in".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "the provider branch opens the selected sign-in method".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.connect-method.oauth.1".to_owned(),
        owner_id: "pending.provider.openai".to_owned(),
        count: 0,
        outcome: "oauth-wait-opened".to_owned(),
    };
    let effect = simulated_effect(
        "effect.connect-method.oauth",
        "provider-connection",
        "browser-sign-in-started",
        "browser sign-in started without writing credentials",
    );
    (before, after, action, transition, attempt, effect)
}

fn connect_oauth_finish() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let origin = vec![
        "model-picker",
        "login",
        "connect-provider",
        "connect-method",
        "connect-oauth",
    ];
    let mut before = state(
        "connect-oauth",
        origin.clone(),
        "Continue the paused conversation.",
        "blocked-model",
        "oauth-waiting",
        "oauth-wait",
        Some("pending.provider.openai"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = state(
        "connect-connected",
        origin,
        "Continue the paused conversation.",
        "blocked-model",
        "openai-connected",
        "connection-success",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        1,
    );
    apply_flow4_provider_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.connect-oauth.finish".to_owned(),
        label: "I've authorized".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "direct".to_owned(),
        path: "successful authentication records the connected provider".to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.connect-oauth.finish.1".to_owned(),
        owner_id: "pending.provider.openai".to_owned(),
        count: 1,
        outcome: "provider-connected".to_owned(),
    };
    let effect = simulated_effect(
        "effect.connect-oauth.finish",
        "provider-connection",
        "provider-connected",
        "OpenAI connection succeeded and restored the blocked-model origin",
    );
    (before, after, action, transition, attempt, effect)
}

fn provider_to_mcp_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "connect-connected",
        vec![
            "model-picker",
            "login",
            "connect-provider",
            "connect-method",
            "connect-oauth",
            "connect-connected",
        ],
        "Continue the paused conversation.",
        "blocked-model",
        "openai-connected",
        "connection-success",
        Some("pending.model.gpt-5-2-codex"),
        "return-to-model-picker",
        0,
    );
    apply_flow4_provider_identity(&mut before);
    let mut after = before.clone();
    after.screen_id = "mcp".to_owned();
    after.selection = "browser-automation".to_owned();
    after.focus = "capability-list".to_owned();
    after.return_policy = "return-to-chat-turn".to_owned();
    after.virtual_tick = 1;
    // Ordered detour rebinds MCP identity openly; it does not continue the provider task.
    apply_flow4_mcp_identity(&mut after);
    after.state_hash = state_hash(&after);
    let action = ActionEvidence {
        action_id: "action.storyboard.open-mcp-after-provider".to_owned(),
        label: "Open MCP connections separately".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "illustrative_detour".to_owned(),
        path: "provider success returns to its blocked model origin before an explicit separate MCP action opens capabilities"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.open-mcp-after-provider.1".to_owned(),
        owner_id: "storyboard.provider-and-mcp".to_owned(),
        count: 1,
        outcome: "mcp-opened-as-separate-capability-branch".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.open-mcp-after-provider",
        "storyboard-navigation",
        "illustrative-detour",
        "MCP opened only as an ordered illustrative detour with its own origin and task; provider success never owned a direct MCP edge",
    );
    (before, after, action, transition, attempt, effect)
}

fn error_network_to_rate_limit_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "error-network",
        vec!["chat-turn", "turn-error", "error-network"],
        "Keep drafting the open risk note.",
        "recovery-pause",
        "composer",
        "recovery-network",
        Some("pending.recovery.retry"),
        "return-to-chat-turn",
        0,
    );
    apply_flow5_identity(&mut before);
    let mut after = state(
        "rate-limit",
        vec!["chat-turn", "rate-limit"],
        "Keep drafting the open risk note.",
        "recovery-pause",
        "composer",
        "rate-limit-wait",
        Some("pending.recovery.retry"),
        "return-to-chat-turn",
        1,
    );
    apply_flow5_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.error-network.storyboard-rate-window".to_owned(),
        label: "Review the automatic wait that follows diagnosis".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "illustrative_detour".to_owned(),
        path: "connection diagnosis is reviewed first; the ordered storyboard next opens the automatic rate-limit wait as an illustrative adjacency, not a silent live navigation from this retry attempt"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.error-network.storyboard-rate-window.1".to_owned(),
        owner_id: "storyboard.recovery".to_owned(),
        count: 1,
        outcome: "rate-limit-reviewed-after-diagnosis".to_owned(),
    };
    let effect = simulated_effect(
        "effect.error-network.storyboard-rate-window",
        "storyboard-navigation",
        "illustrative-detour",
        "automatic waiting was reviewed as a separate recovery motif after connection diagnosis",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_monitor_to_failure_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor",
        vec!["chat-turn", "workflow-launch", "workflow-monitor"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-monitor",
        Some("pending.mission.monitor"),
        "return-to-chat-turn",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "workflow-monitor-failure",
        vec!["chat-turn", "workflow-monitor", "workflow-monitor-failure"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-failure",
        Some("pending.mission.fix"),
        "return-to-chat-turn",
        1,
    );
    apply_flow6_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.storyboard.open-mission-failure".to_owned(),
        label: "Open the validation failure".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "illustrative_detour".to_owned(),
        path: "a summarized visible validation stage surfaces only the actionable failure for review"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.open-mission-failure.1".to_owned(),
        owner_id: "storyboard.mission".to_owned(),
        count: 1,
        outcome: "failure-reviewed-after-compact-monitor".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.open-mission-failure",
        "storyboard-navigation",
        "illustrative-detour",
        "orderly storyboard adjacency opened Needs attention without inventing a live silent navigation",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_failure_to_review_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "workflow-monitor-failure",
        vec!["chat-turn", "workflow-monitor", "workflow-monitor-failure"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-failure",
        Some("pending.mission.fix"),
        "return-to-chat-turn",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-output",
        vec!["chat-turn", "workflow-monitor", "reviewer-output"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-review",
        Some("pending.mission.review"),
        "return-to-chat-turn",
        1,
    );
    apply_flow6_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.storyboard.summarize-fix-to-review".to_owned(),
        label: "Create a fix task".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "summarized".to_owned(),
        path: "fix planning, fix working, and fresh revalidation complete before the review outcome opens"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.summarize-fix-to-review.1".to_owned(),
        owner_id: "storyboard.mission".to_owned(),
        count: 1,
        outcome: "review-reached-after-summarized-fix-loop".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.summarize-fix-to-review",
        "storyboard-navigation",
        "summarized-path",
        "the ordered storyboard summarized fix work and revalidation before review",
    );
    (before, after, action, transition, attempt, effect)
}

fn mission_review_to_approved_bridge() -> (
    ScenarioState,
    ScenarioState,
    ActionEvidence,
    TransitionTrace,
    AttemptEvidence,
    SideEffectEvidence,
) {
    let mut before = state(
        "reviewer-output",
        vec!["chat-turn", "workflow-monitor", "reviewer-output"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-review",
        Some("pending.mission.review"),
        "return-to-chat-turn",
        0,
    );
    apply_flow6_identity(&mut before);
    let mut after = state(
        "reviewer-approved",
        vec!["chat-turn", "workflow-monitor", "reviewer-approved"],
        "Follow the release mission.",
        "mission-running",
        "mission-card",
        "mission-approval",
        Some("pending.mission.approval"),
        "return-to-chat-turn",
        1,
    );
    apply_flow6_identity(&mut after);
        after.origin_stack = before.origin_stack.clone();
    after.conversation_id = before.conversation_id.clone();
    after.transcript_id = before.transcript_id.clone();
    after.session_id = before.session_id.clone();
    after.turn_id = before.turn_id.clone();
    after.user_task = before.user_task.clone();
    after.visible_origin = before.visible_origin.clone();
    after.draft = before.draft.clone();
    after.transcript = before.transcript.clone();
    after.state_hash = state_hash(&after);
let action = ActionEvidence {
        action_id: "action.storyboard.summarize-review-to-approval".to_owned(),
        label: "Continue through review until human completion is ready".to_owned(),
        enabled: true,
        automatic: false,
    };
    let transition = TransitionTrace {
        action_id: action.action_id.clone(),
        from_screen_id: before.screen_id.clone(),
        to_screen_id: after.screen_id.clone(),
        kind: "summarized".to_owned(),
        path: "requested changes re-enter fix and revalidation; once review is ready, the approval frame opens for human completion"
            .to_owned(),
    };
    let attempt = AttemptEvidence {
        attempt_id: "attempt.storyboard.summarize-review-to-approval.1".to_owned(),
        owner_id: "storyboard.mission".to_owned(),
        count: 1,
        outcome: "approval-ready-after-summarized-review-loop".to_owned(),
    };
    let effect = simulated_effect(
        "effect.storyboard.summarize-review-to-approval",
        "storyboard-navigation",
        "summarized-path",
        "the ordered storyboard summarized review and fix loops until human completion was ready",
    );
    (before, after, action, transition, attempt, effect)
}
