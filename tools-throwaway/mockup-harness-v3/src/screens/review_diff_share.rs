//! Flow 1 review, read-only diff evidence, and share result states.
//!
//! - `review` and `diff-viewer` are temporary detail views: dense evidence with
//!   one clear current selection and an explicit return to the origin chain.
//! - Inspection never mutates files, staging, approvals, tasks, or transcript.
//! - `share` is an action dock: states exact local/remote and read-only scope
//!   before the link, then local copy/revoke/return actions with idempotent
//!   exact-target revocation that preserves unrelated shares.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{subject_title_for_origin, DiffBlock, DiffKind, DetailView},
        layout::{
            ACTION_DOCK_BOTTOM, ACTION_DOCK_TOP, CANVAS_ROWS, ScreenComposition, StatusLine,
            SurfaceLayout,
        },
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = ACTION_DOCK_TOP;
const DOCK_BOTTOM: u16 = ACTION_DOCK_BOTTOM;

const DAILY_ORIGIN: &str = "Daily work · current conversation";
const DAILY_TASK: &str =
    "Find where the server applies the rate limit and explain the safe path.";
const DRAFT: &str = "Also list the tests that should cover the search.";
const TURN_ID: &str = "turn.daily-work.review.18";
const CONVERSATION_ID: &str = "conversation.daily-work";
const TRANSCRIPT_ID: &str = "transcript.daily-work";
const SESSION_ID: &str = "session.daily-work.1";
const PAUSED_ACTIVITY: &str = "activity.rate-limit-path";

const SELECTED_FINDING_ID: &str = "finding.middleware-cap";
const SELECTED_FINDING_LABEL: &str =
    "high · middleware.rs:42 · unbounded wait when cap is zero";
const FINDING_MEDIUM: &str =
    "medium · retry_policy.rs:88 · test still asserts the old wait value";
const FINDING_LOW: &str = "low · login.rs:12 · message could name the shared guard";

const FILE_ONE: &str = "src/auth/middleware.rs";
const FILE_TWO: &str = "src/net/retry_policy.rs";
const FILE_ONE_LINE: u32 = 42;
const FILE_TWO_LINE: u32 = 88;

const SHARE_PRIMARY: &str = "share.local.rate-limit-review";
const SHARE_UNRELATED: &str = "share.remote.docs-preview";
const SHARE_LINK: &str = "http://127.0.0.1:3200/share/shr-71b2ce";
const SHARE_SCOPE: &str = "rate-limit path conversation · read-only transcript";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let surface = match screen_id {
        "review" | "diff-viewer" => Surface::TemporaryDetailView,
        "share" => Surface::ActionDock,
        _ => return Err(format!("unknown review/diff/share screen: {screen_id}")),
    };
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    SurfaceLayout::for_surface(surface).validate()?;

    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "review" => render_review(&mut canvas, screen_id, fixture_id)?,
        "diff-viewer" => render_diff_viewer(&mut canvas, screen_id, fixture_id)?,
        "share" => render_share(&mut canvas, screen_id, fixture_id)?,
        _ => unreachable!(),
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
        conversation: Some(ConversationIdentity {
            conversation_id: CONVERSATION_ID.to_owned(),
            transcript_id: TRANSCRIPT_ID.to_owned(),
            session_id: SESSION_ID.to_owned(),
            turn_id: TURN_ID.to_owned(),
            user_task: DAILY_TASK.to_owned(),
            visible_origin: DAILY_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: Some(PAUSED_ACTIVITY.to_owned()),
            later_draft: Some(DRAFT.to_owned()),
        }),
    })
}

fn render_review(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let selected = selected_finding_for_fixture(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(DAILY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, DAILY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, "Code review · temporary detail", TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "From the rate-limit path conversation · return restores that turn",
        TextStyle::muted(),
    )?;
    canvas.write(
        8,
        4,
        "Scope · working tree vs main · 2 files · 3 findings",
        TextStyle::text(),
    )?;
    canvas.write(
        9,
        4,
        "Verdict · Changes requested · 1 high · 1 medium · 1 low",
        TextStyle::accent(),
    )?;

    let rows = [
        (12u16, SELECTED_FINDING_ID, SELECTED_FINDING_LABEL),
        (14, "finding.retry-test", FINDING_MEDIUM),
        (16, "finding.login-message", FINDING_LOW),
    ];
    for (row, id, label) in rows {
        let marker = if id == selected { "› " } else { "  " };
        let style = if id == selected {
            TextStyle::bright()
        } else {
            TextStyle::text()
        };
        canvas.write(row, 4, &format!("{marker}● {label}"), style)?;
    }
    canvas.write(
        13,
        8,
        "Selected · open to see the exact file and line evidence",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Read-only inspection · files, staging, approvals, tasks, and transcript stay unchanged",
        TextStyle::muted(),
    )?;
    canvas.write(22, 4, "› Open selected evidence", TextStyle::accent())?;
    canvas.write(
        23,
        4,
        "↑↓ to choose finding · Esc to return to conversation",
        TextStyle::muted(),
    )?;

    let pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "detail-origin",
            "conversation.daily-work",
            reference(6, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-scope",
            "working-tree-vs-main",
            reference(8, 4, 58),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-verdict",
            "changes-requested",
            reference(9, 4, 58),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-finding",
            SELECTED_FINDING_ID,
            reference(12, 4, 84),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-finding",
            "finding.retry-test",
            reference(14, 4, 84),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-finding",
            "finding.login-message",
            reference(16, 4, 84),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-finding",
            selected,
            reference(12, 4, 84),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "inspection-mode",
            "read-only",
            reference(19, 4, 90),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.review.open-evidence",
            reference(22, 4, 40),
            "focused",
            Some(action(
                "action.review.open-evidence",
                "Open selected evidence",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.review.return",
            reference(23, 4, 56),
            "summary",
            Some(action(
                "action.review.return",
                "Return to conversation",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.review.back",
            reference(23, 4, 56),
            "summary",
            Some(action(
                "action.review.back",
                "Back",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.review.cancel",
            reference(23, 4, 56),
            "summary",
            Some(action(
                "action.review.cancel",
                "Cancel",
                false,
            )),
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

    // DetailView anatomy helper used only for structure checks; paint stays explicit.
    let _detail = DetailView::new(
        "Code review",
        DAILY_ORIGIN,
        vec![
            "Scope · working tree vs main · 2 files · 3 findings",
            "Verdict · Changes requested",
        ],
    );
    crate::chrome::components::validate_component(&_detail)?;

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.review.open-evidence",
                "review",
                "diff-viewer",
                "direct",
                "selected finding opens its file and line evidence while remaining the return target",
            ),
            transition(
                "action.review.return",
                "review",
                "chat-turn",
                "direct",
                "return restores the exact originating conversation, draft, and transcript position",
            ),
            transition(
                "action.review.back",
                "review",
                "chat-turn",
                "cancel",
                "back restores the exact originating conversation without a navigation trap",
            ),
            transition(
                "action.review.cancel",
                "review",
                "chat-turn",
                "cancel",
                "cancel restores the exact originating conversation, draft, and scroll",
            ),
        ],
    ))
}

fn render_diff_viewer(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let file_index = diff_file_index(fixture_id);
    let (path, line, hunk_header, removed, added, position_label, next_label, prev_label) =
        if file_index == 1 {
            (
                FILE_ONE,
                FILE_ONE_LINE,
                "@@ -42,2 +42,3 @@ apply_limit",
                "    return wait_forever()",
                "    return wait_capped(cap)",
                "File 1 of 2",
                "next: src/net/retry_policy.rs (file 2 of 2)",
                "previous stays on file 1 of 2",
            )
        } else {
            (
                FILE_TWO,
                FILE_TWO_LINE,
                "@@ -88,2 +89,2 @@ retry wait assertion",
                "assert_eq!(wait, 4)",
                "assert_eq!(wait, 5)",
                "File 2 of 2",
                "next stays on file 2 of 2",
                "previous: src/auth/middleware.rs (file 1 of 2)",
            )
        };

    canvas.write(2, 4, subject_title_for_origin(DAILY_ORIGIN), TextStyle::muted())?;
    canvas.write(
        3,
        4,
        "Diff · temporary detail · from the selected review finding",
        TextStyle::bright(),
    )?;
    canvas.write(
        4,
        4,
        &format!("Selected finding · {SELECTED_FINDING_LABEL}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        6,
        4,
        &format!("{position_label} · {path} · line {line}"),
        TextStyle::bright(),
    )?;
    canvas.write(
        7,
        4,
        "Same review origin · return restores the selected finding",
        TextStyle::muted(),
    )?;
    canvas.write(9, 4, hunk_header, TextStyle::text())?;

    // Explicit text/glyph meaning for addition and deletion (not color-only).
    canvas.write(
        11,
        6,
        &format!("{line:>3}  {line:>3}  -{removed}"),
        TextStyle::text(),
    )?;
    canvas.write(12, 6, "        - removed line", TextStyle::muted())?;
    canvas.write(
        13,
        6,
        &format!("     {line:>3}  +{added}"),
        TextStyle::text(),
    )?;
    if file_index == 1 {
        canvas.write(
            14,
            6,
            "         +    log_limit_hit(cap)",
            TextStyle::text(),
        )?;
        canvas.write(15, 6, "        + added line", TextStyle::muted())?;
        canvas.write(16, 6, "  43  45      continue", TextStyle::text())?;
    } else {
        canvas.write(
            14,
            6,
            "     89  +assert_eq!(attempts, 1)",
            TextStyle::text(),
        )?;
        canvas.write(15, 6, "        + added line", TextStyle::muted())?;
        canvas.write(16, 6, "  90  91      Ok(())", TextStyle::text())?;
    }

    let summary = if file_index == 1 {
        "Summary · +2 -1 · 1 hunk · next: src/net/retry_policy.rs (file 2 of 2)"
    } else {
        "Summary · +1 -1 · 1 hunk · previous: src/auth/middleware.rs (file 1 of 2)"
    };
    canvas.write(19, 4, summary, TextStyle::accent())?;
    canvas.write(
        21,
        4,
        "Read-only inspection · no write, staging, or approval action here",
        TextStyle::muted(),
    )?;
    canvas.write(
        22,
        4,
        "Any approve or apply would open the Flow 2 decision dock first",
        TextStyle::muted(),
    )?;
    canvas.write(
        25,
        4,
        &format!("n for Next file · {next_label}"),
        TextStyle::text(),
    )?;
    canvas.write(
        26,
        4,
        &format!("p for Previous file · {prev_label}"),
        TextStyle::text(),
    )?;
    canvas.write(
        28,
        4,
        "› Esc to return to selected finding",
        TextStyle::accent(),
    )?;

    let diff = DiffBlock::new(
        path,
        vec![
            (DiffKind::Removed, removed),
            (DiffKind::Added, added),
            (DiffKind::Unchanged, "continue"),
        ],
    );
    crate::chrome::components::validate_component(&diff)?;

    let pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "detail-origin",
            "review-selected-finding",
            reference(4, 4, 76),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-finding",
            SELECTED_FINDING_ID,
            reference(4, 4, 76),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-file",
            path,
            reference(6, 4, 70),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-position",
            if file_index == 1 {
                "file-1-of-2"
            } else {
                "file-2-of-2"
            },
            reference(6, 4, 20),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-hunk",
            if file_index == 1 {
                "hunk.middleware-cap"
            } else {
                "hunk.retry-test"
            },
            reference(9, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-removed",
            "removed-line",
            reference(11, 6, 60),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-added",
            "added-line",
            reference(13, 6, 60),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "diff-summary",
            if file_index == 1 {
                "plus-2-minus-1"
            } else {
                "plus-1-minus-1"
            },
            reference(19, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "inspection-mode",
            "read-only",
            reference(21, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "local-control",
            "action.diff-viewer.next-file",
            reference(25, 4, 56),
            "summary",
            Some(action(
                "action.diff-viewer.next-file",
                "Next file",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "local-control",
            "action.diff-viewer.previous-file",
            reference(26, 4, 62),
            "summary",
            Some(action(
                "action.diff-viewer.previous-file",
                "Previous file",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.diff-viewer.return",
            reference(28, 4, 42),
            "focused",
            Some(action(
                "action.diff-viewer.return",
                "Return to selected finding",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.diff-viewer.back",
            reference(28, 4, 42),
            "summary",
            Some(action(
                "action.diff-viewer.back",
                "Back",
                false,
            )),
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

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.diff-viewer.next-file",
                "diff-viewer",
                "diff-viewer",
                "direct",
                "next updates to the next file in the same review change set without leaving the selection",
            ),
            transition(
                "action.diff-viewer.previous-file",
                "diff-viewer",
                "diff-viewer",
                "direct",
                "previous stays inside the change set and keeps the same selected review finding",
            ),
            transition(
                "action.diff-viewer.return",
                "diff-viewer",
                "review",
                "direct",
                "return restores the same selected review finding before conversation return is available",
            ),
            transition(
                "action.diff-viewer.back",
                "diff-viewer",
                "review",
                "cancel",
                "back restores the same selected review finding with original draft and scroll preserved",
            ),
        ],
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShareVariant {
    Ready,
    RevokeConfirm,
    Revoked,
}

fn render_share(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = share_variant(fixture_id);

    // Keep the originating conversation transcript visible above the dock.
    canvas.write(2, 4, subject_title_for_origin(DAILY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, DAILY_TASK, TextStyle::bright())?;
    canvas.write(
        5,
        4,
        "You · Find where the server applies the rate limit and explain the safe path.",
        TextStyle::bright(),
    )?;
    canvas.write(8, 4, "Code", TextStyle::bright())?;
    canvas.write(
        10,
        4,
        "The shared request guard applies the limit before route work begins.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "→ src/auth/middleware.rs owns the rate-limit decision.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "● Ready to continue · share result attaches below in the action dock",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;

    match variant {
        ShareVariant::Ready => {
            dock_line(canvas, DOCK_TOP + 1, "✓ Local share reference created", TextStyle::bright())?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                &format!("Scope · {SHARE_SCOPE}"),
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "Access · local only · not remote · result is read-only",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                &format!("Link · {SHARE_LINK}"),
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "› Copy link · r for Revoke this local share · Esc to return",
                TextStyle::accent(),
            )?;
        }
        ShareVariant::RevokeConfirm => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "Revoke this local share?",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                &format!("Target · {SHARE_PRIMARY} · {SHARE_SCOPE}"),
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                &format!("Unrelated active · {SHARE_UNRELATED} stays available"),
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                "Only this exact local share is removed · repeated confirm is safe",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "› Confirm revoke · Esc to keep share and return",
                TextStyle::accent(),
            )?;
        }
        ShareVariant::Revoked => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "✓ Local share already revoked",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                &format!("Target · {SHARE_PRIMARY} · removed (idempotent)"),
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                &format!("Still active · {SHARE_UNRELATED} · remote read-only docs preview"),
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                "Repeating confirm changes nothing · origin draft is preserved",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "› Esc to return to conversation",
                TextStyle::accent(),
            )?;
        }
    }

    // Documents the bottom edge for printers; keeping constant referenced.
    let _ = DOCK_BOTTOM;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "conversation.daily-work",
            reference(2, 4, 40),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "active-turn",
            reference(10, 4, 70),
            "summary",
            None,
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

    match variant {
        ShareVariant::Ready => {
            pairs.extend([
                semantic(
                    screen_id,
                    fixture_id,
                    "share-result",
                    "local-share-created",
                    reference(DOCK_TOP + 1, 4, 40),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-scope",
                    "read-only-transcript",
                    reference(DOCK_TOP + 2, 4, 70),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-access",
                    "local-only",
                    reference(DOCK_TOP + 3, 4, 60),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-link",
                    SHARE_PRIMARY,
                    reference(DOCK_TOP + 4, 4, 70),
                    "summary",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "primary-action",
                    "action.share.copy-link",
                    reference(DOCK_TOP + 5, 4, 30),
                    "focused",
                    Some(action("action.share.copy-link", "Copy link", false)),
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "secondary-action",
                    "action.share.revoke",
                    reference(DOCK_TOP + 5, 30, 62),
                    "summary",
                    Some(action(
                        "action.share.revoke",
                        "Revoke this local share",
                        false,
                    )),
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "cancel-action",
                    "action.share.return",
                    reference(DOCK_TOP + 5, 62, 90),
                    "summary",
                    Some(action(
                        "action.share.return",
                        "Return to conversation",
                        false,
                    )),
                ),
            ]);
        }
        ShareVariant::RevokeConfirm => {
            pairs.extend([
                semantic(
                    screen_id,
                    fixture_id,
                    "share-target",
                    SHARE_PRIMARY,
                    reference(DOCK_TOP + 2, 4, 80),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-unrelated",
                    SHARE_UNRELATED,
                    reference(DOCK_TOP + 3, 4, 70),
                    "summary",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "primary-action",
                    "action.share.confirm-revoke",
                    reference(DOCK_TOP + 5, 4, 36),
                    "focused",
                    Some(action(
                        "action.share.confirm-revoke",
                        "Confirm revoke",
                        false,
                    )),
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "cancel-action",
                    "action.share.cancel-revoke",
                    reference(DOCK_TOP + 5, 38, 72),
                    "summary",
                    Some(action(
                        "action.share.cancel-revoke",
                        "Keep share and return",
                        false,
                    )),
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "cancel-action",
                    "action.share.return",
                    reference(DOCK_TOP + 5, 38, 72),
                    "summary",
                    Some(action(
                        "action.share.return",
                        "Return to conversation",
                        false,
                    )),
                ),
            ]);
        }
        ShareVariant::Revoked => {
            pairs.extend([
                semantic(
                    screen_id,
                    fixture_id,
                    "share-result",
                    "local-share-revoked",
                    reference(DOCK_TOP + 1, 4, 40),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-target",
                    SHARE_PRIMARY,
                    reference(DOCK_TOP + 2, 4, 70),
                    "focused",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "share-unrelated",
                    SHARE_UNRELATED,
                    reference(DOCK_TOP + 3, 4, 70),
                    "summary",
                    None,
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "primary-action",
                    "action.share.return",
                    reference(DOCK_TOP + 5, 4, 36),
                    "focused",
                    Some(action(
                        "action.share.return",
                        "Return to conversation",
                        false,
                    )),
                ),
                semantic(
                    screen_id,
                    fixture_id,
                    "secondary-action",
                    "action.share.confirm-revoke",
                    reference(DOCK_TOP + 5, 4, 36),
                    "summary",
                    Some(action(
                        "action.share.confirm-revoke",
                        "Confirm revoke",
                        false,
                    )),
                ),
            ]);
        }
    }

    let transitions = match variant {
        ShareVariant::Ready => vec![
            transition(
                "action.share.copy-link",
                "share",
                "share",
                "direct",
                "copy places the local read-only link without mutating conversation state",
            ),
            transition(
                "action.share.revoke",
                "share",
                "share",
                "direct",
                "revoke opens confirmation that names only this exact local share",
            ),
            transition(
                "action.share.return",
                "share",
                "chat-turn",
                "direct",
                "return restores the originating conversation and kept draft",
            ),
        ],
        ShareVariant::RevokeConfirm => vec![
            transition(
                "action.share.confirm-revoke",
                "share",
                "share",
                "direct",
                "confirm removes only the named share and leaves unrelated shares active",
            ),
            transition(
                "action.share.cancel-revoke",
                "share",
                "share",
                "cancel",
                "cancel keeps the share active and stays on the share result without a dead end",
            ),
            transition(
                "action.share.return",
                "share",
                "chat-turn",
                "direct",
                "return restores the originating conversation and kept draft",
            ),
        ],
        ShareVariant::Revoked => vec![
            transition(
                "action.share.confirm-revoke",
                "share",
                "share",
                "direct",
                "repeated confirm is idempotent and removes nothing additional",
            ),
            transition(
                "action.share.return",
                "share",
                "chat-turn",
                "direct",
                "return restores the originating conversation and kept draft",
            ),
        ],
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

fn selected_finding_for_fixture(fixture_id: &str) -> &'static str {
    if fixture_id.contains("retry-test") {
        "finding.retry-test"
    } else if fixture_id.contains("login-message") {
        "finding.login-message"
    } else {
        SELECTED_FINDING_ID
    }
}

fn diff_file_index(fixture_id: &str) -> u8 {
    if fixture_id.contains("file-2") || fixture_id.contains("retry_policy") {
        2
    } else {
        1
    }
}

fn share_variant(fixture_id: &str) -> ShareVariant {
    if fixture_id.contains("revoke-confirm") || fixture_id.contains("confirm-revoke") {
        ShareVariant::RevokeConfirm
    } else if fixture_id.contains("revoked") || fixture_id.contains("after-revoke") {
        ShareVariant::Revoked
    } else {
        ShareVariant::Ready
    }
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
