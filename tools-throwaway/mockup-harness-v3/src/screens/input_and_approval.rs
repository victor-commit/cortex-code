//! Flow 2 multiline-composer and project-file mention fixtures.
//!
//! These two states keep the paused conversation visible while the bottom
//! surface either contains the authored draft itself or a focused file list.
//! The fixture deliberately records authored line boundaries separately from a
//! visual continuation, so a wrapped display line is never mistaken for a
//! newline the person entered.

use crate::scenario::{diff_evidence_for_fixture, diff_stage_state_for_fixture};
use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace,
    },
    chrome::{
        layout::{ScreenComposition, StatusLine, CANVAS_ROWS},
        tokens::TextStyle,
    },
    Surface,
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = 31;
const DOCK_BOTTOM: u16 = 37;
const FLOW2_FIXTURE: &str = "flow2.input-and-approval.stable";
const FLOW2_TASK: &str = "Check the retry path before we merge.";
const FLOW2_ORIGIN: &str = "Input and approval · paused conversation";
const FLOW2_SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
const FLOW2_LATER_DRAFT: &str = "Add a short note about the remaining test hunk.";

pub(crate) const MENTION_FRAGMENT: &str = "@retry";
pub(crate) const INSERTED_FILE: &str = "@src/net/retry_policy.rs";
pub(crate) const MULTILINE_DRAFT: &str = "Check the retry path in @retry\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
pub(crate) const SHORT_SHELL_COMMAND: &[u8] = b"printf '%s\\n' \"retry check\"";
pub(crate) const LONG_SHELL_COMMAND: &[u8] = b"printf '%s\\n' \"retry check\" && git diff -- src/net/retry_policy.rs > /tmp/retry.diff && printf '%s\\n' \"review complete\"";
pub(crate) const UNSAFE_SHELL_COMMAND: &[u8] = b"printf 'retry\0'\n\x1b[31m";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy)]
struct FileCandidate {
    path: &'static str,
    detail: &'static str,
}

const PROJECT_FILES: [FileCandidate; 3] = [
    FileCandidate {
        path: "src/net/retry_policy.rs",
        detail: "project file · best match",
    },
    FileCandidate {
        path: "src/net/retry_queue.rs",
        detail: "project file",
    },
    FileCandidate {
        path: "tests/net/retry_policy_test.rs",
        detail: "project file · test",
    },
];

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ActionDock, statusline.clone()).validate()?;

    let mut canvas = Canvas::new();
    if fixture_id == FLOW2_FIXTURE {
        canvas.write(2, 4, FLOW2_ORIGIN, TextStyle::muted())?;
        canvas.write(3, 4, FLOW2_TASK, TextStyle::bright())?;
    }
    let (candidates, events, transitions) = match screen_id {
        "composer-multiline" => render_multiline_composer(&mut canvas, screen_id, fixture_id)?,
        "file-mention" => render_file_mention(&mut canvas, screen_id, fixture_id)?,
        "permission-shell" => render_permission_shell(&mut canvas, screen_id, fixture_id)?,
        "permission-deny-reason" => {
            render_permission_deny_reason(&mut canvas, screen_id, fixture_id)?
        }
        "diff-edit" if fixture_id.starts_with("case.permission-allow-once") => {
            render_permission_resume(&mut canvas, screen_id, fixture_id)?
        }
        "diff-edit" => render_diff_edit(&mut canvas, screen_id, fixture_id)?,
        "diff-viewer-staging" => render_diff_staging(&mut canvas, screen_id, fixture_id)?,
        _ => return Err(format!("unknown Flow 2 composer screen: {screen_id}")),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: None,
        model_choice: None,
        conversation: (fixture_id == FLOW2_FIXTURE).then(|| crate::audit::ConversationIdentity {
            conversation_id: "conversation.retry-review".to_owned(),
            transcript_id: "transcript.retry-review".to_owned(),
            session_id: "session.retry-review.18".to_owned(),
            turn_id: "turn.retry-review.18".to_owned(),
            user_task: FLOW2_TASK.to_owned(),
            visible_origin: FLOW2_ORIGIN.to_owned(),
            submitted_prompt: Some(FLOW2_SUBMITTED_PROMPT.to_owned()),
            paused_activity_id: Some("activity.retry-policy-edit.18".to_owned()),
            later_draft: Some(FLOW2_LATER_DRAFT.to_owned()),
        }),
    })
}

fn render_diff_edit(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Check the project files before summarizing the retry result.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code · Pending edit", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Pending edit · nothing has been written.",
        TextStyle::accent(),
    )?;
    canvas.write(
        11,
        4,
        "File: src/net/retry_policy.rs · revision: patch-retry-18",
        TextStyle::text(),
    )?;
    canvas.write(12, 4, "Patch summary +4 -3 · 2 hunks", TextStyle::text())?;
    canvas.write(13, 4, "Hunk @@ -42,2 +42,3", TextStyle::text())?;
    canvas.write(14, 6, "42  42  -    return retry_after", TextStyle::text())?;
    canvas.write(
        15,
        6,
        "43  43  -    backoff = backoff + 1",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        6,
        "    42  +    return retry_after + jitter",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        6,
        "    43  +    backoff = backoff + jitter",
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        6,
        "    44  +    log_retry_window(jitter)",
        TextStyle::text(),
    )?;
    canvas.write(20, 4, "Hunk @@ -88,2 +88,2", TextStyle::text())?;
    canvas.write(21, 6, "88  88  -assert_eq!(wait, 4)", TextStyle::text())?;
    canvas.write(22, 6, "    89  +assert_eq!(wait, 5)", TextStyle::text())?;
    canvas.write(
        24,
        4,
        "Why this change: keep retry timing from repeating the same failed window.",
        TextStyle::muted(),
    )?;
    canvas.write(
        26,
        4,
        "The paused conversation and your draft stay unchanged until you decide.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Pending edit · choose one next step",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› [enter] Review changes · open read-only staging detail",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  [a] Apply all · write this exact patch and resume the paused turn",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · write nothing and return to the paused conversation",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Review is read-only · editing is available inside staging detail",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "pending-edit",
            "pending.edit.patch-retry-18",
            reference(9, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "file",
            "src/net/retry_policy.rs",
            reference(11, 4, 52),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-guard",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-test",
            reference(20, 4, 28),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "rationale",
            "retry-window-jitter",
            reference(24, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.diff-edit.review-changes",
            reference(DOCK_TOP + 2, 4, 60),
            "focused",
            Some(action(
                "action.diff-edit.review-changes",
                "Review changes",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.diff-edit.apply-all",
            reference(DOCK_TOP + 3, 4, 77),
            "summary",
            Some(action("action.diff-edit.apply-all", "Apply all", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.diff-edit.cancel",
            reference(DOCK_TOP + 4, 4, 78),
            "summary",
            Some(action("action.diff-edit.cancel", "Cancel", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    let transitions = vec![
        transition(
            "action.diff-edit.review-changes",
            "diff-edit",
            "diff-viewer-staging",
            "direct",
            "Review changes opens exact patch revision in read-only staging detail without writing",
        ),
        transition(
            "action.diff-edit.apply-all",
            "diff-edit",
            "chat-turn",
            "direct",
            "Apply all writes exactly the shown patch atomically and resumes the paused activity",
        ),
        transition(
            "action.diff-edit.cancel",
            "diff-edit",
            "chat-turn",
            "cancel",
            "Cancel writes nothing and restores the paused conversation and draft",
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_diff_staging(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mut stage = diff_stage_state_for_fixture(fixture_id);
    if let Some(diff) = diff_evidence_for_fixture(fixture_id) {
        stage.hunk_states = diff
            .hunks
            .iter()
            .map(|hunk| hunk.state.clone())
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| "staging evidence must contain exactly two hunks".to_owned())?;
        stage.focused_hunk = diff.focused_hunk;
    }
    let included_count = stage
        .hunk_states
        .iter()
        .filter(|state| state.as_str() == "Included")
        .count();
    let guard_state = &stage.hunk_states[0];
    let test_state = &stage.hunk_states[1];
    let guard_marker = match guard_state.as_str() {
        "Included" => "✓ Included",
        "Excluded" => "✗ Excluded",
        _ => "○ Pending",
    };
    let test_marker = match test_state.as_str() {
        "Included" => "✓ Included",
        "Excluded" => "✗ Excluded",
        _ => "○ Pending",
    };
    let focus_marker = |hunk_id: &str| {
        if stage.focused_hunk == hunk_id {
            "› "
        } else {
            ""
        }
    };
    let focus_style = |hunk_id: &str, state: &str| {
        if stage.focused_hunk == hunk_id && state != "Excluded" {
            TextStyle::bright()
        } else {
            TextStyle::text()
        }
    };
    canvas.write(
        5,
        4,
        "Review changes · temporary detail",
        TextStyle::bright(),
    )?;
    canvas.write(
        6,
        4,
        "From pending edit in the paused retry conversation",
        TextStyle::muted(),
    )?;
    canvas.write(
        9,
        4,
        "File: src/net/retry_policy.rs · revision: patch-retry-18",
        TextStyle::text(),
    )?;
    canvas.write(
        8,
        4,
        &format!("Included {included_count} of 2 hunks · Git index unchanged"),
        TextStyle::accent(),
    )?;
    canvas.write(
        10,
        4,
        &format!(
            "{}{} · hunk.retry-guard · old 42-43 · new 42-44",
            focus_marker("hunk.retry-guard"),
            guard_marker
        ),
        focus_style("hunk.retry-guard", guard_state),
    )?;
    canvas.write(11, 6, "42  42  -    return retry_after", TextStyle::text())?;
    canvas.write(
        12,
        6,
        "43  43  -    backoff = backoff + 1",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        6,
        "    42  +    return retry_after + jitter",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        6,
        "    43  +    backoff = backoff + jitter",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        6,
        "    44  +    log_retry_window(jitter)",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        &format!(
            "{}{} · hunk.retry-test · old 88-89 · new 88-89",
            focus_marker("hunk.retry-test"),
            test_marker
        ),
        focus_style("hunk.retry-test", test_state),
    )?;
    canvas.write(18, 6, "88  88  -assert_eq!(wait, 4)", TextStyle::text())?;
    canvas.write(19, 6, "    89  +assert_eq!(wait, 5)", TextStyle::text())?;
    canvas.write(
        21,
        4,
        if stage.apply_failed {
            "Finish stopped before writing: the patch is stale and no partial change was made."
        } else {
            "Include or Exclude changes the focused hunk; Include all selects both. No file is written until Finish."
        },
        TextStyle::muted(),
    )?;
    canvas.write(
        23,
        4,
        "[i] Include  [x] Exclude  [a] Include all  [e] Edit",
        TextStyle::bright(),
    )?;
    canvas.write(
        24,
        4,
        "[enter] Finish  [esc] Cancel · Return to pending edit",
        TextStyle::accent(),
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "detail-origin",
            "pending.edit.patch-retry-18",
            reference(6, 4, 60),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "file",
            "src/net/retry_policy.rs",
            reference(9, 4, 62),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "included-count",
            &format!("included-{included_count}-of-2"),
            reference(8, 4, 55),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-guard",
            reference(10, 4, 70),
            if stage.focused_hunk == "hunk.retry-guard" {
                "focused"
            } else {
                "summary"
            },
            Some(action(
                "action.staging.include-hunk",
                if guard_state == "Included" {
                    "Include"
                } else {
                    "Exclude"
                },
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "focused-hunk",
            &stage.focused_hunk,
            reference(10, 4, 70),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-test",
            reference(17, 4, 70),
            if stage.focused_hunk == "hunk.retry-test" {
                "focused"
            } else {
                "summary"
            },
            Some(action(
                "action.staging.exclude-hunk",
                if test_state == "Included" {
                    "Include"
                } else {
                    "Exclude"
                },
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "include-all-action",
            "include-all-hunks",
            reference(23, 4, 50),
            "summary",
            Some(action("action.staging.include-all", "Include all", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "edit-action",
            "edit-focused-hunk",
            reference(23, 52, 62),
            "summary",
            Some(action("action.staging.edit", "Edit", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "finish-included-hunks",
            reference(24, 4, 23),
            "focused",
            Some(action("action.staging.finish", "Finish", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "return-pending-edit",
            reference(24, 25, 60),
            "summary",
            Some(action("action.staging.cancel", "Cancel", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "statusline",
            "shared-statusline",
            reference(CANVAS_ROWS - 1, 2, 38),
            "summary",
            None,
        ),
    ];
    let finish_transition = if stage.apply_failed {
        transition(
            "action.staging.finish",
            "diff-viewer-staging",
            "diff-viewer-staging",
            "direct",
            "Finish stopped before writing because the patch is stale; the review remains open",
        )
    } else {
        transition(
            "action.staging.finish",
            "diff-viewer-staging",
            "chat-turn",
            "direct",
            "Finish applies only included hunks atomically and resumes the paused activity",
        )
    };
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.staging.include-hunk",
                "diff-viewer-staging",
                "diff-viewer-staging",
                "direct",
                "Include changes only the focused hunk and updates the textual count",
            ),
            transition(
                "action.staging.exclude-hunk",
                "diff-viewer-staging",
                "diff-viewer-staging",
                "direct",
                "Exclude changes only the focused hunk and updates the textual count",
            ),
            transition(
                "action.staging.include-all",
                "diff-viewer-staging",
                "diff-viewer-staging",
                "direct",
                "Include all marks every eligible hunk Included without writing",
            ),
            transition(
                "action.staging.edit",
                "diff-viewer-staging",
                "diff-viewer-staging",
                "direct",
                "Edit opens only from staging detail and does not approve the patch",
            ),
            finish_transition,
            transition(
                "action.staging.cancel",
                "diff-viewer-staging",
                "diff-edit",
                "cancel",
                "Cancel applies none and returns one level to the unchanged pending edit",
            ),
        ],
    ))
}

fn render_permission_shell(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let unsafe_bytes = fixture_id.contains("unsafe-byte");
    let return_to_choices =
        fixture_id.contains("long-return") || fixture_id.contains("final-to-return");
    let middle_to_final = fixture_id.contains("middle-to-final");
    let long_command = fixture_id.contains("long") && !return_to_choices;
    let out_of_scope = fixture_id.contains("persistent-scope-outside");
    let command = if out_of_scope {
        b"git diff -- src/docs/release_notes.md"
    } else if unsafe_bytes {
        UNSAFE_SHELL_COMMAND
    } else if long_command {
        LONG_SHELL_COMMAND
    } else {
        SHORT_SHELL_COMMAND
    };
    let hash = crate::audit::sha256_hex(command);
    let offset = if !return_to_choices
        && (fixture_id.contains("page-88")
        || fixture_id.contains("long-final")
        || fixture_id.contains("middle")
        || fixture_id.contains("middle-to-final"))
    {
        88
    } else if long_command {
        fixture_id
            .split("offset-")
            .nth(1)
            .and_then(|value| {
                value
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse::<usize>()
                    .ok()
            })
            .unwrap_or(0)
            .min(command.len())
    } else {
        0
    };
    let page_size = if long_command { 44 } else { command.len() };
    let page_end = (offset + page_size).min(command.len());
    let page = &command[offset..page_end];
    let notation = command_notation(page, unsafe_bytes || long_command);
    let page_label = if long_command {
        format!("bytes {offset}..{page_end} of {}", command.len())
    } else {
        format!("bytes 0..{} of {}", command.len(), command.len())
    };

    permission_transcript(canvas, unsafe_bytes)?;
    if return_to_choices {
        canvas.write(
            13,
            4,
            "Command detail closed · permission choices restored",
            TextStyle::bright(),
        )?;
        canvas.write(
            14,
            4,
            "Return changed no command bytes and ran nothing.",
            TextStyle::muted(),
        )?;
    } else if long_command {
        canvas.write(
            13,
            4,
            "Command detail · focused disclosure",
            TextStyle::bright(),
        )?;
        canvas.write(14, 4, &format!("Command {page_label}"), TextStyle::text())?;
        canvas.write(15, 4, &format!("  {notation}"), TextStyle::text())?;
        canvas.write(
            16,
            4,
            &format!("SHA-256 of exact execution bytes: {hash}"),
            TextStyle::muted(),
        )?;
        canvas.write(
            17,
            4,
            &format!(
                "Previous page: {} · Next page: {}",
                if offset == 0 {
                    "disabled".to_owned()
                } else {
                    offset.saturating_sub(page_size).to_string()
                },
                if page_end < command.len() {
                    page_end.to_string()
                } else {
                    "disabled".to_owned()
                }
            ),
            TextStyle::muted(),
        )?;
        canvas.write(
            18,
            4,
            "Use Next page or Previous page to inspect every byte window.",
            TextStyle::muted(),
        )?;
        canvas.write(
            19,
            4,
            if out_of_scope {
                "Working directory: ~/other-project · Scope: this command only"
            } else {
                "Working directory: ~/project · Scope: this command only"
            },
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(13, 4, "Command (case-sensitive)", TextStyle::bright())?;
        canvas.write(14, 4, &format!("  {}", notation), TextStyle::text())?;
        canvas.write(
            15,
            4,
            if out_of_scope {
                "Working directory: ~/other-project · Scope: this command only"
            } else {
                "Working directory: ~/project · Scope: this command only"
            },
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            &format!("Execution {page_label} · SHA-256: {hash}"),
            TextStyle::muted(),
        )?;
    }
    let persistent_supported = fixture_id.contains("persistent-supported")
        || (fixture_id.contains("permission-persistent-scope") && !fixture_id.contains("outside"))
        || fixture_id.contains("flow2.input-and-approval.stable")
        || fixture_id.contains("canonical.permission-shell");
    if persistent_supported {
        canvas.write(
            19,
            4,
            "Persistent scope · matcher: git diff -- src/net/* · duration: this session",
            TextStyle::muted(),
        )?;
        canvas.write(
            20,
            4,
            "Project root: ~/project · work style: Careful edits",
            TextStyle::muted(),
        )?;
        canvas.write(
            21,
            4,
            "Child actions: inherit only read-only checks · cap: same project root",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            19,
            4,
            if out_of_scope {
                "Persistent Allow: unavailable for this command and directory"
            } else {
                "Persistent Allow: unavailable for this command"
            },
            TextStyle::muted(),
        )?;
        canvas.write(
            20,
            4,
            "This request can run once only; no reusable rule will be offered.",
            TextStyle::muted(),
        )?;
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Permission choices · purpose and effect are shown above",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› [enter] Allow once · run once, then open the pending edit",
        TextStyle::accent(),
    )?;
    if persistent_supported {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "  [p] Allow for this project · save the exact scope shown above",
            TextStyle::text(),
        )?;
    } else {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "  Note · Persistent Allow unavailable; scope is not reusable",
            TextStyle::muted(),
        )?;
    }
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [d] Deny · do not run it",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        if long_command || return_to_choices {
            "[r] Return to permission choices · [esc] Cancel · keep request pending"
        } else {
            "[esc] Cancel · keep request pending"
        },
        TextStyle::muted(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "paused-activity",
            "shell.project-check",
            reference(7, 4, 38),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "purpose",
            "why-command-requested",
            reference(9, 4, 84),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "effect",
            "command-effect",
            reference(11, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "command",
            &format!("sha256-{hash}"),
            reference(14, 4, 4 + notation.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "execution-scope",
            "project-command-scope",
            reference(15, 4, 71),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "persistent-scope",
            if persistent_supported {
                "exact-project-session-scope"
            } else {
                "persistent-allow-unavailable"
            },
            reference(19, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.permission.allow-once",
            reference(DOCK_TOP + 2, 4, 67),
            "focused",
            Some(action("action.permission.allow-once", "Allow once", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.permission.deny",
            reference(DOCK_TOP + 4, 4, 32),
            "summary",
            Some(action("action.permission.deny", "Deny", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.permission.cancel",
            reference(DOCK_TOP + 5, 4, 46),
            "summary",
            Some(action(
                "action.permission.cancel",
                "Cancel, keep request pending",
                false,
            )),
        ),
    ];
    if persistent_supported {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.permission.allow-persistent",
            reference(DOCK_TOP + 3, 4, 73),
            "summary",
            Some(action(
                "action.permission.allow-persistent",
                "Allow for this project",
                false,
            )),
        ));
    }
    if long_command || return_to_choices {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "command-navigation",
            &format!("page-{offset}"),
            reference(17, 4, 74),
            "focused",
            None,
        ));
        if return_to_choices || !middle_to_final {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.permission.command-return",
                reference(DOCK_TOP + 5, 4, 48),
                "summary",
                Some(action(
                    "action.permission.command-return",
                    "Return to permission choices",
                    false,
                )),
            ));
        }
        if page_end < command.len() {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.permission.command-next",
                reference(17, 78, 96),
                "summary",
                Some(action("action.permission.command-next", "Next page", false)),
            ));
        }
        if middle_to_final {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "page-transition",
                "middle-to-final",
                reference(18, 4, 74),
                "summary",
                Some(action(
                    "action.permission.command-next-final",
                    "Open final page",
                    false,
                )),
            ));
        }
        if offset > 0 {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.permission.command-previous",
                reference(17, 4, 78),
                "summary",
                Some(action(
                    "action.permission.command-previous",
                    "Previous page",
                    false,
                )),
            ));
        }
    }
    let transitions = if return_to_choices {
        vec![transition(
            "action.permission.command-return",
            "permission-shell",
            "permission-shell",
            "cancel",
            "Return closes command detail and restores the same permission choices and focus",
        )]
    } else if long_command {
        let mut transitions = Vec::new();
        if !middle_to_final {
            transitions.push(transition(
                "action.permission.command-return",
                "permission-shell",
                "permission-shell",
                "cancel",
                "returning from command detail restores the same permission choices and command focus",
            ));
        }
        if page_end < command.len() && !middle_to_final {
            transitions.push(transition(
                "action.permission.command-next",
                "permission-shell",
                "permission-shell",
                "direct",
                "next page advances exactly one byte window without changing execution bytes",
            ));
        }
        if middle_to_final {
            transitions.push(transition(
                "action.permission.command-next-final",
                "permission-shell",
                "permission-shell",
                "direct",
                "the middle-page transition opens the distinct final page at the exact adjacent offset",
            ));
        }
        if offset > 0 {
            transitions.push(transition(
                "action.permission.command-previous",
                "permission-shell",
                "permission-shell",
                "direct",
                "previous page returns exactly one byte window without changing execution bytes",
            ));
        }
        transitions
    } else {
        let mut transitions = vec![
            transition(
                "action.permission.allow-once",
                "permission-shell",
                "diff-edit",
                "direct",
                "allow once runs only the disclosed command and opens its pending edit in the paused activity",
            ),
            transition(
                "action.permission.deny",
                "permission-shell",
                "permission-deny-reason",
                "direct",
                "denial records a textual verdict without executing the command",
            ),
            transition(
                "action.permission.cancel",
                "permission-shell",
                "chat-turn",
                "cancel",
                "cancel leaves the request pending and restores the paused conversation",
            ),
        ];
        if persistent_supported {
            transitions.push(transition(
                "action.permission.allow-persistent",
                "permission-shell",
                "chat-turn",
                "direct",
                "persistent allow stores only the disclosed bounded scope before resuming",
            ));
        }
        transitions
    };
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_permission_resume(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Check the project files before summarizing the retry result.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code · Permission accepted once", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Allowed once · the disclosed command ran exactly one time.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "The pending edit is ready for review. No reusable permission was saved.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "Working directory: ~/project · returning to the same paused turn",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Pending edit · nothing changes until you review it",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› [enter] Inspect changes",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[esc] Return to the paused conversation",
        TextStyle::muted(),
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "paused-turn",
            reference(4, 4, 72),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "verdict",
            "allowed-once",
            reference(9, 4, 63),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "scope",
            "no-reusable-permission",
            reference(11, 4, 75),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.diff-edit.open-review",
            reference(DOCK_TOP + 2, 4, 32),
            "focused",
            Some(action(
                "action.diff-edit.open-review",
                "Inspect changes",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.diff-edit.return-origin",
            reference(DOCK_TOP + 4, 4, 45),
            "summary",
            Some(action(
                "action.diff-edit.return-origin",
                "Return to the paused conversation",
                false,
            )),
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.diff-edit.open-review",
                "diff-edit",
                "diff-viewer-staging",
                "direct",
                "the one approved edit opens focused read-only staging evidence",
            ),
            transition(
                "action.diff-edit.return-origin",
                "diff-edit",
                "chat-turn",
                "cancel",
                "return restores the same paused conversation without another invocation",
            ),
        ],
    ))
}

fn permission_transcript(canvas: &mut Canvas, unsafe_bytes: bool) -> Result<(), String> {
    canvas.write(
        4,
        4,
        "You · Check the project files before summarizing the retry result.",
        TextStyle::bright(),
    )?;
    canvas.write(
        7,
        4,
        "Code paused · Shell command needs your approval",
        TextStyle::bright(),
    )?;
    canvas.write(
        9,
        4,
        "Why Code needs it: to confirm the retry changes before continuing this turn.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        if unsafe_bytes {
            "What running it will do: send the exact escaped bytes to the shell once."
        } else {
            "What running it will do: inspect the retry diff and keep the result in this turn."
        },
        TextStyle::text(),
    )?;
    Ok(())
}

fn render_permission_deny_reason(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    permission_transcript(canvas, false)?;
    canvas.write(13, 4, "Denied · command was not run", TextStyle::text())?;
    canvas.write(
        15,
        4,
        "Denied command: printf '%s\\n' \"retry check\"",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "Optional reason · this will be sent to Code with the denied activity",
        TextStyle::bright(),
    )?;
    canvas.write(18, 4, "› Type one short reason here", TextStyle::text())?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "The original Allow and Deny choices are closed",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› [enter] Send reason · add it to the denied activity",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  [s] Skip · return without adding a reason",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "[esc] Cancel · keep the denied activity unchanged",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "denied-activity",
            "shell.project-check",
            reference(13, 4, 40),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "denied-command",
            "printf-retry-check",
            reference(15, 4, 49),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "reason-input",
            "optional-denial-reason",
            reference(18, 4, 33),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.permission.send-reason",
            reference(DOCK_TOP + 2, 4, 59),
            "focused",
            Some(action(
                "action.permission.send-reason",
                "Send reason",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.permission.skip-reason",
            reference(DOCK_TOP + 3, 4, 55),
            "summary",
            Some(action("action.permission.skip-reason", "Skip", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.permission.cancel-reason",
            reference(DOCK_TOP + 5, 4, 57),
            "summary",
            Some(action("action.permission.cancel-reason", "Cancel", false)),
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.permission.send-reason",
                "permission-deny-reason",
                "chat-turn",
                "direct",
                "the exact optional reason is added to the denied activity and the same transcript resumes",
            ),
            transition(
                "action.permission.skip-reason",
                "permission-deny-reason",
                "chat-turn",
                "cancel",
                "skipping returns without adding a reason or executing the denied command",
            ),
            transition(
                "action.permission.cancel-reason",
                "permission-deny-reason",
                "chat-turn",
                "cancel",
                "cancelling the reason input preserves the denied activity unchanged",
            ),
        ],
    ))
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

fn render_multiline_composer(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let inserted_file = fixture_id.contains("file-mention-insert");
    let file_reference = if inserted_file {
        INSERTED_FILE
    } else {
        MENTION_FRAGMENT
    };
    let cursor_offset = if inserted_file { 48 } else { 30 };
    transcript(canvas)?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Draft · 3 authored lines · wraps visually · [@] Mention a file",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        &format!("› Check the retry path in {file_reference}| cursor at character {cursor_offset}"),
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Then explain the failure path that still needs coverage before the merge;",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "    wrap: include a concise test plan with the visible recovery behavior.",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;

    let transcript = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "paused-conversation",
        reference(4, 4, 83),
        "summary",
        None,
    );
    let draft = semantic(
        screen_id,
        fixture_id,
        "draft",
        "multiline-authored-draft",
        reference(DOCK_TOP + 2, 4, 61),
        "focused",
        None,
    );
    let first_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-1",
        reference(DOCK_TOP + 2, 6, 36),
        "focused",
        None,
    );
    let second_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-2",
        reference(DOCK_TOP + 3, 6, 80),
        "focused",
        None,
    );
    let third_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-3",
        reference(DOCK_TOP + 5, 6, 46),
        "focused",
        None,
    );
    let wrap = semantic(
        screen_id,
        fixture_id,
        "composer-wrap",
        "line-2-visual-continuation",
        reference(DOCK_TOP + 4, 8, 78),
        "focused",
        None,
    );
    let cursor = semantic(
        screen_id,
        fixture_id,
        "cursor",
        &format!("draft-offset-{cursor_offset}"),
        reference(
            DOCK_TOP + 2,
            28 + file_reference.chars().count() as u16,
            29 + file_reference.chars().count() as u16,
        ),
        "focused",
        None,
    );
    let mention = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.composer-multiline.open-file-mention",
        reference(DOCK_TOP + 1, 52, 69),
        "summary",
        Some(action(
            "action.composer-multiline.open-file-mention",
            "Mention a project file",
            false,
        )),
    );
    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    let pairs = [
        transcript,
        draft,
        first_newline,
        second_newline,
        third_newline,
        wrap,
        cursor,
        mention,
        status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.composer-multiline.open-file-mention",
            "composer-multiline",
            "file-mention",
            "direct",
            "the exact multiline draft opens a project-file mention dock at the composer",
        )],
    ))
}

fn render_file_mention(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    transcript(canvas)?;
    let query = if fixture_id.contains("no-results") {
        "@zzq"
    } else {
        MENTION_FRAGMENT
    };
    draft_preview(canvas, query)?;
    let matches = fuzzy_file_matches(query);
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "@ files · project-relative paths · query: {query} · {} matching project files",
            matches.len()
        ),
        TextStyle::bright(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "paused-conversation",
            reference(4, 4, 83),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(12, 4, 74),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            &format!("query-{}", query.trim_start_matches('@')),
            reference(DOCK_TOP + 1, 4, 68),
            "focused",
            None,
        ),
    ];

    if matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 2,
            &format!("No project files match {query}."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "Keep typing or dismiss to keep this literal draft.",
            TextStyle::muted(),
        )?;
        dock_line(canvas, DOCK_TOP + 4, "", TextStyle::text())?;
        dock_line(canvas, DOCK_TOP + 5, "[esc] Dismiss", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            &format!("no-match-{}", query.trim_start_matches('@')),
            reference(DOCK_TOP + 2, 4, 37),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.file-mention.dismiss",
            reference(DOCK_TOP + 5, 4, 17),
            "summary",
            Some(action("action.file-mention.dismiss", "Dismiss", false)),
        ));
    } else {
        for (index, file) in matches.iter().enumerate() {
            let row = DOCK_TOP + 2 + index as u16;
            let selected = index == 0;
            let marker = if selected { "›" } else { " " };
            dock_line(
                canvas,
                row,
                &format!("{marker} {} · {}", file.path, file.detail),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "file-match",
                file.path,
                reference(row, 6, 6 + file.path.chars().count() as u16),
                "focused",
                None,
            ));
            if selected {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "selection",
                    file.path,
                    reference(row, 4, 6 + file.path.chars().count() as u16),
                    "focused",
                    None,
                ));
            }
        }
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "› [enter] Submit prepared prompt  [tab] Insert file  [esc] Dismiss  [↑↓] Choose",
            TextStyle::bright(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.file-mention.submit-prepared-prompt",
            reference(DOCK_TOP + 5, 4, 36),
            "focused",
            Some(action(
                "action.file-mention.submit-prepared-prompt",
                "Submit prepared prompt",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.file-mention.insert-selected",
            reference(DOCK_TOP + 5, 38, 55),
            "summary",
            Some(action(
                "action.file-mention.insert-selected",
                "Insert file",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.file-mention.dismiss",
            reference(DOCK_TOP + 5, 57, 70),
            "summary",
            Some(action("action.file-mention.dismiss", "Dismiss", false)),
        ));
    }

    for row in DOCK_TOP + 2 + matches.len() as u16..=DOCK_TOP + 4 {
        if row <= DOCK_TOP + 4 && !matches.is_empty() {
            dock_line(canvas, row, "", TextStyle::text())?;
        }
    }
    if !matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "› [enter] Submit prepared prompt  [tab] Insert file  [esc] Dismiss  [↑↓] Choose",
            TextStyle::bright(),
        )?;
    }

    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    pairs.push(status);

    let transitions = if matches.is_empty() {
        vec![transition(
            "action.file-mention.dismiss",
            "file-mention",
            "composer-multiline",
            "cancel",
            "dismissal returns the literal visible query to its exact multiline draft",
        )]
    } else {
        vec![
            transition(
                "action.file-mention.submit-prepared-prompt",
                "file-mention",
                "permission-shell",
                "direct",
                "submission inserts the selected file into one immutable transcript prompt and pauses the same activity for permission",
            ),
            transition(
                "action.file-mention.insert-selected",
                "file-mention",
                "composer-multiline",
                "direct",
                "inserting one selected project file replaces only the active mention span",
            ),
            transition(
                "action.file-mention.dismiss",
                "file-mention",
                "composer-multiline",
                "cancel",
                "dismissal restores the literal visible query and original cursor",
            ),
        ]
    };
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn transcript(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        4,
        4,
        "You · Review the retry path before we merge.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I can add a project file without changing the rest of your draft.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn draft_preview(canvas: &mut Canvas, mention_fragment: &str) -> Result<(), String> {
    canvas.write(12, 4, "Paused draft · 3 authored lines", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        &format!("› Check the retry path in {mention_fragment}"),
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "  Then explain the failure path that still needs coverage before the merge;",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "    wrap: include a concise test plan with the visible recovery behavior.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn write_dock_border(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        DOCK_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        DOCK_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    Ok(())
}

fn dock_line(canvas: &mut Canvas, row: u16, text: &str, style: TextStyle) -> Result<(), String> {
    canvas.write(row, 2, "│", TextStyle::muted())?;
    canvas.write(row, 4, text, style)?;
    canvas.write(row, 117, "│", TextStyle::muted())?;
    Ok(())
}

fn fuzzy_file_matches(query: &str) -> Vec<FileCandidate> {
    let fragment = query.trim_start_matches('@').to_ascii_lowercase();
    let mut matches = PROJECT_FILES
        .iter()
        .copied()
        .filter(|file| fuzzy_subsequence(file.path, &fragment))
        .collect::<Vec<_>>();
    matches.sort_by_key(|file| fuzzy_rank(file.path, &fragment));
    matches
}

fn fuzzy_subsequence(path: &str, fragment: &str) -> bool {
    let mut remaining = fragment.chars();
    let mut wanted = remaining.next();
    for character in path.to_ascii_lowercase().chars() {
        if Some(character) == wanted {
            wanted = remaining.next();
            if wanted.is_none() {
                return true;
            }
        }
    }
    wanted.is_none()
}

fn fuzzy_rank<'a>(path: &'a str, fragment: &str) -> (usize, usize, &'a str) {
    let normalized = path.to_ascii_lowercase();
    let file_start = normalized.rfind('/').map_or(0, |index| index + 1);
    let direct_start = normalized[file_start..]
        .find(fragment)
        .map_or(usize::MAX, |index| index + file_start);
    let test_penalty = usize::from(normalized.contains("tests/"));
    (test_penalty, direct_start, path)
}

fn action(action_id: &str, label: &str, automatic: bool) -> ActionEvidence {
    ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic,
    }
}

fn reference(row: u16, start: u16, end: u16) -> RenderRef {
    RenderRef { row, start, end }
}

fn semantic(
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_ref: RenderRef,
    detail_level: &str,
    action: Option<ActionEvidence>,
) -> (SemanticCandidate, SemanticEvent) {
    let identity = Identity {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        candidate_id: format!("candidate.{family}.{subject_id}"),
        event_id: format!("event.{family}.{subject_id}"),
        family: family.to_owned(),
        subject_id: subject_id.to_owned(),
        render_refs: vec![render_ref],
        detail_level: detail_level.to_owned(),
    };
    (
        SemanticCandidate {
            identity: identity.clone(),
        },
        SemanticEvent { identity, action },
    )
}

fn transition(
    action_id: &str,
    from_screen_id: &str,
    to_screen_id: &str,
    kind: &str,
    path: &str,
) -> TransitionTrace {
    TransitionTrace {
        action_id: action_id.to_owned(),
        from_screen_id: from_screen_id.to_owned(),
        to_screen_id: to_screen_id.to_owned(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_file_matches, fuzzy_subsequence};

    #[test]
    fn fuzzy_file_results_are_query_matched_and_rank_the_project_policy_first() {
        let matches = fuzzy_file_matches("@retry");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].path, "src/net/retry_policy.rs");
        assert!(matches
            .iter()
            .all(|candidate| fuzzy_subsequence(candidate.path, "retry")));
        assert!(fuzzy_file_matches("@zzq").is_empty());
    }
}
