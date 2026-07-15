//! Flow 5 pilot states for turn, tool, blocked-command, network, rate-limit,
//! context exhaustion, interrupt, toasts, and checkpoint recovery.
//!
//! Turn failures keep partial replies inline with one adjacent primary recovery.
//! Tool failures keep the exact invocation, completed siblings, partial output,
//! one plain cause, and a cause-specific next action with safe retry boundaries.
//! Blocked commands keep the command and draft offline, explain one policy cause,
//! and resume only through approval or work-style with an explicit run decision.
//! Network failures preserve work, hide attempts behind evidence, distinguish
//! automatic bounded backoff from manual retry, and route connection inspection
//! without losing recovery origin or submitting more than once.
//! Context exhaustion preserves the refactor state, keeps token detail under
//! Evidence, and offers Compact and continue as the only primary recovery.
//! Interrupt is neutral (not a failure): retained partial, visible seam, Continue reply.
//! Toasts stay compact, severity-ordered, notice-owned, and nonblocking.
//! Rate-limit owns one automatic countdown with optional secondary Retry now.
//! Checkpoint undo names exact file effects, preserves unrelated work, and
//! offers symmetric redo with conflict and no-op safety.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::subject_title_for_origin,
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = 31;
const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str = "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const COMPLETED_WORK: &str = "Completed earlier · retry window copy accepted for the release branch.";
const DRAFT: &str = "Keep the risk note short before we publish.";
const TURN_CAUSE: &str = "The reply stopped before it finished";
const PARTIAL_KEPT: &str = "The partial reply was kept";
const NETWORK_CAUSE_GENERIC: &str = "Cortex could not reach the provider";
const NETWORK_CAUSE_ENDED: &str = "The connection ended before the reply finished";
const NETWORK_CAUSE_OFFLINE: &str = "Network access is off";
const NETWORK_SAVED: &str = "Your work and partial output are saved.";
const TIMER_OWNER: &str = "timer.network.backoff";
const RATE_TIMER_OWNER: &str = "timer.rate-limit.wait";
const PENDING_NETWORK: &str = "pending.network.release-notes";
const CHECKPOINT_NAME: &str = "Before risk-note expansion";
const AFFECTED_A: &str = "notes/release.md";
const AFFECTED_B: &str = "notes/risks.md";
const UNAFFECTED: &str = "drafts/later-note.md";
const STALE_APPROVAL: &str = "Approval for the risks note · Stale · needs re-review";
const TOOL_USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const TOOL_COMPLETED: &str = "Read session rules";
const TOOL_FAILED_NAME: &str = "Write release notes patch";
const TOOL_INTENDED_EFFECT: &str = "Update notes/release.md with the accepted retry wording";
const TOOL_PARTIAL_OUTPUT: &str = "Prepared the first two bullet lines";
const TOOL_CAUSE_MISSING_FILE: &str = "The repo map file is missing";
const TOOL_CAUSE_CAPABILITY: &str = "Docs search is unavailable until you connect it";
const TOOL_CAUSE_UNKNOWN: &str = "The outcome of this tool is unknown";
const TOOL_CAUSE_COMPLETED: &str = "This tool already changed the file before it failed";
const TOOL_CAUSE_POLICY: &str = "Docs search is blocked by the project policy";
const TOOL_CAUSE_IDEMPOTENT: &str = "The lookup failed before any change was made";
pub(crate) const BLOCKED_COMMAND: &str =
    "rm -rf notes/archive && printf 'cleared\\n' >> notes/release.md";
const BLOCKED_POLICY_CAUSE: &str = "The current work style does not allow write commands";
const BLOCKED_PERMISSION_CAUSE: &str = "This command needs approval before it can run";
const BLOCKED_COMPLETED_WORK: &str =
    "Completed earlier · retry window copy accepted for the release branch.";
const CONTEXT_CAUSE: &str = "This conversation is too large to continue as-is";
const CONTEXT_SAVED: &str = "Your transcript, draft, and completed edits are saved.";
const CONTEXT_COMPLETED_EDITS: &str = "Completed edits · notes/release.md · notes/risks.md";
const CONTEXT_PENDING_TASK: &str = "Pending · finish the short risk note before publish";
const CONTEXT_CONSEQUENCE: &str =
    "A summary replaces older visible context. Full history stays preserved.";
const CONTEXT_PRIMARY: &str = "Compact and continue";
const CONTEXT_SECONDARY: &str = "Start new session";
const CONTEXT_PENDING: &str = "pending.context.release-notes";
const INTERRUPT_STATEMENT: &str = "You stopped this reply";
const INTERRUPT_KEPT: &str = "Partial answer kept · saved in this conversation";
const INTERRUPT_PRIMARY: &str = "Continue reply";
const INTERRUPT_SECONDARY: &str = "Rephrase request";
const INTERRUPT_QUEUE_LINE: &str =
    "Queued follow-ups and attachments stay queued in their original order.";
const INTERRUPT_CONTINUE_NOTE: &str =
    "Continue runs first, then queued follow-ups keep their order.";
const INTERRUPT_REPHRASE_NOTE: &str =
    "Rephrase restores the original prompt in the composer for editing.";
const INTERRUPT_PENDING: &str = "pending.interrupt.release-notes";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;
const TOAST_FAILURE: &str = "Settings were not saved";
const TOAST_WARN: &str = "Connection recovered after a brief pause";
// Note: rendered toast lines use a shorter visual form; the full phrase is
// split across two compact rows so the canvas stays within 120 columns.
const TOAST_INFO: &str = "Model switched";
const TOAST_SUCCESS: &str = "Checkpoint saved";
const TOAST_RETRY: &str = "Retry save";
const TOAST_TRANSCRIPT: &str = "Your preferences stay in this project after they are saved.";
const TOAST_ORIGIN_LINE: &str = "Recovery · paused conversation";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NetworkVariant {
    Collapsed,
    Details,
    Evidence,
    Offline,
    Backoff,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContextVariant {
    Collapsed,
    Evidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RateLimitVariant {
    WaitingEarly,
    WaitingLater,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointVariant {
    Success,
    Conflict,
    Noop,
    StaleRedo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolFailedVariant {
    MissingFile,
    Capability,
    Unknown,
    CompletedEffect,
    Idempotent,
    Policy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockedCommandVariant {
    WorkStyle,
    Permission,
    AfterApproval,
    AfterWorkStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastVariant {
    Full,
    AfterAutoDismiss,
    AfterRetry,
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "turn-error"
        | "error-network"
        | "error-context"
        | "rate-limit"
        | "interrupt"
        | "checkpoint-undo"
        | "tool-failed"
        | "blocked-command" => Surface::ActionDock,
        "toasts" => Surface::ConversationShell,
        _ => return Err(format!("unknown Flow 5 recovery screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "turn-error" => render_turn_error(&mut canvas, screen_id, fixture_id)?,
        "error-network" => render_error_network(&mut canvas, screen_id, fixture_id)?,
        "error-context" => render_error_context(&mut canvas, screen_id, fixture_id)?,
        "rate-limit" => render_rate_limit(&mut canvas, screen_id, fixture_id)?,
        "interrupt" => render_interrupt(&mut canvas, screen_id, fixture_id)?,
        "toasts" => render_toasts(&mut canvas, screen_id, fixture_id)?,
        "checkpoint-undo" => render_checkpoint_undo(&mut canvas, screen_id, fixture_id)?,
        "tool-failed" => render_tool_failed(&mut canvas, screen_id, fixture_id)?,
        "blocked-command" => render_blocked_command(&mut canvas, screen_id, fixture_id)?,
        _ => unreachable!(),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;
    let paused_activity = match screen_id {
        "error-context" => CONTEXT_PENDING,
        "interrupt" => INTERRUPT_PENDING,
        "toasts" => PENDING_NETWORK,
        _ => PENDING_NETWORK,
    };
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
            conversation_id: "conversation.recovery".to_owned(),
            transcript_id: "transcript.recovery".to_owned(),
            session_id: "session.recovery.1".to_owned(),
            turn_id: "turn.recovery.1".to_owned(),
            user_task: RECOVERY_TASK.to_owned(),
            visible_origin: RECOVERY_ORIGIN.to_owned(),
            submitted_prompt: Some(USER_PROMPT.trim_start_matches("You · ").to_owned()),
            paused_activity_id: Some(paused_activity.to_owned()),
            later_draft: Some(DRAFT.to_owned()),
        }),
    })
}

fn context_variant(fixture_id: &str) -> ContextVariant {
    if fixture_id.contains("evidence") {
        ContextVariant::Evidence
    } else {
        ContextVariant::Collapsed
    }
}

fn render_error_context(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = context_variant(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(11, 4, CONTEXT_COMPLETED_EDITS, TextStyle::muted())?;
    canvas.write(12, 4, CONTEXT_PENDING_TASK, TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;
    canvas.write(15, 4, &format!("✗ {CONTEXT_CAUSE}"), TextStyle::bright())?;
    canvas.write(16, 4, CONTEXT_SAVED, TextStyle::text())?;
    canvas.write(
        17,
        4,
        "Transcript, draft, pending task, and completed edits stay readable.",
        TextStyle::muted(),
    )?;
    canvas.write(
        18,
        4,
        "Draft kept · Keep the risk note short before we publish.",
        TextStyle::muted(),
    )?;
    match variant {
        ContextVariant::Collapsed => {
            canvas.write(
                20,
                4,
                "› Evidence Enter to Expand technical details",
                TextStyle::muted(),
            )?;
        }
        ContextVariant::Evidence => {
            canvas.write(20, 4, "▾ Evidence", TextStyle::text())?;
            canvas.write(
                21,
                4,
                "  Tokens used · 178k of 200k · about 89% of the model limit",
                TextStyle::muted(),
            )?;
            canvas.write(
                22,
                4,
                "  Breakdown · earlier turns, tool results, and open draft",
                TextStyle::muted(),
            )?;
            canvas.write(
                23,
                4,
                "  Compacting keeps full history while freeing room to continue",
                TextStyle::muted(),
            )?;
        }
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Recover from full context",
        TextStyle::bright(),
    )?;
    dock_line(canvas, DOCK_TOP + 2, CONTEXT_CONSEQUENCE, TextStyle::text())?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› {CONTEXT_PRIMARY}"),
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        &format!(" n for {CONTEXT_SECONDARY}"),
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        " Esc to Cancel · keep this conversation paused",
        TextStyle::muted(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-reply",
            "interrupted-answer",
            reference(9, 4, 95),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "completed-work",
            "completed-edits",
            reference(11, 4, 4 + CONTEXT_COMPLETED_EDITS.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-task",
            "risk-note-task",
            reference(12, 4, 4 + CONTEXT_PENDING_TASK.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "recovery-cause",
            "context-exhaustion",
            reference(15, 4, 4 + CONTEXT_CAUSE.chars().count() as u16 + 2),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "context-work-saved",
            reference(16, 4, 4 + CONTEXT_SAVED.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "composer-draft",
            "risk-note-draft",
            reference(18, 4, 58),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.error-context.compact-continue",
            reference(DOCK_TOP + 3, 4, 4 + CONTEXT_PRIMARY.chars().count() as u16 + 10),
            "focused",
            Some(action(
                "action.error-context.compact-continue",
                CONTEXT_PRIMARY,
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.error-context.new-session",
            reference(DOCK_TOP + 4, 4, 4 + CONTEXT_SECONDARY.chars().count() as u16 + 6),
            "summary",
            Some(action(
                "action.error-context.new-session",
                CONTEXT_SECONDARY,
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.error-context.cancel",
            reference(DOCK_TOP + 5, 4, 48),
            "summary",
            Some(action("action.error-context.cancel", "Cancel", false)),
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
        ContextVariant::Collapsed => {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "disclosure",
                "action.error-context.expand-evidence",
                reference(20, 4, 48),
                "summary",
                Some(action(
                    "action.error-context.expand-evidence",
                    "Expand technical details",
                    false,
                )),
            ));
        }
        ContextVariant::Evidence => {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "evidence",
                "token-usage",
                reference(21, 4, 62),
                "evidence",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "disclosure",
                "action.error-context.collapse-evidence",
                reference(20, 4, 16),
                "evidence",
                Some(action(
                    "action.error-context.collapse-evidence",
                    "Collapse technical details",
                    false,
                )),
            ));
        }
    }

    let mut transitions = vec![
        transition(
            "action.error-context.compact-continue",
            "error-context",
            "compact",
            "direct",
            "explicit compact replaces older visible context while full history stays recoverable",
        ),
        transition(
            "action.error-context.new-session",
            "error-context",
            "home",
            "direct",
            "start new session is secondary only and never automatic",
        ),
        transition(
            "action.error-context.cancel",
            "error-context",
            "error-context",
            "cancel",
            "cancel keeps the paused conversation and all refactor state intact",
        ),
    ];
    match variant {
        ContextVariant::Collapsed => transitions.push(transition(
            "action.error-context.expand-evidence",
            "error-context",
            "error-context",
            "direct",
            "evidence discloses token usage after the cause and primary action",
        )),
        ContextVariant::Evidence => transitions.push(transition(
            "action.error-context.collapse-evidence",
            "error-context",
            "error-context",
            "direct",
            "collapsing evidence restores progressive disclosure without losing state",
        )),
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

fn tool_failed_variant(fixture_id: &str) -> ToolFailedVariant {
    if fixture_id.contains("capability") {
        ToolFailedVariant::Capability
    } else if fixture_id.contains("unknown") {
        ToolFailedVariant::Unknown
    } else if fixture_id.contains("completed") {
        ToolFailedVariant::CompletedEffect
    } else if fixture_id.contains("policy") {
        ToolFailedVariant::Policy
    } else if fixture_id.contains("idempotent") || fixture_id.contains("safe") {
        ToolFailedVariant::Idempotent
    } else {
        ToolFailedVariant::MissingFile
    }
}

fn toast_variant(fixture_id: &str) -> ToastVariant {
    if fixture_id.contains("auto-dismiss") || fixture_id.contains("after-dismiss") {
        ToastVariant::AfterAutoDismiss
    } else if fixture_id.contains("retry") || fixture_id.contains("after-retry") {
        ToastVariant::AfterRetry
    } else {
        ToastVariant::Full
    }
}

fn style_for_token(token: crate::chrome::tokens::ColorToken) -> TextStyle {
    TextStyle {
        foreground: token,
        background: None,
        bold: matches!(
            token,
            crate::chrome::tokens::ColorToken::Bright
                | crate::chrome::tokens::ColorToken::Accent
                | crate::chrome::tokens::ColorToken::Bad
                | crate::chrome::tokens::ColorToken::Ok
        ),
    }
}

fn render_interrupt(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let _ = fixture_id;
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(10, 4, "█", TextStyle::bright())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    // Neutral seam: muted rule only. No BAD red or failure glyph.
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        &format!("● {INTERRUPT_STATEMENT} · 12s in"),
        TextStyle::bright(),
    )?;
    canvas.write(16, 4, INTERRUPT_KEPT, TextStyle::text())?;
    canvas.write(17, 4, INTERRUPT_QUEUE_LINE, TextStyle::muted())?;
    canvas.write(
        18,
        4,
        "Draft kept · Keep the risk note short before we publish.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Attachment · notes/risk-follow-up.md · stays queued second",
        TextStyle::muted(),
    )?;
    canvas.write(20, 4, INTERRUPT_CONTINUE_NOTE, TextStyle::muted())?;
    canvas.write(21, 4, INTERRUPT_REPHRASE_NOTE, TextStyle::muted())?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Resume the stopped reply",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Continue keeps the partial answer and runs before queued work.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› {INTERRUPT_PRIMARY}"),
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        &format!(" r for {INTERRUPT_SECONDARY}"),
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        " Esc to Cancel · keep the partial answer",
        TextStyle::muted(),
    )?;

    let pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-reply",
            "interrupted-answer",
            reference(9, 4, 95),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "interrupt-statement",
            "user-stopped-reply",
            reference(15, 4, 4 + INTERRUPT_STATEMENT.chars().count() as u16 + 10),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "partial-kept",
            reference(16, 4, 4 + INTERRUPT_KEPT.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "queued-input",
            "risk-follow-up",
            reference(17, 4, 4 + INTERRUPT_QUEUE_LINE.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.interrupt.continue-reply",
            reference(
                DOCK_TOP + 3,
                4,
                4 + INTERRUPT_PRIMARY.chars().count() as u16 + 10,
            ),
            "focused",
            Some(action(
                "action.interrupt.continue-reply",
                INTERRUPT_PRIMARY,
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.interrupt.rephrase-request",
            reference(
                DOCK_TOP + 4,
                4,
                4 + INTERRUPT_SECONDARY.chars().count() as u16 + 6,
            ),
            "summary",
            Some(action(
                "action.interrupt.rephrase-request",
                INTERRUPT_SECONDARY,
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.interrupt.cancel",
            reference(DOCK_TOP + 5, 4, 46),
            "summary",
            Some(action("action.interrupt.cancel", "Cancel", false)),
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
                "action.interrupt.continue-reply",
                "interrupt",
                "chat-turn",
                "direct",
                "continue appends only new text from the retained partial in the same turn",
            ),
            transition(
                "action.interrupt.rephrase-request",
                "interrupt",
                "chat-turn",
                "direct",
                "rephrase restores the original prompt into the composer for editing",
            ),
            transition(
                "action.interrupt.cancel",
                "interrupt",
                "interrupt",
                "cancel",
                "cancel keeps the partial answer, queued follow-ups, and draft",
            ),
        ],
    ))
}

fn render_toasts(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    use crate::chrome::tokens::ColorToken;

    let variant = toast_variant(fixture_id);
    // Conversation remains fully readable under the compact toast stack.
    canvas.write(2, 4, TOAST_ORIGIN_LINE, TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, "You · /settings save", TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, TOAST_TRANSCRIPT, TextStyle::text())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    canvas.write(
        12,
        4,
        "Draft kept · Keep the risk note short before we publish.",
        TextStyle::muted(),
    )?;

    // Borderless toast stack, upper-right, severity-ordered regardless of timestamps.
    // Construction order would be success (t0), info (t1), warn (t2), failure (t3).
    // Visible order is failure, warn, info, success.
    // Keep lines within the fixed 120-wide canvas (toast_col + longest line <= 120).
    let toast_col: u16 = 68;
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "settings-save",
            reference(5, 4, 28),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "preferences-saved-note",
            reference(9, 4, 4 + TOAST_TRANSCRIPT.chars().count() as u16),
            "summary",
            None,
        ),
    ];

    match variant {
        ToastVariant::Full => {
            // Failure (created last) · actionable · persistent
            canvas.write(
                2,
                toast_col,
                &format!("│ ✗ BAD  {TOAST_FAILURE}"),
                style_for_token(ColorToken::Bad),
            )?;
            canvas.write(
                3,
                toast_col,
                &format!("│ s for {TOAST_RETRY} · stays until you act"),
                TextStyle::muted(),
            )?;
            // Warning (created third)
            canvas.write(
                5,
                toast_col,
                &format!("│ ● WARN  {TOAST_WARN}"),
                style_for_token(ColorToken::Warn),
            )?;
            canvas.write(6, toast_col, "│  clears on its own", TextStyle::muted())?;
            // Info (created second)
            canvas.write(
                8,
                toast_col,
                &format!("│ ● INFO  {TOAST_INFO}"),
                style_for_token(ColorToken::Info),
            )?;
            canvas.write(9, toast_col, "│  clears on its own", TextStyle::muted())?;
            // Success (created first)
            canvas.write(
                11,
                toast_col,
                &format!("│ ✓ OK    {TOAST_SUCCESS}"),
                style_for_token(ColorToken::Ok),
            )?;
            canvas.write(12, toast_col, "│  clears on its own", TextStyle::muted())?;

            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "failure-settings-save",
                reference(2, toast_col, toast_col + 38),
                "focused",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.toasts.retry-save",
                reference(3, toast_col, toast_col + 40),
                "focused",
                Some(action("action.toasts.retry-save", TOAST_RETRY, false)),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "warn-connection",
                reference(5, toast_col, toast_col + 42),
                "summary",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "info-model",
                reference(8, toast_col, toast_col + 28),
                "summary",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "success-checkpoint",
                reference(11, toast_col, toast_col + 28),
                "summary",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast-lifetime",
                "failure-persistent",
                reference(3, toast_col + 4, toast_col + 40),
                "summary",
                None,
            ));
        }
        ToastVariant::AfterAutoDismiss => {
            // Only the actionable failure remains after transient notices auto-dismiss.
            canvas.write(
                2,
                toast_col,
                &format!("│ ✗ BAD  {TOAST_FAILURE}"),
                style_for_token(ColorToken::Bad),
            )?;
            canvas.write(
                3,
                toast_col,
                &format!("│ s for {TOAST_RETRY} · stays until you act"),
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "failure-settings-save",
                reference(2, toast_col, toast_col + 38),
                "focused",
                None,
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.toasts.retry-save",
                reference(3, toast_col, toast_col + 40),
                "focused",
                Some(action("action.toasts.retry-save", TOAST_RETRY, false)),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast-lifetime",
                "transient-dismissed",
                reference(2, toast_col, toast_col + 20),
                "summary",
                None,
            ));
        }
        ToastVariant::AfterRetry => {
            // Failure cleared after the notice-owned Retry save succeeded.
            canvas.write(
                2,
                toast_col,
                "│ ✓ OK    Settings saved",
                style_for_token(ColorToken::Ok),
            )?;
            canvas.write(3, toast_col, "│  clears on its own", TextStyle::muted())?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "toast",
                "success-settings-saved",
                reference(2, toast_col, toast_col + 28),
                "summary",
                None,
            ));
        }
    }

    // Normal composer remains the focus owner; toasts never replace it.
    canvas.write(
        COMPOSER_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_TOP + 1,
        2,
        "│ › Reply when ready…",
        TextStyle::bright(),
    )?;
    canvas.write(
        COMPOSER_TOP + 2,
        2,
        &format!("│   Draft · {DRAFT}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM - 1,
        4,
        "Enter to Send reply",
        TextStyle::accent(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;

    pairs.push(semantic(
        screen_id,
        fixture_id,
        "composer",
        "focus-owner",
        reference(COMPOSER_TOP + 1, 4, 28),
        "focused",
        Some(action("action.composer.send", "Send reply", false)),
    ));
    // Automatic lifetime tick for transient notices. Always present so the
    // automatic-transition bijection matches the transition list.
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "automatic-transition",
        "action.toasts.auto-dismiss",
        reference(12, toast_col, toast_col + 22),
        "summary",
        Some(ActionEvidence {
            action_id: "action.toasts.auto-dismiss".to_owned(),
            label: "Auto-dismiss routine notices".to_owned(),
            enabled: true,
            automatic: true,
        }),
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

    let transitions = vec![
        transition(
            "action.toasts.retry-save",
            "toasts",
            "chat-turn",
            "direct",
            "notice-owned retry returns to the conversation that emitted the toast",
        ),
        transition(
            "action.toasts.auto-dismiss",
            "toasts",
            "toasts",
            "automatic",
            "success and info notices auto-dismiss while failure stays until acted on",
        ),
        transition(
            "action.composer.send",
            "toasts",
            "chat-turn",
            "direct",
            "composer remains usable while toasts stay nonblocking",
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

fn blocked_command_variant(fixture_id: &str) -> BlockedCommandVariant {
    if fixture_id.contains("after-approval") || fixture_id.contains("approval-return") {
        BlockedCommandVariant::AfterApproval
    } else if fixture_id.contains("after-work-style") || fixture_id.contains("work-style-return")
    {
        BlockedCommandVariant::AfterWorkStyle
    } else if fixture_id.contains("permission") || fixture_id.contains("approval") {
        BlockedCommandVariant::Permission
    } else {
        BlockedCommandVariant::WorkStyle
    }
}

fn render_tool_failed(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = tool_failed_variant(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, TOOL_USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        &format!("✓ {TOOL_COMPLETED} · Complete · shared guard found"),
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        &format!("✗ {TOOL_FAILED_NAME} · Needs attention"),
        TextStyle::bright(),
    )?;
    canvas.write(12, 6, TOOL_INTENDED_EFFECT, TextStyle::text())?;
    canvas.write(
        13,
        6,
        &format!("Partial output · {TOOL_PARTIAL_OUTPUT}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;

    let (cause, primary_label, primary_action, primary_target) = match variant {
        ToolFailedVariant::MissingFile => (
            TOOL_CAUSE_MISSING_FILE,
            "Choose a file",
            "action.tool-failed.choose-file",
            "tool-failed",
        ),
        ToolFailedVariant::Capability => (
            TOOL_CAUSE_CAPABILITY,
            "Connect Docs search",
            "action.tool-failed.connect-capability",
            "mcp",
        ),
        ToolFailedVariant::Unknown => (
            TOOL_CAUSE_UNKNOWN,
            "Inspect result",
            "action.tool-failed.inspect-result",
            "tool-failed",
        ),
        ToolFailedVariant::CompletedEffect => (
            TOOL_CAUSE_COMPLETED,
            "Inspect result",
            "action.tool-failed.inspect-result",
            "tool-failed",
        ),
        ToolFailedVariant::Idempotent => (
            TOOL_CAUSE_IDEMPOTENT,
            "Retry tool",
            "action.tool-failed.retry",
            "tool-running",
        ),
        ToolFailedVariant::Policy => (
            TOOL_CAUSE_POLICY,
            "Review policy",
            "action.tool-failed.review-policy",
            "tool-failed",
        ),
    };
    let secondary_label = "Show evidence";
    let secondary_action = "action.tool-failed.show-evidence";

    canvas.write(17, 4, &format!("✗ {cause}"), TextStyle::bright())?;
    canvas.write(
        18,
        4,
        "Completed tools, draft, and partial output stay available.",
        TextStyle::text(),
    )?;
    match variant {
        ToolFailedVariant::Unknown | ToolFailedVariant::CompletedEffect => {
            canvas.write(
                19,
                4,
                "Retry stays off until inspection confirms a safe boundary.",
                TextStyle::muted(),
            )?;
        }
        ToolFailedVariant::Idempotent => {
            canvas.write(
                19,
                4,
                "Retry reuses this invocation and never re-runs completed tools.",
                TextStyle::muted(),
            )?;
        }
        ToolFailedVariant::Capability => {
            canvas.write(
                19,
                4,
                "Connect the human capability, then return to this paused tool.",
                TextStyle::muted(),
            )?;
        }
        ToolFailedVariant::Policy => {
            canvas.write(
                19,
                4,
                "Policy review stays separate from connection setup.",
                TextStyle::muted(),
            )?;
        }
        ToolFailedVariant::MissingFile => {
            canvas.write(
                19,
                4,
                "Choose the file this tool needs, then continue from the same turn.",
                TextStyle::muted(),
            )?;
        }
    }

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Recover the failed tool", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "One next step keeps completed sibling work intact.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› {primary_label}"),
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        &format!(" d for {secondary_label}"),
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        " Esc to Keep this failure in the transcript",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "activity",
            "tool-read-rules",
            reference(9, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "activity",
            "tool-write-release-notes",
            reference(11, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "tool-intent",
            "write-release-notes",
            reference(12, 6, 64),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-output",
            "write-release-notes-partial",
            reference(13, 6, 50),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "recovery-cause",
            match variant {
                ToolFailedVariant::MissingFile => "missing-file",
                ToolFailedVariant::Capability => "capability-unavailable",
                ToolFailedVariant::Unknown => "unknown-outcome",
                ToolFailedVariant::CompletedEffect => "completed-effect",
                ToolFailedVariant::Idempotent => "idempotent-failure",
                ToolFailedVariant::Policy => "policy-blocked",
            },
            reference(17, 4, 4 + 2 + cause.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            primary_action,
            reference(DOCK_TOP + 3, 4, 4 + 10 + primary_label.chars().count() as u16),
            "focused",
            Some(action(primary_action, primary_label, false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            secondary_action,
            reference(
                DOCK_TOP + 4,
                4,
                4 + 5 + secondary_label.chars().count() as u16,
            ),
            "summary",
            Some(action(secondary_action, secondary_label, false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.tool-failed.cancel",
            reference(DOCK_TOP + 5, 4, 48),
            "summary",
            Some(action(
                "action.tool-failed.cancel",
                "Keep this failure in the transcript",
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

    let mut transitions = vec![
        transition(
            primary_action,
            "tool-failed",
            primary_target,
            "direct",
            match variant {
                ToolFailedVariant::MissingFile => {
                    "choose a file keeps the failed tool identity and completed sibling results"
                }
                ToolFailedVariant::Capability => {
                    "connect capability opens the capability surface without losing the tool"
                }
                ToolFailedVariant::Unknown | ToolFailedVariant::CompletedEffect => {
                    "inspection is required before any retry or resume"
                }
                ToolFailedVariant::Idempotent => {
                    "safe retry reuses the paused tool and never duplicates completed siblings"
                }
                ToolFailedVariant::Policy => {
                    "policy review stays on the failed tool without starting an invocation"
                }
            },
        ),
        transition(
            "action.tool-failed.cancel",
            "tool-failed",
            "tool-failed",
            "cancel",
            "cancel keeps the failure, draft, and completed tool work in place",
        ),
    ];
    if matches!(
        variant,
        ToolFailedVariant::Unknown | ToolFailedVariant::CompletedEffect
    ) {
        // Explicitly do not expose a retry transition for unknown/completed-effect outcomes.
    } else if matches!(variant, ToolFailedVariant::Idempotent) {
        transitions.push(transition(
            "action.tool-failed.retry",
            "tool-failed",
            "tool-running",
            "direct",
            "safe retry resumes the same invocation once",
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

fn render_blocked_command(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = blocked_command_variant(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, TOOL_USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, BLOCKED_COMPLETED_WORK, TextStyle::muted())?;
    canvas.write(
        11,
        4,
        "✗ Blocked command · did not run",
        TextStyle::bright(),
    )?;
    canvas.write(12, 6, BLOCKED_COMMAND, TextStyle::text())?;
    canvas.write(
        13,
        6,
        "Composer draft kept · Keep the risk note short before we publish.",
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;

    let (cause, primary_label, primary_action, primary_target, note) = match variant {
        BlockedCommandVariant::WorkStyle => (
            BLOCKED_POLICY_CAUSE,
            "Change how Code works",
            "action.blocked-command.change-work-style",
            "agent-picker",
            "Change the work style, then return for one explicit run decision.",
        ),
        BlockedCommandVariant::Permission => (
            BLOCKED_PERMISSION_CAUSE,
            "Request approval",
            "action.blocked-command.request-approval",
            "permission-shell",
            "Approval stays separate; the command still needs an explicit run.",
        ),
        BlockedCommandVariant::AfterApproval => (
            BLOCKED_PERMISSION_CAUSE,
            "Run command once",
            "action.blocked-command.run-once",
            "shell",
            "Approval returned. Run once only when you choose it here.",
        ),
        BlockedCommandVariant::AfterWorkStyle => (
            BLOCKED_POLICY_CAUSE,
            "Run command once",
            "action.blocked-command.run-once",
            "shell",
            "Work style returned. Run once only when you choose it here.",
        ),
    };

    canvas.write(17, 4, &format!("✗ {cause}"), TextStyle::bright())?;
    canvas.write(
        18,
        4,
        "Earlier completed work stays readable and is not marked blocked.",
        TextStyle::text(),
    )?;
    canvas.write(19, 4, note, TextStyle::muted())?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Recover the blocked command",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Keep the exact command available without running it yet.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("› {primary_label}"),
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        " d for Show evidence",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        " Esc to Keep command blocked",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "completed-work",
            "retry-window-copy",
            reference(9, 4, 68),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "blocked-command",
            "command-not-run",
            reference(11, 4, 36),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "command",
            "blocked-shell-command",
            reference(12, 6, 6 + BLOCKED_COMMAND.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "composer-draft",
            reference(13, 6, 62),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "recovery-cause",
            match variant {
                BlockedCommandVariant::WorkStyle | BlockedCommandVariant::AfterWorkStyle => {
                    "work-style-restriction"
                }
                BlockedCommandVariant::Permission | BlockedCommandVariant::AfterApproval => {
                    "permission-required"
                }
            },
            reference(17, 4, 4 + 2 + cause.chars().count() as u16),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            primary_action,
            reference(DOCK_TOP + 3, 4, 4 + 10 + primary_label.chars().count() as u16),
            "focused",
            Some(action(primary_action, primary_label, false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.blocked-command.show-evidence",
            reference(DOCK_TOP + 4, 4, 22),
            "summary",
            Some(action(
                "action.blocked-command.show-evidence",
                "Show evidence",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.blocked-command.cancel",
            reference(DOCK_TOP + 5, 4, 34),
            "summary",
            Some(action(
                "action.blocked-command.cancel",
                "Keep command blocked",
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
                primary_action,
                "blocked-command",
                primary_target,
                "direct",
                match variant {
                    BlockedCommandVariant::WorkStyle => {
                        "change how Code works routes to the work-style surface and keeps the command"
                    }
                    BlockedCommandVariant::Permission => {
                        "request approval routes to the permission surface without executing"
                    }
                    BlockedCommandVariant::AfterApproval | BlockedCommandVariant::AfterWorkStyle => {
                        "an explicit run decision stays on the blocked command after return"
                    }
                },
            ),
            transition(
                "action.blocked-command.cancel",
                "blocked-command",
                "blocked-command",
                "cancel",
                "cancel keeps the verbatim command and draft without execution",
            ),
        ],
    ))
}

fn render_turn_error(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    // Visible seam between partial reply and failure recovery card.
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;
    canvas.write(15, 4, &format!("✗ {TURN_CAUSE}"), TextStyle::bright())?;
    canvas.write(
        16,
        4,
        PARTIAL_KEPT,
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "Draft, transcript, and completed work stay readable.",
        TextStyle::muted(),
    )?;
    canvas.write(
        18,
        4,
        "Inspect the connection problem when the provider path looks broken.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Retry starts a new reply and replaces the partial text.",
        TextStyle::muted(),
    )?;
    canvas.write(
        20,
        4,
        "Continue keeps the partial reply and restores the composer.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Recover the interrupted turn", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Pick whether to replace or keep the partial reply.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› Retry reply",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        " c for Continue from partial reply",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        " Esc to Cancel · keep failure record",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-reply",
            "interrupted-answer",
            reference(9, 4, 95),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "recovery-cause",
            "turn-stopped",
            reference(15, 4, 42),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.turn-error.retry",
            reference(DOCK_TOP + 3, 4, 28),
            "focused",
            Some(action("action.turn-error.retry", "Retry reply", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.turn-error.continue",
            reference(DOCK_TOP + 4, 4, 42),
            "summary",
            Some(action(
                "action.turn-error.continue",
                "Continue from partial reply",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.turn-error.cancel",
            reference(DOCK_TOP + 5, 4, 40),
            "summary",
            Some(action("action.turn-error.cancel", "Cancel", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.turn-error.inspect-connection",
            // Visible inspect-copy lives on canvas row 18 (row 17 is draft preservation).
            reference(18, 4, 58),
            "summary",
            Some(action(
                "action.turn-error.inspect-connection",
                "Inspect the connection problem",
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
                "action.turn-error.retry",
                "turn-error",
                "chat-turn",
                "direct",
                "successful turn retry restores the same conversation with a new reply",
            ),
            transition(
                "action.turn-error.continue",
                "turn-error",
                "chat-turn",
                "direct",
                "continue restores the ordinary composer while keeping the partial reply",
            ),
            transition(
                "action.turn-error.cancel",
                "turn-error",
                "turn-error",
                "cancel",
                "cancel keeps the partial reply and failure record in place",
            ),
            transition(
                "action.turn-error.inspect-connection",
                "turn-error",
                "error-network",
                "direct",
                "inspecting the connection problem opens the network recovery state",
            ),
        ],
    ))
}

fn render_error_network(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = network_variant(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;

    let cause = match variant {
        NetworkVariant::Offline => NETWORK_CAUSE_OFFLINE,
        NetworkVariant::Backoff => NETWORK_CAUSE_GENERIC,
        NetworkVariant::Exhausted => NETWORK_CAUSE_ENDED,
        _ => NETWORK_CAUSE_GENERIC,
    };
    canvas.write(15, 4, &format!("✗ {cause}"), TextStyle::bright())?;
    canvas.write(16, 4, NETWORK_SAVED, TextStyle::text())?;
    canvas.write(
        17,
        4,
        "Transcript, draft, and completed work stay readable.",
        TextStyle::muted(),
    )?;

    match variant {
        NetworkVariant::Collapsed => {
            canvas.write(
                19,
                4,
                "› Evidence Enter to Expand technical details",
                TextStyle::muted(),
            )?;
        }
        NetworkVariant::Details => {
            canvas.write(
                19,
                4,
                "▾ Details",
                TextStyle::text(),
            )?;
            canvas.write(
                20,
                4,
                "  The provider accepted earlier requests from this session.",
                TextStyle::muted(),
            )?;
            canvas.write(
                21,
                4,
                "  This failure is about reaching it again, not about your files.",
                TextStyle::muted(),
            )?;
        }
        NetworkVariant::Evidence => {
            canvas.write(19, 4, "▾ Evidence", TextStyle::text())?;
            canvas.write(
                20,
                4,
                "  Provider · Anthropic",
                TextStyle::muted(),
            )?;
            canvas.write(
                21,
                4,
                "  Endpoint · [redacted host]",
                TextStyle::muted(),
            )?;
            canvas.write(
                22,
                4,
                "  Attempts · 3 of 3 · backoff 2s → 4s → 8s",
                TextStyle::muted(),
            )?;
            canvas.write(
                23,
                4,
                "  Last request · req_78c4a1  ·  low-level · connection timed out",
                TextStyle::muted(),
            )?;
        }
        NetworkVariant::Offline => {
            canvas.write(
                19,
                4,
                "No automatic retry runs while network access stays off.",
                TextStyle::muted(),
            )?;
        }
        NetworkVariant::Backoff => {
            canvas.write(
                19,
                4,
                "○ Trying again automatically · attempt 2 of 3 · next in 4s",
                TextStyle::text(),
            )?;
            canvas.write(
                20,
                4,
                "One timer owns this wait. Cancel stops future attempts.",
                TextStyle::muted(),
            )?;
        }
        NetworkVariant::Exhausted => {
            canvas.write(
                19,
                4,
                "Automatic retries stopped after 3 attempts.",
                TextStyle::text(),
            )?;
            canvas.write(
                20,
                4,
                "Choose a manual recovery when you are ready.",
                TextStyle::muted(),
            )?;
        }
    }

    write_dock_border(canvas)?;
    let mut transitions = Vec::new();
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-reply",
            "interrupted-answer",
            reference(9, 4, 95),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "recovery-cause",
            "network-failure",
            reference(15, 4, 4 + cause.chars().count() as u16 + 2),
            "focused",
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
        NetworkVariant::Offline => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "Network recovery",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Review the network setting that paused this model action.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Review network setting",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " Esc to Cancel · keep paused request",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "No timer or provider attempt runs here.",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.error-network.review-setting",
                reference(DOCK_TOP + 3, 4, 40),
                "focused",
                Some(action(
                    "action.error-network.review-setting",
                    "Review network setting",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.error-network.cancel",
                reference(DOCK_TOP + 4, 4, 42),
                "summary",
                Some(action("action.error-network.cancel", "Cancel", false)),
            ));
            transitions.push(transition(
                "action.error-network.review-setting",
                "error-network",
                "connect-provider",
                "direct",
                "offline recovery opens the relevant connection review with the paused request as origin",
            ));
            transitions.push(transition(
                "action.error-network.cancel",
                "error-network",
                "error-network",
                "cancel",
                "cancel keeps the offline recovery state without attempts",
            ));
        }
        NetworkVariant::Backoff => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "Automatic network wait",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "One bounded backoff timer owns the next attempt.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "  Waiting · no primary action while the timer runs",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " Esc to Cancel",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "Cancel stops future attempts and keeps this origin.",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "timer-owner",
                TIMER_OWNER,
                reference(19, 4, 68),
                "focused",
                Some(action(
                    "action.error-network.backoff-tick",
                    "Automatic backoff",
                    true,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.error-network.cancel-backoff",
                reference(DOCK_TOP + 4, 4, 18),
                "summary",
                Some(action(
                    "action.error-network.cancel-backoff",
                    "Cancel",
                    false,
                )),
            ));
            if fixture_id.contains("gate.recovery.error-network") {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "bridge-note",
                    "action.error-network.storyboard-rate-window",
                    reference(19, 4, 48),
                    "summary",
                    Some(action(
                        "action.error-network.storyboard-rate-window",
                        "Review the automatic wait that follows diagnosis",
                        false,
                    )),
                ));
                transitions.push(transition(
                    "action.error-network.storyboard-rate-window",
                    "error-network",
                    "rate-limit",
                    "illustrative",
                    "connection diagnosis is reviewed first; the ordered storyboard next opens the automatic rate-limit wait as an illustrative adjacency",
                ));
            }
            transitions.push(transition(
                "action.error-network.backoff-tick",
                "error-network",
                "error-network",
                "automatic",
                "the single backoff owner advances one deterministic attempt",
            ));
            transitions.push(transition(
                "action.error-network.cancel-backoff",
                "error-network",
                "error-network",
                "cancel",
                "cancel stops the timer and returns a stable preserved origin",
            ));
        }
        NetworkVariant::Exhausted => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "Network recovery",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Automatic attempts are exhausted. Retry is manual.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Retry request",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " d for Check connection",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                " Esc to Cancel · keep paused request",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.error-network.retry",
                reference(DOCK_TOP + 3, 4, 30),
                "focused",
                Some(action(
                    "action.error-network.retry",
                    "Retry request",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.error-network.check-connection",
                reference(DOCK_TOP + 4, 4, 30),
                "summary",
                Some(action(
                    "action.error-network.check-connection",
                    "Check connection",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.error-network.cancel",
                reference(DOCK_TOP + 5, 4, 42),
                "summary",
                Some(action("action.error-network.cancel", "Cancel", false)),
            ));
            transitions.extend([
                transition(
                    "action.error-network.retry",
                    "error-network",
                    "chat-turn",
                    "direct",
                    "manual retry resumes the exact paused request once",
                ),
                transition(
                    "action.error-network.check-connection",
                    "error-network",
                    "connect-provider",
                    "direct",
                    "connection inspection opens with the failed model action as origin",
                ),
                transition(
                    "action.error-network.cancel",
                    "error-network",
                    "error-network",
                    "cancel",
                    "cancel leaves the exhausted recovery origin intact",
                ),
            ]);
        }
        NetworkVariant::Collapsed | NetworkVariant::Details | NetworkVariant::Evidence => {
            dock_line(
                canvas,
                DOCK_TOP + 1,
                "Network recovery",
                TextStyle::bright(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Retry the paused request or inspect the connection.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Retry request",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " d for Check connection",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                if matches!(variant, NetworkVariant::Evidence) {
                    " Esc to Hide evidence"
                } else {
                    " Esc to Cancel · keep paused request"
                },
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.error-network.retry",
                reference(DOCK_TOP + 3, 4, 30),
                "focused",
                Some(action(
                    "action.error-network.retry",
                    "Retry request",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.error-network.check-connection",
                reference(DOCK_TOP + 4, 4, 30),
                "summary",
                Some(action(
                    "action.error-network.check-connection",
                    "Check connection",
                    false,
                )),
            ));
            if matches!(variant, NetworkVariant::Evidence) {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "evidence",
                    "network-technical-evidence",
                    reference(19, 4, 20),
                    "evidence",
                    Some(action(
                        "action.error-network.hide-evidence",
                        "Hide evidence",
                        false,
                    )),
                ));
            } else if matches!(variant, NetworkVariant::Collapsed) {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "disclosure",
                    "network-evidence-collapsed",
                    reference(19, 4, 48),
                    "summary",
                    Some(action(
                        "action.error-network.show-evidence",
                        "Show evidence",
                        false,
                    )),
                ));
            } else {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "disclosure",
                    "network-details",
                    reference(19, 4, 14),
                    "focused",
                    Some(action(
                        "action.error-network.hide-details",
                        "Hide details",
                        false,
                    )),
                ));
            }
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.error-network.cancel",
                reference(DOCK_TOP + 5, 4, 42),
                "summary",
                Some(action("action.error-network.cancel", "Cancel", false)),
            ));
            transitions.extend([
                transition(
                    "action.error-network.retry",
                    "error-network",
                    "chat-turn",
                    "direct",
                    "manual retry resumes the exact paused request once",
                ),
                transition(
                    "action.error-network.check-connection",
                    "error-network",
                    "connect-provider",
                    "direct",
                    "connection inspection opens with the failed model action as origin",
                ),
                transition(
                    "action.error-network.cancel",
                    "error-network",
                    "error-network",
                    "cancel",
                    "cancel preserves the network recovery origin",
                ),
            ]);
            if matches!(variant, NetworkVariant::Evidence) {
                transitions.push(transition(
                    "action.error-network.hide-evidence",
                    "error-network",
                    "error-network",
                    "direct",
                    "evidence collapses back to the human summary",
                ));
            } else if matches!(variant, NetworkVariant::Collapsed) {
                transitions.push(transition(
                    "action.error-network.show-evidence",
                    "error-network",
                    "error-network",
                    "direct",
                    "evidence discloses attempts and endpoint detail on request",
                ));
            } else {
                transitions.push(transition(
                    "action.error-network.hide-details",
                    "error-network",
                    "error-network",
                    "direct",
                    "distinct details collapse without emptying evidence later",
                ));
            }
        }
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

fn network_variant(fixture_id: &str) -> NetworkVariant {
    if fixture_id.contains("evidence") {
        NetworkVariant::Evidence
    } else if fixture_id.contains("details") {
        NetworkVariant::Details
    } else if fixture_id.contains("offline") {
        NetworkVariant::Offline
    } else if fixture_id.contains("exhaust") {
        NetworkVariant::Exhausted
    } else if fixture_id.contains("backoff")
        || fixture_id.contains("automatic")
        || fixture_id.contains("gate.recovery.error-network")
    {
        NetworkVariant::Backoff
    } else {
        NetworkVariant::Collapsed
    }
}

fn render_rate_limit(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = rate_limit_variant(fixture_id);
    let countdown = match variant {
        RateLimitVariant::WaitingEarly => "20s",
        RateLimitVariant::WaitingLater => "12s",
        RateLimitVariant::Cancelled => "0s",
    };
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;

    match variant {
        RateLimitVariant::WaitingEarly | RateLimitVariant::WaitingLater => {
            canvas.write(
                15,
                4,
                &format!("○ Safe waiting window · retry in {countdown} remaining"),
                TextStyle::text(),
            )?;
            canvas.write(
                16,
                4,
                "Your work is saved. Code will retry automatically.",
                TextStyle::bright(),
            )?;
            canvas.write(
                17,
                4,
                "Transcript, partial reply, draft, and completed work stay readable.",
                TextStyle::muted(),
            )?;
            canvas.write(
                18,
                4,
                "One timer owns this wait. No primary action is required.",
                TextStyle::muted(),
            )?;
        }
        RateLimitVariant::Cancelled => {
            canvas.write(
                15,
                4,
                "○ Automatic wait cancelled",
                TextStyle::text(),
            )?;
            canvas.write(
                16,
                4,
                "Your work is still saved. No provider attempt was sent.",
                TextStyle::bright(),
            )?;
            canvas.write(
                17,
                4,
                "Transcript, partial reply, draft, and completed work stay readable.",
                TextStyle::muted(),
            )?;
            canvas.write(
                18,
                4,
                "Choose a recovery later when you are ready.",
                TextStyle::muted(),
            )?;
        }
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Automatic rate-limit wait",
        TextStyle::bright(),
    )?;
    match variant {
        RateLimitVariant::WaitingEarly | RateLimitVariant::WaitingLater => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Code retries once when the safe window ends.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "  Waiting · no primary action while the timer runs",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " r for Retry now · optional",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                " Esc to Cancel · stop automatic wait",
                TextStyle::muted(),
            )?;
        }
        RateLimitVariant::Cancelled => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "The automatic wait was stopped without a provider attempt.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "  No countdown remains active",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " r for Retry now · optional",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                " Esc to Keep cancelled wait",
                TextStyle::muted(),
            )?;
        }
    }

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "partial-reply",
            "interrupted-answer",
            reference(9, 4, 95),
            "focused",
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
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.rate-limit.retry-now",
            reference(DOCK_TOP + 4, 4, 30),
            "summary",
            Some(action(
                "action.rate-limit.retry-now",
                "Retry now",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.rate-limit.cancel",
            reference(DOCK_TOP + 5, 4, 36),
            "summary",
            Some(action("action.rate-limit.cancel", "Cancel", false)),
        ),
    ];
    let mut transitions = vec![
        transition(
            "action.rate-limit.retry-now",
            "rate-limit",
            "chat-turn",
            "direct",
            "optional manual retry wins once and restores the paused conversation",
        ),
        transition(
            "action.rate-limit.cancel",
            "rate-limit",
            "rate-limit",
            "cancel",
            "cancel stops the single countdown owner without a provider attempt",
        ),
    ];

    if matches!(
        variant,
        RateLimitVariant::WaitingEarly | RateLimitVariant::WaitingLater
    ) {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "timer-owner",
            RATE_TIMER_OWNER,
            reference(15, 4, 58),
            "focused",
            Some(action(
                "action.rate-limit.timer-tick",
                "Automatic rate-limit wait",
                true,
            )),
        ));
        transitions.push(transition(
            "action.rate-limit.timer-tick",
            "rate-limit",
            "chat-turn",
            "automatic",
            "countdown completion triggers exactly one provider attempt and restores chat",
        ));
    } else {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "waiting-cancelled",
            "rate-limit-cancelled",
            reference(15, 4, 30),
            "focused",
            None,
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

fn rate_limit_variant(fixture_id: &str) -> RateLimitVariant {
    if fixture_id.contains("sample-later") {
        RateLimitVariant::WaitingLater
    } else if fixture_id.contains("cancel") {
        RateLimitVariant::Cancelled
    } else {
        // gate fixture and sample-early share the early countdown snapshot.
        RateLimitVariant::WaitingEarly
    }
}

fn render_checkpoint_undo(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let variant = checkpoint_variant(fixture_id);
    canvas.write(2, 4, subject_title_for_origin(RECOVERY_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, RECOVERY_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, PARTIAL_REPLY, TextStyle::text())?;
    canvas.write(11, 4, COMPLETED_WORK, TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "────────────────────────────────────────────────────────",
        TextStyle::muted(),
    )?;

    match variant {
        CheckpointVariant::Success => {
            canvas.write(
                15,
                4,
                &format!("✓ Restored checkpoint · {CHECKPOINT_NAME}"),
                TextStyle::bright(),
            )?;
            canvas.write(
                16,
                4,
                "2 files were restored. Unrelated work stays untouched.",
                TextStyle::text(),
            )?;
            canvas.write(18, 4, &format!("  {AFFECTED_A}"), TextStyle::muted())?;
            canvas.write(19, 4, &format!("  {AFFECTED_B}"), TextStyle::muted())?;
            canvas.write(
                21,
                4,
                STALE_APPROVAL,
                TextStyle::muted(),
            )?;
            canvas.write(
                22,
                4,
                "Transcript, draft, queue, and later notes remain available.",
                TextStyle::muted(),
            )?;
        }
        CheckpointVariant::Conflict => {
            canvas.write(
                15,
                4,
                &format!("✗ Restore refused · {CHECKPOINT_NAME}"),
                TextStyle::bright(),
            )?;
            canvas.write(
                16,
                4,
                "Target files changed after the checkpoint. No files were written.",
                TextStyle::text(),
            )?;
            canvas.write(18, 4, &format!("  {AFFECTED_A} · conflict"), TextStyle::muted())?;
            canvas.write(19, 4, &format!("  {AFFECTED_B} · conflict"), TextStyle::muted())?;
            canvas.write(
                21,
                4,
                "The checkpoint is still available. Review the conflict before retrying.",
                TextStyle::muted(),
            )?;
        }
        CheckpointVariant::Noop => {
            canvas.write(
                15,
                4,
                &format!("○ Already restored · {CHECKPOINT_NAME}"),
                TextStyle::bright(),
            )?;
            canvas.write(
                16,
                4,
                "No further changes. 2 files already match the checkpoint.",
                TextStyle::text(),
            )?;
            canvas.write(18, 4, &format!("  {AFFECTED_A}"), TextStyle::muted())?;
            canvas.write(19, 4, &format!("  {AFFECTED_B}"), TextStyle::muted())?;
            canvas.write(
                21,
                4,
                "Unrelated work, draft, and queue remain exactly as they were.",
                TextStyle::muted(),
            )?;
        }
        CheckpointVariant::StaleRedo => {
            canvas.write(
                15,
                4,
                &format!("✓ Restored checkpoint · {CHECKPOINT_NAME}"),
                TextStyle::bright(),
            )?;
            canvas.write(
                16,
                4,
                "2 files were restored. A later edit made redo unavailable.",
                TextStyle::text(),
            )?;
            canvas.write(18, 4, &format!("  {AFFECTED_A}"), TextStyle::muted())?;
            canvas.write(19, 4, &format!("  {AFFECTED_B}"), TextStyle::muted())?;
            canvas.write(
                21,
                4,
                "Redo was invalidated after a fresh mutation of a restored file.",
                TextStyle::muted(),
            )?;
        }
    }

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Checkpoint restore",
        TextStyle::bright(),
    )?;
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "paused-conversation",
            reference(2, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-request",
            "release-notes-prompt",
            reference(5, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "confirmation",
            "checkpoint-confirmation",
            reference(15, 4, 56),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "affected-files",
            "checkpoint-affected-paths",
            reference(18, 4, 28),
            "focused",
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
    let mut transitions = Vec::new();

    match variant {
        CheckpointVariant::Success => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Redo reapplies the same checkpoint file set.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Redo change",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " c for Continue in conversation",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                " Esc to Back · keep restoration",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.checkpoint-undo.redo",
                reference(DOCK_TOP + 3, 4, 28),
                "focused",
                Some(action(
                    "action.checkpoint-undo.redo",
                    "Redo change",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.checkpoint-undo.continue",
                reference(DOCK_TOP + 4, 4, 34),
                "summary",
                Some(action(
                    "action.checkpoint-undo.continue",
                    "Continue in conversation",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.checkpoint-undo.back",
                reference(DOCK_TOP + 5, 4, 34),
                "summary",
                Some(action(
                    "action.checkpoint-undo.back",
                    "Back",
                    false,
                )),
            ));
            transitions.extend([
                transition(
                    "action.checkpoint-undo.redo",
                    "checkpoint-undo",
                    "chat-turn",
                    "direct",
                    "redo reapplies the same checkpoint and restores the chat composer",
                ),
                transition(
                    "action.checkpoint-undo.continue",
                    "checkpoint-undo",
                    "chat-turn",
                    "direct",
                    "continue returns to the exact prior chat position with a restored composer",
                ),
                transition(
                    "action.checkpoint-undo.back",
                    "checkpoint-undo",
                    "checkpoint-undo",
                    "cancel",
                    "back keeps the inline confirmation without rewriting files",
                ),
            ]);
        }
        CheckpointVariant::Conflict => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Review the changed files before another restore.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Review conflict",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " c for Continue in conversation",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                " Esc to Back · keep current files",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.checkpoint-undo.review-conflict",
                reference(DOCK_TOP + 3, 4, 30),
                "focused",
                Some(action(
                    "action.checkpoint-undo.review-conflict",
                    "Review conflict",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "action.checkpoint-undo.continue",
                reference(DOCK_TOP + 4, 4, 34),
                "summary",
                Some(action(
                    "action.checkpoint-undo.continue",
                    "Continue in conversation",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.checkpoint-undo.back",
                reference(DOCK_TOP + 5, 4, 34),
                "summary",
                Some(action(
                    "action.checkpoint-undo.back",
                    "Back",
                    false,
                )),
            ));
            transitions.extend([
                transition(
                    "action.checkpoint-undo.review-conflict",
                    "checkpoint-undo",
                    "checkpoint-undo",
                    "direct",
                    "review conflict stays with the checkpoint and writes no files",
                ),
                transition(
                    "action.checkpoint-undo.continue",
                    "checkpoint-undo",
                    "chat-turn",
                    "direct",
                    "continue returns to chat without applying a refused restore",
                ),
                transition(
                    "action.checkpoint-undo.back",
                    "checkpoint-undo",
                    "checkpoint-undo",
                    "cancel",
                    "back keeps the conflict confirmation without writing files",
                ),
            ]);
        }
        CheckpointVariant::Noop => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "The selected checkpoint is already current.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Continue in conversation",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " Esc to Stay on confirmation",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "No extra writes or duplicate toasts were created.",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.checkpoint-undo.continue",
                reference(DOCK_TOP + 3, 4, 40),
                "focused",
                Some(action(
                    "action.checkpoint-undo.continue",
                    "Continue in conversation",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.checkpoint-undo.back",
                reference(DOCK_TOP + 4, 4, 32),
                "summary",
                Some(action(
                    "action.checkpoint-undo.back",
                    "Back",
                    false,
                )),
            ));
            transitions.extend([
                transition(
                    "action.checkpoint-undo.continue",
                    "checkpoint-undo",
                    "chat-turn",
                    "direct",
                    "continue leaves the already-restored files and opens the composer",
                ),
                transition(
                    "action.checkpoint-undo.back",
                    "checkpoint-undo",
                    "checkpoint-undo",
                    "cancel",
                    "back keeps the single no-op confirmation without rewrite",
                ),
            ]);
        }
        CheckpointVariant::StaleRedo => {
            dock_line(
                canvas,
                DOCK_TOP + 2,
                "Redo is unavailable after a later file change.",
                TextStyle::text(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 3,
                "› Continue in conversation",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                " Esc to Stay on confirmation",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "No automatic rewrite runs while redo remains stale.",
                TextStyle::muted(),
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "action.checkpoint-undo.continue",
                reference(DOCK_TOP + 3, 4, 40),
                "focused",
                Some(action(
                    "action.checkpoint-undo.continue",
                    "Continue in conversation",
                    false,
                )),
            ));
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "cancel-action",
                "action.checkpoint-undo.back",
                reference(DOCK_TOP + 4, 4, 32),
                "summary",
                Some(action(
                    "action.checkpoint-undo.back",
                    "Back",
                    false,
                )),
            ));
            transitions.extend([
                transition(
                    "action.checkpoint-undo.continue",
                    "checkpoint-undo",
                    "chat-turn",
                    "direct",
                    "continue returns to chat after redo was invalidated",
                ),
                transition(
                    "action.checkpoint-undo.back",
                    "checkpoint-undo",
                    "checkpoint-undo",
                    "cancel",
                    "back keeps the structured confirmation without a redo action",
                ),
            ]);
        }
    }

    // Prevent unused-constant warnings while still documenting the exclusion rule.
    let _ = UNAFFECTED;

    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn checkpoint_variant(fixture_id: &str) -> CheckpointVariant {
    if fixture_id.contains("conflict") {
        CheckpointVariant::Conflict
    } else if fixture_id.contains("noop") || fixture_id.contains("no-op") {
        CheckpointVariant::Noop
    } else if fixture_id.contains("stale") {
        CheckpointVariant::StaleRedo
    } else {
        CheckpointVariant::Success
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
        DOCK_TOP + 6,
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
