//! Flow 2 multiline-composer and project-file mention fixtures.
//!
//! These two states keep the paused conversation visible while the bottom
//! surface either contains the authored draft itself or a focused file list.
//! The fixture deliberately records authored line boundaries separately from a
//! visual continuation, so a wrapped display line is never mistaken for a
//! newline the person entered.

use crate::scenario::{
    diff_evidence_for_fixture, diff_stage_state_for_fixture, validated_diff_hunks_for_render,
};
use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{composer_header_meta, subject_title_for_origin},
        layout::{
            ScreenComposition, StatusLine, ACTION_DOCK_BOTTOM, ACTION_DOCK_TOP, CANVAS_ROWS,
        },
        tokens::TextStyle,
    },
    Surface,
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = ACTION_DOCK_TOP;
const DOCK_BOTTOM: u16 = ACTION_DOCK_BOTTOM;
const FLOW2_FIXTURE: &str = "flow2.input-and-approval.stable";
const FLOW2_TASK: &str = "Check the retry path before we merge.";
/// Continuous pause-window You-line: must match FLOW2_TASK verb/subject
/// (no Check/Review synonym swaps, no silent mid-window task rewrite).
const FLOW2_YOU_LINE: &str = "You · Check the retry path before we merge.";
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
/// Exact multiline draft before `/` opens the slash menu (cancel restores this).
pub(crate) const SLASH_PRE_OPEN_DRAFT: &str = "Check the retry path before we merge.\n\
Then explain the failure path that still needs coverage before the merge.\n\
Keep the summary short before we merge.";
/// Draft while the slash menu is open: active command span is the leading `/`.
pub(crate) const SLASH_OPEN_DRAFT: &str = "/\n\
Check the retry path before we merge.\n\
Then explain the failure path that still needs coverage before the merge.\n\
Keep the summary short before we merge.";
pub(crate) const SLASH_ACTIVE_SPAN: &str = "/";
pub(crate) const SLASH_CONFIRMED_EXPORT: &str = "/export";
/// Unchanged draft while the command palette is open (palette does not mutate it).
pub(crate) const PALETTE_PRESERVED_DRAFT: &str = MULTILINE_DRAFT;
/// Exact multiline draft while/after the attachment picker (token insertion is additive).
pub(crate) const ATTACHMENT_OPEN_DRAFT: &str = MULTILINE_DRAFT;
/// Selected attachable project-relative path (confirm inserts this once).
pub(crate) const ATTACHMENT_SELECTED_PATH: &str = "assets/retry-diagram.png";
/// Exact token chip inserted by confirming the selected eligible file.
pub(crate) const ATTACHMENT_TOKEN: &str = "[attachment: assets/retry-diagram.png]";
/// Exact multiline draft while the export picker is open (export never mutates it).
pub(crate) const EXPORT_OPEN_DRAFT: &str = MULTILINE_DRAFT;
/// Default and currently selected export format on open.
pub(crate) const EXPORT_DEFAULT_FORMAT: &str = "Markdown";
/// Sandboxed artifact path created when Markdown is confirmed.
pub(crate) const EXPORT_ARTIFACT_MARKDOWN: &str = "exports/retry-review.md";
/// Single inline success summary after a successful export.
pub(crate) const EXPORT_SUCCESS_SUMMARY: &str =
    "Exported conversation as Markdown · exports/retry-review.md";
/// Exact multiline draft while the queue manager is open (queue never submits it).
pub(crate) const QUEUE_OPEN_DRAFT: &str = MULTILINE_DRAFT;
/// Stable opened queue: two items with identity, revision, authored newlines,
/// mentions, and attachments that move as one payload unit.
pub(crate) const QUEUE_OPENED_AUTHORITY: &str = "1:queue-item-1:rev=1:hash=qa11 Also list unit tests.\nCover the guard edge cases.\n[attachment: queue-notes-a.md] | 2:queue-item-2:rev=2:hash=qb22 Then summarize risks.\n@src/net/retry_policy.rs\n[attachment: queue-notes-b.md]";
/// Shown applied order after reorder + run-next against the opened revisions.
pub(crate) const QUEUE_APPLIED_ORDER: &str = "1:queue-item-2:rev=2:hash=qb22 Then summarize risks.\n@src/net/retry_policy.rs\n[attachment: queue-notes-b.md] | 2:queue-item-1:rev=1:hash=qa11 Also list unit tests.\nCover the guard edge cases.\n[attachment: queue-notes-a.md]";
/// Authoritative residual after concurrent dequeue of item-1 (consumed; never resurrected).
pub(crate) const QUEUE_AFTER_DEQUEUE: &str = "1:queue-item-2:rev=2:hash=qb22 Then summarize risks.\n@src/net/retry_policy.rs\n[attachment: queue-notes-b.md]";
/// Filter string preserved across cancel/apply/refresh (empty default match-all).
pub(crate) const QUEUE_FILTER: &str = "query-empty";
/// One inline summary after a successful apply of the shown queue edits.
pub(crate) const QUEUE_APPLY_SUMMARY: &str =
    "Queue updated · 2 prompts · item-2 runs next";
pub(crate) const SHORT_SHELL_COMMAND: &[u8] = b"printf '%s\\n' \"retry check\"";
pub(crate) const LONG_SHELL_COMMAND: &[u8] = b"printf '%s\\n' \"retry check\" && git diff -- src/net/retry_policy.rs > /tmp/retry.diff && printf '%s\\n' \"review complete\"";
pub(crate) const UNSAFE_SHELL_COMMAND: &[u8] = b"printf 'retry\0'\n\x1b[31m";

/// Exact blocked recovery shell command for Flow 5 permission round-trip.
/// Kept byte-identical to recovery::BLOCKED_COMMAND / scenario TOOL_COMMAND.
pub(crate) const BLOCKED_RECOVERY_SHELL_COMMAND: &[u8] =
    b"rm -rf notes/archive && printf 'cleared\\n' >> notes/release.md";

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

#[derive(Clone, Copy)]
struct SlashCommand {
    name: &'static str,
    consequence: &'static str,
}

const SLASH_COMMANDS: [SlashCommand; 4] = [
    SlashCommand {
        name: "/export",
        consequence: "Export this conversation as a file",
    },
    SlashCommand {
        name: "/help",
        consequence: "Open the command reference",
    },
    SlashCommand {
        name: "/queue",
        consequence: "Manage queued prompts",
    },
    SlashCommand {
        name: "/actions",
        consequence: "Open the full action palette",
    },
];

#[derive(Clone, Copy)]
struct PaletteAction {
    label: &'static str,
    consequence: &'static str,
    opens_submenu: bool,
    subject_id: &'static str,
}

const PALETTE_ROOT_ACTIONS: [PaletteAction; 4] = [
    PaletteAction {
        label: "Recall an older prompt",
        consequence: "put an earlier prompt in the draft",
        opens_submenu: false,
        subject_id: "recall-older-prompt",
    },
    PaletteAction {
        label: "Stash this draft",
        consequence: "save it without sending",
        opens_submenu: false,
        subject_id: "stash-draft",
    },
    PaletteAction {
        label: "Restore stashed draft",
        consequence: "put the saved draft back in the composer",
        opens_submenu: false,
        subject_id: "restore-stashed-draft",
    },
    PaletteAction {
        label: "Queued prompts",
        consequence: "manage the queued prompt backlog",
        opens_submenu: true,
        subject_id: "queued-prompts",
    },
];

#[derive(Clone, Copy)]
struct SubmenuRow {
    label: &'static str,
    consequence: &'static str,
    subject_id: &'static str,
}

const QUEUED_SUBMENU_ROWS: [SubmenuRow; 3] = [
    SubmenuRow {
        label: "Add buffer cap tests",
        consequence: "run this prompt next",
        subject_id: "queued-add-buffer-cap-tests",
    },
    SubmenuRow {
        label: "Trace the retry backoff",
        consequence: "drop this queued prompt",
        subject_id: "queued-trace-retry-backoff",
    },
    SubmenuRow {
        label: "Document the rate-limit path",
        consequence: "run after the current turn",
        subject_id: "queued-document-rate-limit",
    },
];

#[derive(Clone, Copy)]
struct AttachmentCandidate {
    path: &'static str,
    detail: &'static str,
    eligible: bool,
}

const ATTACHMENT_CANDIDATES: [AttachmentCandidate; 5] = [
    AttachmentCandidate {
        path: "assets/retry-diagram.png",
        detail: "png · 48.2k · eligible",
        eligible: true,
    },
    AttachmentCandidate {
        path: "docs/retry-overview.pdf",
        detail: "pdf · 120.4k · eligible",
        eligible: true,
    },
    AttachmentCandidate {
        path: "assets/flow-sketch.svg",
        detail: "svg · 12.1k · eligible",
        eligible: true,
    },
    AttachmentCandidate {
        path: "README.md",
        detail: "md · 11.4k · unsupported",
        eligible: false,
    },
    AttachmentCandidate {
        path: "Cargo.toml",
        detail: "toml · 4.6k · unsupported",
        eligible: false,
    },
];

#[derive(Clone, Copy)]
struct ExportFormat {
    name: &'static str,
    artifact: &'static str,
    consequence: &'static str,
    is_default: bool,
}

const EXPORT_FORMATS: [ExportFormat; 4] = [
    ExportFormat {
        name: "Markdown",
        artifact: "readable .md",
        consequence: "headings and code",
        is_default: true,
    },
    ExportFormat {
        name: "JSON",
        artifact: "structured messages (.json)",
        consequence: "keeps each turn as data",
        is_default: false,
    },
    ExportFormat {
        name: "HTML",
        artifact: "styled shareable page (.html)",
        consequence: "ready to open in a browser",
        is_default: false,
    },
    ExportFormat {
        name: "Patch",
        artifact: "git-applyable diff (.patch)",
        consequence: "captures the change set only",
        is_default: false,
    },
];

#[derive(Clone, Copy)]
struct QueueItem {
    order: u8,
    identity: &'static str,
    revision: &'static str,
    /// One-line scannable summary; full authored payload rides in the idempotent queue ledger.
    summary: &'static str,
    unit_bits: &'static str,
    runs_next: bool,
}

const QUEUE_ITEMS: [QueueItem; 2] = [
    QueueItem {
        order: 1,
        identity: "queue-item-1",
        revision: "rev=1",
        summary: "Also list unit tests.",
        unit_bits: "[attachment: queue-notes-a.md]",
        runs_next: true,
    },
    QueueItem {
        order: 2,
        identity: "queue-item-2",
        revision: "rev=2",
        summary: "Then summarize risks.",
        unit_bits: "[attachment: queue-notes-b.md]",
        runs_next: false,
    },
];

/// Paint FLOW2 subject title + task chrome when the origin is the shared input
/// pause window **or** when a public/canonical permission gallery render would
/// otherwise open with only a You line (N-12).
fn paints_flow2_subject_chrome(screen_id: &str, fixture_id: &str) -> bool {
    if fixture_id == FLOW2_FIXTURE {
        return true;
    }
    matches!(
        screen_id,
        "permission-shell" | "permission-file-edit" | "permission-deny-reason"
    ) && (fixture_id == "public-audit"
        || fixture_id.starts_with("canonical.permission-")
        || fixture_id.starts_with("canonical."))
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ActionDock, statusline.clone()).validate()?;

    let mut canvas = Canvas::new();
    // Subject chrome for FLOW2 and for public/canonical permission IDs.
    // Gallery publication uses `public-audit` / `canonical.<id>` for screens
    // that are not bound to a flow step fixture; those still must paint a
    // human subject title (N-12) rather than jumping straight to the You line.
    if paints_flow2_subject_chrome(screen_id, fixture_id) {
        canvas.write(2, 4, subject_title_for_origin(FLOW2_ORIGIN), TextStyle::muted())?;
        canvas.write(3, 4, FLOW2_TASK, TextStyle::bright())?;
    }
    let (candidates, events, transitions) = match screen_id {
        "composer-multiline" => render_multiline_composer(&mut canvas, screen_id, fixture_id)?,
        "file-mention" => render_file_mention(&mut canvas, screen_id, fixture_id)?,
        "slash-menu" => render_slash_menu(&mut canvas, screen_id, fixture_id)?,
        "command-palette" => render_command_palette(&mut canvas, screen_id, fixture_id)?,
        "command-palette-submenu" => {
            render_command_palette_submenu(&mut canvas, screen_id, fixture_id)?
        }
        "attachment-picker" => render_attachment_picker(&mut canvas, screen_id, fixture_id)?,
        "export-picker" => render_export_picker(&mut canvas, screen_id, fixture_id)?,
        "queue-manager" => render_queue_manager(&mut canvas, screen_id, fixture_id)?,
        "permission-shell" => render_permission_shell(&mut canvas, screen_id, fixture_id)?,
        "permission-file-edit" => render_permission_file_edit(&mut canvas, screen_id, fixture_id)?,
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
    let hunks = validated_diff_hunks_for_render()?;
    let guard = &hunks[0];
    let test = &hunks[1];
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
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
    canvas.write(
        13,
        4,
        &format!(
            "Hunk @@ -{},{} +{},{}",
            guard.old_start, guard.old_lines, guard.new_start, guard.new_lines
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start, guard.new_start, guard.old_payload[0]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start + 1,
            guard.new_start + 1,
            guard.old_payload[1]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        6,
        &format!("    {:>2}  +{}", guard.new_start, guard.new_payload[0]),
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 1, guard.new_payload[1]),
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 2, guard.new_payload[2]),
        TextStyle::text(),
    )?;
    // Progressive disclosure: second hunk summarized until staging detail.
    canvas.write(
        20,
        4,
        &format!(
            "Second hunk · hunk.retry-test · @@ -{},{} +{},{} · open staging for lines",
            test.old_start, test.old_lines, test.new_start, test.new_lines
        ),
        TextStyle::muted(),
    )?;
    canvas.write(
        22,
        4,
        "Why this change: keep retry timing from repeating the same failed window.",
        TextStyle::muted(),
    )?;
    canvas.write(
        24,
        4,
        "Paused conversation and draft stay unchanged until you decide.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Pending edit",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› Review changes · open read-only staging detail",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Apply all · write this exact patch and resume",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  Cancel · write nothing",
        TextStyle::text(),
    )?;
    dock_line(canvas, DOCK_TOP + 5, "", TextStyle::muted())?;

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
            reference(20, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "rationale",
            "retry-window-jitter",
            reference(22, 4, 78),
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
            reference(DOCK_TOP + 3, 4, 58),
            "summary",
            Some(action("action.diff-edit.apply-all", "Apply all", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.diff-edit.cancel",
            reference(DOCK_TOP + 4, 4, 36),
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
    let hunks = validated_diff_hunks_for_render()?;
    let guard = &hunks[0];
    let test = &hunks[1];
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
    let patch_revision = if stage.refreshed {
        "patch-retry-19"
    } else {
        "patch-retry-18"
    };
    canvas.write(
        9,
        4,
        &format!("File: src/net/retry_policy.rs · revision: {patch_revision}"),
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
            "{}{} · hunk.retry-guard · old {},{} -> new {},{}",
            focus_marker("hunk.retry-guard"),
            guard_marker,
            guard.old_start,
            guard.old_lines,
            guard.new_start,
            guard.new_lines
        ),
        focus_style("hunk.retry-guard", guard_state),
    )?;
    canvas.write(
        11,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start, guard.new_start, guard.old_payload[0]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start + 1,
            guard.new_start + 1,
            guard.old_payload[1]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        6,
        &format!("    {:>2}  +{}", guard.new_start, guard.new_payload[0]),
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 1, guard.new_payload[1]),
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 2, guard.new_payload[2]),
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        &format!(
            "{}{} · hunk.retry-test · old {},{} -> new {},{}",
            focus_marker("hunk.retry-test"),
            test_marker,
            test.old_start,
            test.old_lines,
            test.new_start,
            test.new_lines
        ),
        focus_style("hunk.retry-test", test_state),
    )?;
    canvas.write(
        18,
        6,
        &format!(
            "{}  {:>2}  -{}",
            test.old_start, test.new_start, test.old_payload[0]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        19,
        6,
        &format!(
            "{}  {:>2}  -{}",
            test.old_start + 1,
            test.new_start + 1,
            test.old_payload[1]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        20,
        6,
        &format!("    {:>2}  +{}", test.new_start, test.new_payload[0]),
        TextStyle::text(),
    )?;
    canvas.write(
        21,
        6,
        &format!("    {:>2}  +{}", test.new_start + 1, test.new_payload[1]),
        TextStyle::text(),
    )?;
    canvas.write(
        23,
        4,
        if stage.apply_failed {
            "Workspace changed after review. No hunks were written. Refresh and review to continue."
        } else if stage.refreshed {
            "Refreshed review · unchanged Included/Excluded kept · Finish writes only after this review."
        } else {
            "No file is written until Finish."
        },
        TextStyle::muted(),
    )?;
    let chrome = if stage.apply_failed {
        "Enter to refresh and review · Esc to cancel"
    } else {
        "i include · x exclude · a include all · e edit · Enter to finish · Esc to cancel"
    };
    canvas.write(25, 4, chrome, TextStyle::bright())?;
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "detail-origin",
            &format!("pending.edit.{patch_revision}"),
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
            "focused-hunk",
            &stage.focused_hunk,
            reference(10, 4, 70),
            "focused",
            None,
        ),
    ];
    if stage.apply_failed {
        // Hunk rows remain visible for context, without local include/exclude while
        // the patch is stale (mutations resume only after Refresh and review).
        pairs.push(semantic(
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
            None,
        ));
        pairs.push(semantic(
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
            None,
        ));
        // Stale finish is fail-closed: Finish stays unavailable until an explicit refresh.
        // Chrome: "Enter to refresh and review · Esc to cancel" written at column 4.
        // Labels covered: "refresh and review" @13..31, "cancel" @41..47.
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "refresh-and-review",
            reference(25, 13, 31),
            "focused",
            Some(action(
                "action.staging.refresh",
                "Refresh and review",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "return-pending-edit",
            reference(25, 41, 47),
            "summary",
            Some(action("action.staging.cancel", "Cancel", false)),
        ));
    } else {
        pairs.push(semantic(
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
        ));
        pairs.push(semantic(
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
        ));
        // Staging chrome (Phase-A prose):
        // "i include · x exclude · a include all · e edit · Enter to finish · Esc to cancel"
        // written at column 4. Cover the human label spans after the local letter key.
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "include-all-action",
            "include-all-hunks",
            reference(25, 30, 41),
            "summary",
            Some(action("action.staging.include-all", "Include all", false)),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "edit-action",
            "edit-focused-hunk",
            reference(25, 46, 50),
            "summary",
            Some(action("action.staging.edit", "Edit", false)),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "finish-included-hunks",
            reference(25, 62, 68),
            "focused",
            Some(action("action.staging.finish", "Finish", false)),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "return-pending-edit",
            reference(25, 78, 84),
            "summary",
            Some(action("action.staging.cancel", "Cancel", false)),
        ));
    }
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));
    let transitions = if stage.apply_failed {
        // Stale workspace: only Refresh and review (primary) and Cancel remain.
        // The refused Finish attempt is recorded on the scenario side-effect path
        // that landed here; the after-state must not re-announce Finish as live.
        vec![
            transition(
                "action.staging.refresh",
                "diff-viewer-staging",
                "diff-viewer-staging",
                "direct",
                "Refresh writes nothing, builds a new patch revision, and reopens staging review",
            ),
            transition(
                "action.staging.cancel",
                "diff-viewer-staging",
                "diff-edit",
                "cancel",
                "Cancel applies none and returns one level to the unchanged pending edit",
            ),
        ]
    } else {
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
            transition(
                "action.staging.finish",
                "diff-viewer-staging",
                "chat-turn",
                "direct",
                "Finish applies only included hunks atomically and resumes the paused activity",
            ),
            transition(
                "action.staging.cancel",
                "diff-viewer-staging",
                "diff-edit",
                "cancel",
                "Cancel applies none and returns one level to the unchanged pending edit",
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

fn render_permission_shell(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let recovery_blocked = fixture_id.contains("blocked-command")
        || fixture_id.contains("recovery.blocked");
    let unsafe_bytes = fixture_id.contains("unsafe-byte");
    let return_to_choices =
        fixture_id.contains("long-return") || fixture_id.contains("final-to-return");
    let long_command = fixture_id.contains("long") && !return_to_choices;
    let out_of_scope = fixture_id.contains("persistent-scope-outside");
    let command = if recovery_blocked {
        BLOCKED_RECOVERY_SHELL_COMMAND
    } else if out_of_scope {
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
        && (fixture_id.contains("after-offset-88")
            || fixture_id.contains("page-88")
            || fixture_id.contains("long-final")
            || fixture_id.contains("middle"))
    {
        88
    } else if !return_to_choices && fixture_id.contains("after-offset-44") {
        44
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

    permission_transcript(canvas, unsafe_bytes, recovery_blocked)?;
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
        // Summary-first for short commands: hash evidence waits on expand/detail.
        canvas.write(
            16,
            4,
            "Command detail collapsed · e to expand",
            TextStyle::muted(),
        )?;
    }
    let persistent_supported = fixture_id.contains("persistent-supported")
        || (fixture_id.contains("permission-persistent-scope") && !fixture_id.contains("outside"))
        || fixture_id.contains("flow2.input-and-approval.stable")
        || fixture_id.contains("canonical.permission-shell");
    let persistent_row = if long_command { 20 } else { 18 };
    if persistent_supported {
        canvas.write(
            persistent_row,
            4,
            "Persistent: this session · matcher git diff -- src/net/* · Careful edits",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            persistent_row,
            4,
            if out_of_scope {
                "Persistent Allow: unavailable for this command and directory"
            } else {
                "Persistent Allow: unavailable for this command"
            },
            TextStyle::muted(),
        )?;
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        if recovery_blocked {
            "Permission choices · approve only, then return for an explicit run"
        } else {
            "Permission choices"
        },
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        if recovery_blocked {
            "› Allow once · do not run yet"
        } else {
            "› Allow once · run once, then open the pending edit"
        },
        TextStyle::accent(),
    )?;
    if persistent_supported {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "  Allow for this project · save the exact scope shown above",
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
        "  Deny · do not run it",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        if long_command {
            "r to return to permission choices · Esc to cancel"
        } else {
            "Esc to cancel · keep request pending"
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
            reference(
                if long_command { 15 } else { 14 },
                4,
                4 + notation.chars().count() as u16,
            ),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "execution-scope",
            "project-command-scope",
            reference(if long_command { 19 } else { 15 }, 4, 71),
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
            reference(persistent_row, 4, 78),
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
    if return_to_choices {
        pairs.retain(|(_, event)| {
            !matches!(
                event.identity.family.as_str(),
                "command" | "execution-scope"
            )
        });
    }
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
    if long_command {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "command-navigation",
            &format!("page-{offset}"),
            reference(17, 4, 74),
            "focused",
            None,
        ));
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
        if page_end < command.len() {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                if page_end + page_size >= command.len() {
                    "page-transition"
                } else {
                    "secondary-action"
                },
                if page_end + page_size >= command.len() {
                    "action.permission.command-next-final"
                } else {
                    "action.permission.command-next"
                },
                reference(17, 78, 96),
                "summary",
                Some(action(
                    if page_end + page_size >= command.len() {
                        "action.permission.command-next-final"
                    } else {
                        "action.permission.command-next"
                    },
                    if page_end + page_size >= command.len() {
                        "Open final page"
                    } else {
                        "Next page"
                    },
                    false,
                )),
            ));
        }
    }
    let transitions = if return_to_choices {
        vec![
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
        ]
    } else if long_command {
        let mut transitions = Vec::new();
        transitions.push(transition(
            "action.permission.command-return",
            "permission-shell",
            "permission-shell",
            "cancel",
            "returning from command detail restores the same permission choices and command focus",
        ));
        transitions.push(transition(
            "action.permission.command-previous",
            "permission-shell",
            "permission-shell",
            "direct",
            "previous page returns exactly one byte window without changing execution bytes",
        ));
        if page_end < command.len() {
            let opens_final_page = page_end + page_size >= command.len();
            transitions.push(transition(
                if opens_final_page {
                    "action.permission.command-next-final"
                } else {
                    "action.permission.command-next"
                },
                "permission-shell",
                "permission-shell",
                "direct",
                if opens_final_page {
                    "the middle-page transition opens the distinct final page at the exact adjacent offset"
                } else {
                    "next page advances exactly one byte window without changing execution bytes"
                },
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
    let transitions = if recovery_blocked {
        vec![
            transition(
                "action.permission.allow-once",
                "permission-shell",
                "blocked-command",
                "direct",
                "allow once records approval without executing and returns for one explicit run",
            ),
            transition(
                "action.permission.deny",
                "permission-shell",
                "blocked-command",
                "direct",
                "denial returns to the blocked command without executing it",
            ),
            transition(
                "action.permission.cancel",
                "permission-shell",
                "blocked-command",
                "cancel",
                "cancel leaves the request pending on the blocked command without execution",
            ),
        ]
    } else {
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
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
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
        "› Inspect changes",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "Esc to return to the paused conversation",
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

fn permission_transcript(
    canvas: &mut Canvas,
    unsafe_bytes: bool,
    recovery_blocked: bool,
) -> Result<(), String> {
    if recovery_blocked {
        canvas.write(2, 4, "Recovery · paused conversation", TextStyle::muted())?;
        canvas.write(
            3,
            4,
            "Finish the release notes without losing the current work.",
            TextStyle::bright(),
        )?;
        canvas.write(
            4,
            4,
            "You · Summarize the remaining release notes and the still-open risks.",
            TextStyle::bright(),
        )?;
        canvas.write(
            7,
            4,
            "Code paused · Blocked command needs your approval",
            TextStyle::bright(),
        )?;
        canvas.write(
            9,
            4,
            "Why Code needs it: the blocked write was held and still needs an explicit allow.",
            TextStyle::text(),
        )?;
        canvas.write(
            11,
            4,
            "What running it will do: remove notes/archive and append a cleared marker to notes/release.md.",
            TextStyle::text(),
        )?;
        return Ok(());
    }
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
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

fn render_permission_file_edit(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let out_of_scope = fixture_id.contains("scope-outside") || fixture_id.contains("outside");
    let persistent_supported = !out_of_scope
        && (fixture_id.contains("persistent")
            || fixture_id.contains("flow2-permission-file-edit")
            || fixture_id.contains("canonical.permission-file-edit")
            || fixture_id.contains("public-audit")
            || fixture_id.contains("flow2.input-and-approval.stable"));
    let hunks = validated_diff_hunks_for_render()?;
    let guard = &hunks[0];
    let test = &hunks[1];
    let target_file = if out_of_scope {
        "src/docs/release_notes.md"
    } else {
        "src/net/retry_policy.rs"
    };

    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(
        7,
        4,
        "Code paused · File edit needs your approval",
        TextStyle::bright(),
    )?;
    canvas.write(
        9,
        4,
        "Why Code needs it: to keep retry timing from repeating the same failed window.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "What applying it will do: write the disclosed patch into the project file once.",
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        4,
        &format!("File: {target_file} · revision: patch-retry-18"),
        TextStyle::text(),
    )?;
    canvas.write(13, 4, "Patch summary +4 -3 · 2 hunks", TextStyle::text())?;
    canvas.write(
        14,
        4,
        &format!(
            "Hunk @@ -{},{} +{},{}",
            guard.old_start, guard.old_lines, guard.new_start, guard.new_lines
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start, guard.new_start, guard.old_payload[0]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        6,
        &format!(
            "{}  {:>2}  -{}",
            guard.old_start + 1,
            guard.new_start + 1,
            guard.old_payload[1]
        ),
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        6,
        &format!("    {:>2}  +{}", guard.new_start, guard.new_payload[0]),
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 1, guard.new_payload[1]),
        TextStyle::text(),
    )?;
    canvas.write(
        19,
        6,
        &format!("    {:>2}  +{}", guard.new_start + 2, guard.new_payload[2]),
        TextStyle::text(),
    )?;
    canvas.write(
        20,
        4,
        &format!(
            "Second hunk · hunk.retry-test · @@ -{},{} +{},{} · e to review full patch",
            test.old_start, test.old_lines, test.new_start, test.new_lines
        ),
        TextStyle::muted(),
    )?;
    canvas.write(
        21,
        4,
        "Scope: this edit only · project root ~/project · work style Careful edits",
        TextStyle::muted(),
    )?;
    if persistent_supported {
        canvas.write(
            22,
            4,
            "Persistent: this session · matcher src/net/retry_policy.rs · Careful edits",
            TextStyle::muted(),
        )?;
        canvas.write(
            23,
            4,
            "Children inherit only read-only checks · child cap: no write elevation",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            22,
            4,
            "Persistent Allow: unavailable · request is outside the saved scope",
            TextStyle::muted(),
        )?;
        canvas.write(
            23,
            4,
            "new permission required · children remain capped by the parent",
            TextStyle::muted(),
        )?;
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Permission choices",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› Allow once · apply only the disclosed patch",
        TextStyle::accent(),
    )?;
    if persistent_supported {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "  Allow for this project · save the exact scope shown above",
            TextStyle::text(),
        )?;
    } else {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "  Note · Persistent Allow unavailable for this outside request",
            TextStyle::muted(),
        )?;
    }
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  Deny · do not write the file",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "e to review full patch · Esc to cancel",
        TextStyle::muted(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "paused-activity",
            "file-edit.retry-policy",
            reference(7, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "purpose",
            "why-edit-requested",
            reference(9, 4, 86),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "effect",
            "edit-effect",
            reference(11, 4, 86),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "file",
            target_file,
            reference(12, 4, 58),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-guard",
            reference(14, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "hunk",
            "hunk.retry-test",
            reference(20, 4, 88),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "execution-scope",
            "this-edit-only",
            reference(21, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "persistent-scope",
            if persistent_supported {
                "exact-file-session-scope"
            } else {
                "persistent-allow-unavailable"
            },
            reference(22, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.permission.allow-once",
            reference(DOCK_TOP + 2, 4, 64),
            "focused",
            Some(action("action.permission.allow-once", "Allow once", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.permission.deny",
            reference(DOCK_TOP + 4, 4, 38),
            "summary",
            Some(action("action.permission.deny", "Deny", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.permission.cancel",
            reference(DOCK_TOP + 5, 4, 62),
            "summary",
            Some(action(
                "action.permission.cancel",
                "Cancel, keep request pending",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            // Progressive disclosure of the full patch — not a competing decision
            // secondary (decision secondaries remain Deny + optional persistent allow).
            "disclosure-action",
            "action.permission.review-detail",
            reference(DOCK_TOP + 5, 4, 24),
            "summary",
            Some(action(
                "action.permission.review-detail",
                "Review full patch",
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
    let mut transitions = vec![
        transition(
            "action.permission.allow-once",
            "permission-file-edit",
            "chat-turn",
            "direct",
            "allow once writes only the disclosed patch and resumes the paused activity",
        ),
        transition(
            "action.permission.deny",
            "permission-file-edit",
            "permission-deny-reason",
            "direct",
            "denial writes nothing and opens the optional reason dock with denied edit context",
        ),
        transition(
            "action.permission.cancel",
            "permission-file-edit",
            "chat-turn",
            "cancel",
            "cancel leaves the request pending and restores the exact paused conversation",
        ),
        transition(
            "action.permission.review-detail",
            "permission-file-edit",
            "diff-viewer-staging",
            "direct",
            "review opens exact patch detail without writing or approving the edit",
        ),
    ];
    if persistent_supported {
        transitions.push(transition(
            "action.permission.allow-persistent",
            "permission-file-edit",
            "chat-turn",
            "direct",
            "persistent allow stores only the disclosed matcher, duration, project root, work style, and child cap",
        ));
    }
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_permission_deny_reason(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let file_edit_denial = fixture_id.contains("file-edit");
    if file_edit_denial {
        canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
        canvas.write(
            7,
            4,
            "Code paused · File edit needs your approval",
            TextStyle::bright(),
        )?;
        canvas.write(
            9,
            4,
            "Why Code needs it: to keep retry timing from repeating the same failed window.",
            TextStyle::text(),
        )?;
        canvas.write(
            11,
            4,
            "What applying it will do: write the disclosed patch into the project file once.",
            TextStyle::text(),
        )?;
        canvas.write(13, 4, "Denied · file edit was not applied", TextStyle::text())?;
        canvas.write(
            15,
            4,
            "Denied file edit: src/net/retry_policy.rs · patch-retry-18",
            TextStyle::text(),
        )?;
    } else {
        permission_transcript(canvas, false, false)?;
        canvas.write(13, 4, "Denied · command was not run", TextStyle::text())?;
        canvas.write(
            15,
            4,
            "Denied command: printf '%s\\n' \"retry check\"",
            TextStyle::text(),
        )?;
    }
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
        "› Send reason · add it to the denied activity",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Skip · return without adding a reason",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Esc to cancel · keep the denied activity unchanged",
        TextStyle::muted(),
    )?;

    let denied_subject = if file_edit_denial {
        "file-edit.retry-policy"
    } else {
        "shell.project-check"
    };
    let denied_command_subject = if file_edit_denial {
        "src/net/retry_policy.rs"
    } else {
        "printf-retry-check"
    };
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "denied-activity",
            denied_subject,
            reference(13, 4, 40),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "denied-command",
            denied_command_subject,
            reference(15, 4, 58),
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
                "skipping returns without adding a reason or executing the denied action",
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
        &composer_header_meta(3)?,
        TextStyle::bright(),
    )?;
    // Cursor remains available as semantic evidence (subject draft-offset-N) but
    // is no longer painted as implementation HUD text inside the composer jewel.
    dock_line(
        canvas,
        DOCK_TOP + 2,
        &format!("› Check the retry path in {file_reference}█"),
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
        "    include a concise test plan with the visible recovery behavior.",
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
        dock_line(canvas, DOCK_TOP + 5, "Esc to dismiss", TextStyle::bright())?;
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
            "› Submit prepared prompt · Tab to insert file · Esc to dismiss · ↑↓ to choose",
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
            "› Submit prepared prompt · Tab to insert file · Esc to dismiss · ↑↓ to choose",
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

fn render_slash_menu(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    transcript(canvas)?;
    let query = if fixture_id.contains("no-results") {
        "/zzq"
    } else if fixture_id.contains("filter-export") {
        "/ex"
    } else {
        "/"
    };
    let fragment = query.trim_start_matches('/');
    slash_draft_preview(canvas, query)?;
    let matches = slash_matches(fragment);
    write_dock_border(canvas)?;
    // Header carries match count and local keys so all four commands fit the dock body.
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "/ commands · query: {query} · {} matching · Enter to confirm · Esc to cancel",
            matches.len()
        ),
        TextStyle::bright(),
    )?;

    // Keep a stable owned subject id for filtered cases.
    let filter_subject_owned = if fragment == "ex" {
        "query-ex".to_owned()
    } else if fragment.is_empty() {
        "query-empty".to_owned()
    } else {
        format!("query-{fragment}")
    };

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
            "origin",
            "composer-multiline",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(13, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "active-command-span",
            &format!("span-{query}"),
            reference(13, 6, 6 + query.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            &filter_subject_owned,
            reference(DOCK_TOP + 1, 4, 72),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.slash-menu.confirm-selected",
            reference(DOCK_TOP + 1, 52, 70),
            "summary",
            Some(action(
                "action.slash-menu.confirm-selected",
                "Confirm command",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.slash-menu.cancel",
            reference(DOCK_TOP + 1, 74, 88),
            "summary",
            Some(action("action.slash-menu.cancel", "Cancel", false)),
        ),
    ];

    if matches.is_empty() {
        // Remove confirm when nothing is selectable; cancel remains.
        pairs.retain(|(_, event)| event.identity.family != "primary-action");
        dock_line(
            canvas,
            DOCK_TOP + 2,
            &format!("No commands match {query}."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "Keep typing or cancel to restore the draft before /.",
            TextStyle::muted(),
        )?;
        dock_line(canvas, DOCK_TOP + 4, "", TextStyle::text())?;
        dock_line(canvas, DOCK_TOP + 5, "", TextStyle::text())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            &format!("no-match-{fragment}"),
            reference(DOCK_TOP + 2, 4, 32 + query.chars().count() as u16),
            "focused",
            None,
        ));
    } else {
        for (index, command) in matches.iter().enumerate() {
            let row = DOCK_TOP + 2 + index as u16;
            if row > DOCK_TOP + 5 {
                break;
            }
            let selected = index == 0;
            let marker = if selected { "›" } else { " " };
            dock_line(
                canvas,
                row,
                &format!("{marker} {} · {}", command.name, command.consequence),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "command-match",
                command.name,
                reference(row, 6, 6 + command.name.chars().count() as u16),
                "focused",
                None,
            ));
            if selected {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "selection",
                    command.name,
                    reference(row, 4, 6 + command.name.chars().count() as u16),
                    "focused",
                    None,
                ));
            }
        }
        for row in DOCK_TOP + 2 + matches.len().min(4) as u16..=DOCK_TOP + 5 {
            dock_line(canvas, row, "", TextStyle::text())?;
        }
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    let transitions = if matches.is_empty() {
        vec![transition(
            "action.slash-menu.cancel",
            "slash-menu",
            "composer-multiline",
            "cancel",
            "cancel restores the exact draft, cursor, height, selection, and scroll from before / opened",
        )]
    } else {
        vec![
            transition(
                "action.slash-menu.confirm-selected",
                "slash-menu",
                "composer-multiline",
                "direct",
                "confirm replaces only the active command span and keeps surrounding draft text byte-stable",
            ),
            transition(
                "action.slash-menu.cancel",
                "slash-menu",
                "composer-multiline",
                "cancel",
                "cancel restores the exact draft, cursor, height, selection, and scroll from before / opened",
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

fn render_attachment_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    transcript(canvas)?;
    draft_preview(canvas, MENTION_FRAGMENT)?;
    let query = if fixture_id.contains("no-results") {
        "zzq"
    } else if fixture_id.contains("filter-png") {
        "png"
    } else {
        ""
    };
    let matches = attachment_matches(query);
    let first_eligible = matches.iter().position(|row| row.eligible);
    write_dock_border(canvas)?;

    // Dock body text starts at column 4 and must end before the border at 117.
    let header = if matches.is_empty() {
        format!(
            "Attach file · project-relative paths · query: {query} · 0 matching"
        )
    } else if first_eligible.is_some() {
        format!(
            "Attach file · project-relative paths · query: {query} · {} matching · Enter to attach · Esc to cancel",
            matches.len()
        )
    } else {
        format!(
            "Attach file · project-relative paths · query: {query} · {} matching · Esc to cancel",
            matches.len()
        )
    };
    debug_assert!(
        header.chars().count() <= 113,
        "attachment picker header overflowed the dock body"
    );
    dock_line(canvas, DOCK_TOP + 1, &header, TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Attaches one project file to this draft · png, jpg, pdf, svg",
        TextStyle::muted(),
    )?;

    let filter_subject = if query.is_empty() {
        "query-empty".to_owned()
    } else {
        format!("query-{query}")
    };
    let attach_label = "Enter to attach";
    let cancel_label = "Esc to cancel";
    let attach_start = header.find(attach_label).map(|index| 4 + index as u16);
    let cancel_start = header
        .find(cancel_label)
        .map(|index| 4 + index as u16)
        .unwrap_or(4);

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
            "origin",
            "composer-multiline",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(13, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            &filter_subject,
            reference(DOCK_TOP + 1, 4, 4 + header.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "attach-one-to-draft",
            reference(DOCK_TOP + 2, 4, 48),
            "summary",
            None,
        ),
    ];

    if matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 3,
            &format!("No project files match {query}."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "Keep typing or cancel to restore the original draft.",
            TextStyle::muted(),
        )?;
        dock_line(canvas, DOCK_TOP + 5, "Esc to cancel", TextStyle::bright())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            &format!("no-match-{query}"),
            reference(DOCK_TOP + 3, 4, 32 + query.chars().count() as u16),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.attachment-picker.cancel",
            reference(DOCK_TOP + 5, 4, 17),
            "summary",
            Some(action(
                "action.attachment-picker.cancel",
                "Cancel",
                false,
            )),
        ));
    } else {
        if let Some(start) = attach_start {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.attachment-picker.confirm-selected",
                reference(DOCK_TOP + 1, start, start + attach_label.chars().count() as u16),
                "focused",
                Some(action(
                    "action.attachment-picker.confirm-selected",
                    "Attach",
                    false,
                )),
            ));
        }
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.attachment-picker.cancel",
            reference(
                DOCK_TOP + 1,
                cancel_start,
                cancel_start + cancel_label.chars().count() as u16,
            ),
            "summary",
            Some(action(
                "action.attachment-picker.cancel",
                "Cancel",
                false,
            )),
        ));

        // Three dock body rows: keep at least one unsupported label visible in the
        // default catalogue so eligibility is understandable without color.
        let visible = attachment_visible_rows(&matches);
        for (index, candidate) in visible.iter().enumerate() {
            let row = DOCK_TOP + 3 + index as u16;
            let selected = first_eligible.is_some() && candidate.path == matches[first_eligible.unwrap()].path;
            let marker = if selected { "›" } else { " " };
            let style = if selected {
                TextStyle::accent()
            } else if candidate.eligible {
                TextStyle::text()
            } else {
                TextStyle::muted()
            };
            let line = format!("{marker} {} · {}", candidate.path, candidate.detail);
            dock_line(canvas, row, &line, style)?;
            let subject = if candidate.eligible {
                candidate.path.to_owned()
            } else {
                format!("{}·unsupported", candidate.path)
            };
            // Ineligible rows carry textual eligibility only; disabled actions are rejected by audit.
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "attachment-candidate",
                &subject,
                reference(row, 6, 6 + candidate.path.chars().count() as u16),
                "focused",
                None,
            ));
            if selected {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "selection",
                    candidate.path,
                    reference(row, 4, 6 + candidate.path.chars().count() as u16),
                    "focused",
                    None,
                ));
            }
            let eligibility_family = if candidate.eligible {
                "eligible-row"
            } else {
                "ineligible-row"
            };
            let detail_start = 6 + candidate.path.chars().count() as u16 + 3;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                eligibility_family,
                candidate.path,
                reference(
                    row,
                    detail_start,
                    detail_start + candidate.detail.chars().count() as u16,
                ),
                "summary",
                None,
            ));
        }
        for row in DOCK_TOP + 3 + visible.len() as u16..=DOCK_TOP + 5 {
            dock_line(canvas, row, "", TextStyle::text())?;
        }
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    let transitions = if first_eligible.is_some() {
        vec![
            transition(
                "action.attachment-picker.confirm-selected",
                "attachment-picker",
                "composer-multiline",
                "direct",
                "confirm inserts exactly one attachment token at the preserved cursor and keeps surrounding draft text byte-stable",
            ),
            transition(
                "action.attachment-picker.cancel",
                "attachment-picker",
                "composer-multiline",
                "cancel",
                "cancel adds nothing and restores the original draft byte for byte",
            ),
        ]
    } else {
        vec![transition(
            "action.attachment-picker.cancel",
            "attachment-picker",
            "composer-multiline",
            "cancel",
            "cancel adds nothing and restores the original draft byte for byte",
        )]
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

fn render_export_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    export_transcript(canvas)?;
    draft_preview(canvas, MENTION_FRAGMENT)?;
    write_dock_border(canvas)?;

    // Attachment/slash pattern: header names the action + consequence + local keys.
    // Four format rows fill the rest of the dock. Default uses a [default] badge so
    // monochrome TXT still distinguishes it; selection uses › without filling a bar.
    let selected_index = export_selected_index(fixture_id);
    let selected = &EXPORT_FORMATS[selected_index];
    let header =
        "Export conversation · Creates one sandboxed file from this conversation · Enter to export · Esc to cancel";
    debug_assert!(
        header.chars().count() <= 113,
        "export picker header overflowed the dock body"
    );
    dock_line(canvas, DOCK_TOP + 1, header, TextStyle::bright())?;

    let export_label = "Enter to export";
    let cancel_label = "Esc to cancel";
    let export_start = header
        .find(export_label)
        .map(|index| 4 + index as u16)
        .expect("export primary action must be present");
    let cancel_start = header
        .find(cancel_label)
        .map(|index| 4 + index as u16)
        .expect("export cancel action must be present");

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "paused-conversation",
            reference(4, 4, 74),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "composer-multiline",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(13, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "picker",
            "export-format-selection",
            reference(DOCK_TOP + 1, 4, 4 + header.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "sandbox-one-export-file",
            reference(DOCK_TOP + 1, 4, 72),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.export-picker.confirm-selected",
            reference(
                DOCK_TOP + 1,
                export_start,
                export_start + export_label.chars().count() as u16,
            ),
            "focused",
            Some(action(
                "action.export-picker.confirm-selected",
                "Export",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.export-picker.cancel",
            reference(
                DOCK_TOP + 1,
                cancel_start,
                cancel_start + cancel_label.chars().count() as u16,
            ),
            "summary",
            Some(action("action.export-picker.cancel", "Cancel", false)),
        ),
    ];

    for (index, format) in EXPORT_FORMATS.iter().enumerate() {
        let row = DOCK_TOP + 2 + index as u16;
        let selected_row = index == selected_index;
        let marker = if selected_row { "›" } else { " " };
        // Bracketed default badge is monochrome-distinct without color dependence.
        let default_badge = if format.is_default { " [default]" } else { "" };
        let line = format!(
            "{marker} {} · {} · {}{default_badge}",
            format.name, format.artifact, format.consequence
        );
        debug_assert!(
            line.chars().count() <= 113,
            "export format row overflowed the dock body"
        );
        dock_line(
            canvas,
            row,
            &line,
            if selected_row {
                TextStyle::accent()
            } else {
                TextStyle::text()
            },
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "export-format",
            format.name,
            reference(row, 6, 6 + format.name.chars().count() as u16),
            if selected_row { "focused" } else { "summary" },
            None,
        ));
        let detail_start = 6 + format.name.chars().count() as u16 + 3;
        let detail_len =
            format.artifact.chars().count() as u16 + 3 + format.consequence.chars().count() as u16;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "format-consequence",
            format.name,
            reference(row, detail_start, detail_start + detail_len),
            "summary",
            None,
        ));
        if selected_row {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "selection",
                format.name,
                reference(row, 4, 6 + format.name.chars().count() as u16),
                "focused",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "selected-consequence",
                &format!(
                    "{}-{}",
                    format.name.to_ascii_lowercase(),
                    format.artifact
                ),
                reference(row, detail_start, detail_start + detail_len),
                "focused",
                None,
            ));
        }
        if format.is_default {
            let badge_label = "[default]";
            let badge_start = line
                .find(badge_label)
                .map(|index| 4 + index as u16)
                .expect("default badge must appear on the default format row");
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "default",
                format.name,
                reference(
                    row,
                    badge_start,
                    badge_start + badge_label.chars().count() as u16,
                ),
                "summary",
                None,
            ));
        }
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "consequence",
        &format!("{}-export", selected.name.to_ascii_lowercase()),
        reference(DOCK_TOP + 2 + selected_index as u16, 6, 80),
        "focused",
        None,
    ));

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.export-picker.confirm-selected",
                "export-picker",
                "composer-multiline",
                "direct",
                "confirm creates exactly the selected sandboxed format, shows one inline success summary, and restores the origin draft and geometry",
            ),
            transition(
                "action.export-picker.cancel",
                "export-picker",
                "composer-multiline",
                "cancel",
                "cancel creates no artifact, shows no success claim, and restores the original draft and geometry",
            ),
        ],
    ))
}

fn export_selected_index(fixture_id: &str) -> usize {
    if fixture_id.contains("select-json") {
        1
    } else if fixture_id.contains("select-html") {
        2
    } else if fixture_id.contains("select-patch") {
        3
    } else {
        0
    }
}

fn export_transcript(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Choose an export format. Your draft stays exactly as you left it.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn render_queue_manager(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    queue_transcript(canvas, fixture_id)?;
    draft_preview(canvas, MENTION_FRAGMENT)?;
    write_dock_border(canvas)?;

    // Fixture branches surface conflict (refresh-only primary) vs default open.
    let conflict = fixture_id.contains("conflict") || fixture_id.contains("stale");
    let refreshed = fixture_id.contains("refresh") || fixture_id.contains("after-dequeue");
    let selected_identity =
        if refreshed || fixture_id.contains("select-item-2") {
            "queue-item-2"
        } else {
            "queue-item-1"
        };
    let items: Vec<QueueItem> = if refreshed {
        vec![QueueItem {
            order: 1,
            identity: "queue-item-2",
            revision: "rev=2",
            summary: "Then summarize risks.",
            unit_bits: "[attachment: queue-notes-b.md]",
            runs_next: true,
        }]
    } else {
        QUEUE_ITEMS.to_vec()
    };

    // Conflict keeps Apply as a labeled refused action and adds exactly one
    // Refresh queue primary so stale apply remains auditable against after-state.
    let header = if conflict {
        "Queued prompts · run after the current turn · Enter to refresh · Esc to cancel"
    } else {
        "Queued prompts · run after the current turn · Enter to apply · Esc to cancel"
    };
    debug_assert!(
        header.chars().count() <= 113,
        "queue manager header overflowed the dock body"
    );
    dock_line(canvas, DOCK_TOP + 1, header, TextStyle::bright())?;

    let primary_line = if conflict {
        "Stale queue edits refused · Refresh reloads identities and drops pending mutations"
    } else {
        "Apply shown reorders and removals against opened revisions · payloads move as one"
    };
    dock_line(canvas, DOCK_TOP + 2, primary_line, TextStyle::muted())?;

    // Header paints the compact protons "Enter to refresh" / "Enter to apply";
    // action labels stay longer prose for audit readability, but reference
    // geometry must bind the exact painted substring.
    let refresh_label = "Enter to refresh";
    let apply_label = "Enter to apply";
    let cancel_label = "Esc to cancel";
    let cancel_start = header
        .find(cancel_label)
        .map(|index| 4 + index as u16)
        .expect("queue cancel action must be present");

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "paused-conversation",
            reference(4, 4, 74),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "composer-multiline",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(13, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            QUEUE_FILTER,
            reference(DOCK_TOP + 1, 4, 18),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "picker",
            "queue-item-selection",
            reference(DOCK_TOP + 1, 4, 4 + header.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "queue-runs-after-current-turn",
            reference(DOCK_TOP + 1, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "apply-shown-edits-as-one-unit",
            reference(DOCK_TOP + 2, 4, 4 + primary_line.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.queue-manager.cancel",
            reference(
                DOCK_TOP + 1,
                cancel_start,
                cancel_start + cancel_label.chars().count() as u16,
            ),
            "summary",
            Some(action(
                "action.queue-manager.cancel",
                "Cancel",
                false,
            )),
        ),
    ];

    if conflict {
        let refresh_start = header
            .find(refresh_label)
            .map(|index| 4 + index as u16)
            .expect("refresh primary action must be present");
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.queue-manager.refresh",
            reference(
                DOCK_TOP + 1,
                refresh_start,
                refresh_start + refresh_label.chars().count() as u16,
            ),
            "focused",
            Some(action(
                "action.queue-manager.refresh",
                "Refresh queue",
                false,
            )),
        ));
        // Retain apply identity for the refused transition against after-state audit.
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "refused-action",
            "action.queue-manager.apply",
            reference(DOCK_TOP + 2, 4, 28),
            "summary",
            Some(action("action.queue-manager.apply", "Apply", false)),
        ));
    } else {
        let apply_start = header
            .find(apply_label)
            .map(|index| 4 + index as u16)
            .expect("apply primary action must be present");
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.queue-manager.apply",
            reference(
                DOCK_TOP + 1,
                apply_start,
                apply_start + apply_label.chars().count() as u16,
            ),
            "focused",
            Some(action("action.queue-manager.apply", "Apply", false)),
        ));
    }

    // Two body rows for visible items (or one after authoritative refresh).
    for (index, item) in items.iter().enumerate() {
        let row = DOCK_TOP + 3 + index as u16;
        let selected = item.identity == selected_identity;
        let marker = if selected { "›" } else { " " };
        let next_badge = if item.runs_next { " · [next]" } else { "" };
        let line = format!(
            "{marker} {} · {} {} · {} {}{next_badge}",
            item.order, item.identity, item.revision, item.summary, item.unit_bits
        );
        debug_assert!(
            line.chars().count() <= 113,
            "queue item row overflowed the dock body"
        );
        dock_line(
            canvas,
            row,
            &line,
            if selected {
                TextStyle::accent()
            } else {
                TextStyle::text()
            },
        )?;
        let subject = format!("{}·{}", item.identity, item.revision);
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "queue-item",
            &subject,
            reference(row, 6, 6 + item.identity.chars().count() as u16),
            if selected { "focused" } else { "summary" },
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "queue-order",
            &format!("order-{}", item.order),
            reference(row, 6, 8),
            "summary",
            None,
        ));
        if item.runs_next {
            let next_label = "[next]";
            let next_start = line
                .find(next_label)
                .map(|index| 4 + index as u16)
                .expect("next badge must appear on the next queue item");
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "queue-next",
                &format!("{}-runs-next", item.identity),
                reference(
                    row,
                    next_start,
                    next_start + next_label.chars().count() as u16,
                ),
                "focused",
                None,
            ));
        }
        if selected {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "selection",
                item.identity,
                reference(row, 4, 6 + item.identity.chars().count() as u16),
                "focused",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "revision",
                &format!("{}·{}", item.identity, item.revision),
                reference(row, 6, 30),
                "focused",
                None,
            ));
        }
    }
    for row in DOCK_TOP + 3 + items.len() as u16..=DOCK_TOP + 4 {
        dock_line(canvas, row, "", TextStyle::text())?;
    }

    // Local controls live on the selected row's action strip (never unlabeled).
    let controls = if conflict {
        "Enter to refresh queue · discards stale pending edits · never resurrects consumed items"
    } else {
        // Prose byline SYM needles must match the RenderRef bind list below 1:1.
        "r to reorder · x to remove · n to run next · selected payload moves as one unit"
    };
    debug_assert!(
        controls.chars().count() <= 113,
        "queue controls row overflowed the dock body"
    );
    dock_line(canvas, DOCK_TOP + 5, controls, TextStyle::bright())?;

    if !conflict {
        for (control_id, label, needle) in [
            (
                "action.queue-manager.reorder",
                "Reorder",
                "r to reorder",
            ),
            ("action.queue-manager.remove", "Remove", "x to remove"),
            (
                "action.queue-manager.run-next",
                "Run next",
                "n to run next",
            ),
        ] {
            let start = controls
                .find(needle)
                .map(|index| 4 + index as u16)
                .expect("queue control label must appear");
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "row-action",
                control_id,
                reference(DOCK_TOP + 5, start, start + needle.chars().count() as u16),
                "summary",
                Some(action(control_id, label, false)),
            ));
        }
    } else {
        // The single Refresh queue action is already the primary-action above.
        // This row is consequence-only copy so audit action identities stay unique.
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "conflict-guidance",
            "refresh-discards-stale",
            reference(DOCK_TOP + 5, 4, 4 + controls.chars().count() as u16),
            "summary",
            None,
        ));
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    let mut transitions = vec![
        transition(
            "action.queue-manager.cancel",
            "queue-manager",
            "composer-multiline",
            "cancel",
            "cancel restores the original queue order and originating draft exactly without submitting",
        ),
    ];
    if conflict {
        transitions.insert(
            0,
            transition(
                "action.queue-manager.refresh",
                "queue-manager",
                "queue-manager",
                "direct",
                "refresh discards stale pending mutations, reloads authoritative identities and revisions, and never resurrects consumed items",
            ),
        );
        transitions.insert(
            0,
            transition(
                "action.queue-manager.apply",
                "queue-manager",
                "queue-manager",
                "direct",
                "stale concurrent dequeue or revision conflict refuses apply and keeps one Refresh queue action",
            ),
        );
    } else {
        transitions.insert(
            0,
            transition(
                "action.queue-manager.apply",
                "queue-manager",
                "composer-multiline",
                "direct",
                "apply persists only the shown reorder/removal against opened revisions and returns without submitting the draft",
            ),
        );
        transitions.insert(
            1,
            transition(
                "action.queue-manager.reorder",
                "queue-manager",
                "queue-manager",
                "direct",
                "reorder moves the complete selected payload (text, newlines, mentions, attachments) as one unit",
            ),
        );
        transitions.insert(
            2,
            transition(
                "action.queue-manager.remove",
                "queue-manager",
                "queue-manager",
                "direct",
                "labeled remove drops the complete selected payload without unlabeled destructive keys",
            ),
        );
        transitions.insert(
            3,
            transition(
                "action.queue-manager.run-next",
                "queue-manager",
                "queue-manager",
                "direct",
                "run-next promotes the complete selected payload to the front of the queue",
            ),
        );
    }

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn queue_transcript(canvas: &mut Canvas, fixture_id: &str) -> Result<(), String> {
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    let message = if fixture_id.contains("conflict") || fixture_id.contains("stale") {
        "The queue changed while you were editing. Refresh before applying."
    } else if fixture_id.contains("refresh") || fixture_id.contains("after-dequeue") {
        "Authoritative queue reloaded. Consumed prompts stay removed."
    } else {
        "Manage queued prompts. Your composer draft stays exactly as you left it."
    };
    canvas.write(9, 4, message, TextStyle::text())?;
    Ok(())
}

fn render_command_palette(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    palette_transcript(canvas)?;
    let query = if fixture_id.contains("no-results") {
        "zzq"
    } else if fixture_id.contains("filter-queue") {
        "queue"
    } else {
        ""
    };
    let matches = palette_matches(query);
    write_dock_border(canvas)?;
    let primary_opens_submenu = matches.first().is_some_and(|row| row.opens_submenu);
    let enter_label = if matches.is_empty() {
        "Esc to cancel"
    } else if primary_opens_submenu {
        "Enter to open scope · Esc to cancel"
    } else {
        "Enter to run · Esc to cancel"
    };
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "Filter actions · query: {} · {} matching · {enter_label}",
            if query.is_empty() { "(all)" } else { query },
            matches.len()
        ),
        TextStyle::bright(),
    )?;

    let filter_subject_owned = if query == "queue" {
        "query-queue".to_owned()
    } else if query.is_empty() {
        "query-empty".to_owned()
    } else {
        format!("query-{query}")
    };

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
            "origin",
            "composer-multiline",
            reference(3, 4, 52),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "preserved-multiline-draft",
            reference(12, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            &filter_subject_owned,
            reference(DOCK_TOP + 1, 4, 70),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.command-palette.cancel",
            reference(DOCK_TOP + 1, 78, 92),
            "summary",
            Some(action("action.command-palette.cancel", "Cancel", false)),
        ),
    ];

    if matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 2,
            &format!("No actions match {query}."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "Keep the original draft. Type another filter or cancel.",
            TextStyle::muted(),
        )?;
        dock_line(canvas, DOCK_TOP + 4, "", TextStyle::text())?;
        dock_line(canvas, DOCK_TOP + 5, "", TextStyle::text())?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            &format!("no-match-{query}"),
            reference(DOCK_TOP + 2, 4, 28 + query.chars().count() as u16),
            "focused",
            None,
        ));
    } else {
        // Header primary keys always expose the selected-row consequence.
        if primary_opens_submenu {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.command-palette.open-submenu",
                reference(DOCK_TOP + 1, 52, 72),
                "focused",
                Some(action(
                    "action.command-palette.open-submenu",
                    "Open scope",
                    false,
                )),
            ));
        } else {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.command-palette.run-selected",
                reference(DOCK_TOP + 1, 52, 66),
                "focused",
                Some(action(
                    "action.command-palette.run-selected",
                    "Run action",
                    false,
                )),
            ));
        }
        for (index, action_row) in matches.iter().enumerate() {
            let row = DOCK_TOP + 2 + index as u16;
            if row > DOCK_TOP + 5 {
                break;
            }
            let selected = index == 0;
            let marker = if selected { "›" } else { " " };
            let disclosure = if action_row.opens_submenu { " ▸" } else { "" };
            dock_line(
                canvas,
                row,
                &format!(
                    "{marker} {} · {}{disclosure}",
                    action_row.label, action_row.consequence
                ),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "palette-action",
                action_row.subject_id,
                reference(row, 6, 6 + action_row.label.chars().count() as u16),
                "focused",
                None,
            ));
            if selected {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "selection",
                    action_row.subject_id,
                    reference(row, 4, 6 + action_row.label.chars().count() as u16),
                    "focused",
                    None,
                ));
            }
            if action_row.opens_submenu {
                let end = 6
                    + action_row.label.chars().count()
                    + 3
                    + action_row.consequence.chars().count()
                    + 2;
                // Affordance glyph is color-independent evidence for deeper scope.
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "submenu-affordance",
                    action_row.subject_id,
                    reference(row, end as u16 - 1, end as u16 + 1),
                    "summary",
                    None,
                ));
                // Always publish the open-submenu action on rows that can open a scope
                // so root transitions stay bijective even when another row is selected.
                if !selected || !primary_opens_submenu {
                    pairs.push(semantic(
                        screen_id,
                        fixture_id,
                        "secondary-action",
                        "action.command-palette.open-submenu",
                        reference(row, end as u16 - 1, end as u16 + 1),
                        "summary",
                        Some(action(
                            "action.command-palette.open-submenu",
                            "Open scope",
                            false,
                        )),
                    ));
                }
            }
        }
        for row in DOCK_TOP + 2 + matches.len().min(4) as u16..=DOCK_TOP + 5 {
            dock_line(canvas, row, "", TextStyle::text())?;
        }
    }

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    let mut transitions = vec![transition(
        "action.command-palette.cancel",
        "command-palette",
        "composer-multiline",
        "cancel",
        "cancel returns to the originating composer with the exact original multiline draft and transcript position",
    )];
    if matches.iter().any(|row| row.opens_submenu) {
        transitions.insert(
            0,
            transition(
                "action.command-palette.open-submenu",
                "command-palette",
                "command-palette-submenu",
                "direct",
                "opening a root row with a deeper scope enters the named submenu without mutating the draft",
            ),
        );
    }
    if matches.iter().any(|row| !row.opens_submenu) {
        transitions.insert(
            0,
            transition(
                "action.command-palette.run-selected",
                "command-palette",
                "composer-multiline",
                "direct",
                "running a root action applies that consequence once and restores the origin composer",
            ),
        );
    }
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_command_palette_submenu(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let _ = fixture_id;
    palette_transcript(canvas)?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Palette › Queued prompts · scoped list · Esc to root",
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
            "origin",
            "composer-multiline",
            reference(3, 4, 52),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "scope",
            "queued-prompts",
            reference(DOCK_TOP + 1, 4, 58),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "parent-scope",
            "command-palette",
            reference(DOCK_TOP + 1, 4, 11),
            "summary",
            None,
        ),
    ];

    for (index, row_item) in QUEUED_SUBMENU_ROWS.iter().enumerate() {
        let row = DOCK_TOP + 2 + index as u16;
        let selected = index == 0;
        let marker = if selected { "›" } else { " " };
        dock_line(
            canvas,
            row,
            &format!("{marker} {} · {}", row_item.label, row_item.consequence),
            if selected {
                TextStyle::accent()
            } else {
                TextStyle::text()
            },
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "submenu-row",
            row_item.subject_id,
            reference(row, 6, 6 + row_item.label.chars().count() as u16),
            "focused",
            None,
        ));
        if selected {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "selection",
                row_item.subject_id,
                reference(row, 4, 6 + row_item.label.chars().count() as u16),
                "focused",
                None,
            ));
        }
    }
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "› Root · Enter to run scoped action · Esc to cancel",
        TextStyle::bright(),
    )?;
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.command-palette-submenu.back",
        reference(DOCK_TOP + 5, 4, 16),
        "focused",
        Some(action(
            "action.command-palette-submenu.back",
            "Back to root",
            false,
        )),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "secondary-action",
        "action.command-palette-submenu.run-selected",
        reference(DOCK_TOP + 5, 18, 46),
        "summary",
        Some(action(
            "action.command-palette-submenu.run-selected",
            "Run scoped action",
            false,
        )),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "cancel-action",
        "action.command-palette-submenu.cancel",
        reference(DOCK_TOP + 5, 48, 80),
        "summary",
        Some(action(
            "action.command-palette-submenu.cancel",
            "Cancel",
            false,
        )),
    ));
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    ));

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.command-palette-submenu.back",
                "command-palette-submenu",
                "command-palette",
                "cancel",
                "back restores the root palette with the prior root query, selected row, and list position",
            ),
            transition(
                "action.command-palette-submenu.run-selected",
                "command-palette-submenu",
                "composer-multiline",
                "direct",
                "running a scoped action returns to the origin composer with the unchanged draft",
            ),
            transition(
                "action.command-palette-submenu.cancel",
                "command-palette-submenu",
                "composer-multiline",
                "cancel",
                "cancel from the submenu returns directly to the origin composer with the exact original multiline draft",
            ),
        ],
    ))
}

fn slash_draft_preview(canvas: &mut Canvas, query: &str) -> Result<(), String> {
    canvas.write(2, 4, subject_title_for_origin(FLOW2_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, FLOW2_TASK, TextStyle::bright())?;
    canvas.write(12, 4, "Paused draft · 3 authored lines", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        &format!("› {query} · active command span"),
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "  Check the retry path before we merge.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "  Then explain the failure path that still needs coverage before the merge.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn palette_transcript(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(2, 4, subject_title_for_origin(FLOW2_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, FLOW2_TASK, TextStyle::bright())?;
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The conversation stays visible while you choose an action.",
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        4,
        "Preserved draft · unchanged while the palette is open",
        TextStyle::muted(),
    )?;
    canvas.write(
        13,
        4,
        "› Check the retry path in @retry",
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
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn slash_matches(fragment: &str) -> Vec<SlashCommand> {
    let needle = fragment.to_ascii_lowercase();
    SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|command| {
            if needle.is_empty() {
                true
            } else {
                command.name.trim_start_matches('/').to_ascii_lowercase().contains(&needle)
                    || command.consequence.to_ascii_lowercase().contains(&needle)
            }
        })
        .collect()
}

fn palette_matches(query: &str) -> Vec<PaletteAction> {
    let needle = query.to_ascii_lowercase();
    PALETTE_ROOT_ACTIONS
        .iter()
        .copied()
        .filter(|action| {
            if needle.is_empty() {
                true
            } else {
                action.label.to_ascii_lowercase().contains(&needle)
                    || action.consequence.to_ascii_lowercase().contains(&needle)
            }
        })
        .collect()
}

fn attachment_matches(query: &str) -> Vec<AttachmentCandidate> {
    let needle = query.to_ascii_lowercase();
    ATTACHMENT_CANDIDATES
        .iter()
        .copied()
        .filter(|candidate| {
            if needle.is_empty() {
                true
            } else {
                candidate.path.to_ascii_lowercase().contains(&needle)
                    || candidate.detail.to_ascii_lowercase().contains(&needle)
            }
        })
        .collect()
}

fn attachment_visible_rows(matches: &[AttachmentCandidate]) -> Vec<AttachmentCandidate> {
    if matches.len() <= 3 {
        return matches.to_vec();
    }
    let eligible = matches
        .iter()
        .copied()
        .filter(|candidate| candidate.eligible)
        .take(2)
        .collect::<Vec<_>>();
    let mut visible = eligible;
    if let Some(ineligible) = matches.iter().copied().find(|candidate| !candidate.eligible) {
        visible.push(ineligible);
    }
    while visible.len() < 3 {
        if let Some(next) = matches
            .iter()
            .copied()
            .find(|candidate| !visible.iter().any(|seen| seen.path == candidate.path))
        {
            visible.push(next);
        } else {
            break;
        }
    }
    visible
}

fn transcript(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
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
