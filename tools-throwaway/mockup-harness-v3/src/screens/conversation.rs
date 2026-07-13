//! Daily conversation pilot fixtures.
//!
//! The first three canonical states deliberately share the same resumed
//! transcript. Home emphasizes entry, the completed turn emphasizes reading,
//! and streaming adds exactly one compact live owner without replacing either
//! the transcript or the bottom composer.

use crate::scenario::diff_evidence_for_fixture;
use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace, WorkStyleEvidence,
    },
    chrome::{
        components::{
            ActivityRow, ActivityState, Component, DisclosureBlock, DisclosureState,
        },
        layout::{CANVAS_ROWS, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;
const DAILY_WORK_TASK: &str =
    "Find where the server applies the rate limit and explain the safe path.";
const DAILY_WORK_ORIGIN: &str = "Daily work · current conversation";
const FLOW2_TASK: &str = "Check the retry path before we merge.";
/// Continuous pause-window You-line: must match FLOW2_TASK (no synonym swaps).
const FLOW2_YOU_LINE: &str = "You · Check the retry path before we merge.";
const FLOW2_ORIGIN: &str = "Input and approval · paused conversation";
const FLOW2_SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
const FLOW2_LATER_DRAFT: &str = "Add a short note about the remaining test hunk.";
const FLOW2_ACTIVITY_ID: &str = "activity.retry-policy-edit.18";
/// Product copy for the paused edit (IDs stay audit-only).
const FLOW2_PAUSED_ACTIVITY_PLAIN: &str = "Paused activity · retry-policy edit · still open";
const FLOW2_SESSION_PLAIN: &str = "Session · retry-path review · same paused turn";
const FLOW2_TURN_PLAIN: &str = "Turn · same pause window · resumed";
const FLOW2_SESSION_TURN_PLAIN: &str =
    "Session · retry-path review · same pause window · continuous";
const FLOW3_RESUME_TASK: &str = "Resume a conversation or inspect its history.";
const FLOW3_CURRENT_ORIGIN: &str = "Work configuration · current conversation";
const FLOW5_TASK: &str = "Finish the release notes without losing the current work.";
const FLOW5_ORIGIN: &str = "Recovery · paused conversation";
const FLOW5_USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const FLOW5_PARTIAL: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const FLOW5_COMPLETED: &str =
    "Completed earlier · retry window copy accepted for the release branch.";
const FLOW5_DRAFT: &str = "Keep the risk note short before we publish.";
const FLOW6_TASK: &str = "Prepare the release safely and review the result.";
const FLOW6_ORIGIN: &str = "Mission · current conversation";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

/// Fixture-bound chat-turn journey. Shared screen ID must never silently fall
/// through to Daily work or another origin when the fixture is known for a
/// different journey (F2-049 / P1-1 fail-closed).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChatTurnJourney {
    Flow1Daily,
    Flow2InputAndApproval,
    Flow3SessionReturn,
    Flow3WorkStyleReturn,
    Flow5Recovery,
    Flow6MissionReturn,
    ToolInterrupt,
    StreamingInterrupt,
}

/// Classifies a chat-turn fixture. Unknown fixtures fail closed instead of
/// inheriting Daily-work origin/task/session identity by accident.
fn classify_chat_turn_fixture(fixture_id: &str) -> Result<ChatTurnJourney, String> {
    if fixture_id.trim().is_empty() {
        return Err("chat-turn fixture identity is required".to_owned());
    }

    // Explicit permission / diff / staging after-states stay Input-and-approval.
    if fixture_id.starts_with("case.permission-")
        || fixture_id.starts_with("case.diff-")
        || fixture_id.starts_with("case.staging-finish")
        || fixture_id.starts_with("case.staging-apply")
        || fixture_id.starts_with("case.staging-include")
        || fixture_id.starts_with("case.staging-exclude")
        || fixture_id.starts_with("case.staging-cancel")
        || matches!(
            fixture_id,
            "flow2.input-and-approval.stable"
                | "case.staging-finish.tick.1.flow2.input-and-approval.stable"
        )
        || fixture_id.starts_with("flow2-storyboard.")
        || fixture_id.starts_with("flow2.")
    {
        return Ok(ChatTurnJourney::Flow2InputAndApproval);
    }

    if fixture_id.starts_with("case.session-resume-latest")
        || fixture_id.starts_with("case.session-no-sessions-open")
        || fixture_id.starts_with("case.session-") && fixture_id.contains("after.chat-turn")
        || fixture_id.starts_with("case.fork-confirm")
        || fixture_id.contains("fork-confirm") && fixture_id.contains("chat-turn")
    {
        return Ok(ChatTurnJourney::Flow3SessionReturn);
    }

    if work_style_for_fixture(fixture_id).is_some()
        || fixture_id.starts_with("case.work-style-")
        || fixture_id.starts_with("case.skills-")
        || fixture_id.starts_with("case.subagent-")
        || fixture_id.starts_with("case.variant-")
        || fixture_id.starts_with("case.theme-")
    {
        return Ok(ChatTurnJourney::Flow3WorkStyleReturn);
    }

    // Recovery returns: gate fixture, explicit rate-limit/checkpoint paths, and
    // the deterministic automatic-retry case that restores the paused recovery turn.
    if fixture_id == "gate.recovery.chat-turn"
        || fixture_id.starts_with("case.rate-limit-")
        || fixture_id.starts_with("case.checkpoint-")
        || fixture_id.starts_with("case.turn-error-retry")
        || fixture_id.starts_with("case.chat-turn-open-checkpoint-undo")
        || fixture_id.starts_with("case.network-backoff-race-retry")
        || fixture_id.starts_with("case.automatic-retry")
        || fixture_id.starts_with("case.network-")
            && fixture_id.contains("chat-turn")
        || fixture_id.starts_with("flow5.")
        || fixture_id.starts_with("gate.recovery.")
    {
        return Ok(ChatTurnJourney::Flow5Recovery);
    }

    if fixture_id.contains("mission.cancel")
        || fixture_id.contains("mission.complete")
        || fixture_id.contains("mission.launch-cancel")
        || fixture_id.contains("mission.blocked-cancel")
        || fixture_id.contains("mission.approve")
        || fixture_id.starts_with("case.mission-")
        || fixture_id.starts_with("flow6.")
        || fixture_id.starts_with("gate.mission.")
    {
        return Ok(ChatTurnJourney::Flow6MissionReturn);
    }

    if fixture_id.starts_with("case.tool-running-interrupt")
        || fixture_id.starts_with("case.queued-interrupt-tool")
    {
        return Ok(ChatTurnJourney::ToolInterrupt);
    }

    if fixture_id.starts_with("case.streaming-interrupt")
        || fixture_id.starts_with("case.queued-interrupt-streaming")
    {
        return Ok(ChatTurnJourney::StreamingInterrupt);
    }

    // Daily work and explicit daily-origin return cases from configuration /
    // provider / tool / onboarding cancel paths. These remain Daily only when
    // the fixture string itself is declared for that journey (not silent default).
    if matches!(
        fixture_id,
        "flow1.daily-work.stable"
            | "canonical.chat-turn"
            | "conversation-completed"
            | "public-audit"
    ) || fixture_id.starts_with("flow1-storyboard.")
        || fixture_id.starts_with("flow1.")
        || fixture_id.starts_with("case.tool-running-complete")
        || fixture_id.starts_with("case.tool-success-continue")
        || fixture_id.starts_with("case.model-picker-")
        || fixture_id.starts_with("case.onboarding-existing-cancel")
        || fixture_id.starts_with("case.onboarding-")
            && fixture_id.contains("chat-turn")
        || fixture_id.starts_with("case.mcp-return")
        || fixture_id.starts_with("case.connect-connected-return")
        || fixture_id.starts_with("case.login-cancel")
        || fixture_id.starts_with("case.agent-question-submit")
        || fixture_id.starts_with("case.transcript-")
        || fixture_id.starts_with("case.queued-interrupt-shell")
    {
        return Ok(ChatTurnJourney::Flow1Daily);
    }

    // Hostile / misbound fixtures must not silently become Daily work.
    Err(format!(
        "chat-turn fixture is not bound to a known journey identity (fail-closed): {fixture_id}"
    ))
}

fn chat_turn_origin_and_task(journey: ChatTurnJourney) -> (&'static str, &'static str) {
    match journey {
        ChatTurnJourney::Flow1Daily
        | ChatTurnJourney::ToolInterrupt
        | ChatTurnJourney::StreamingInterrupt => (DAILY_WORK_ORIGIN, DAILY_WORK_TASK),
        ChatTurnJourney::Flow2InputAndApproval => (FLOW2_ORIGIN, FLOW2_TASK),
        ChatTurnJourney::Flow3SessionReturn => (FLOW3_CURRENT_ORIGIN, FLOW3_RESUME_TASK),
        ChatTurnJourney::Flow3WorkStyleReturn => (
            FLOW3_CURRENT_ORIGIN,
            "Choose a model and how Code should work.",
        ),
        ChatTurnJourney::Flow5Recovery => (FLOW5_ORIGIN, FLOW5_TASK),
        ChatTurnJourney::Flow6MissionReturn => (FLOW6_ORIGIN, FLOW6_TASK),
    }
}

fn chat_turn_conversation_identity(
    journey: ChatTurnJourney,
    _fixture_id: &str,
) -> crate::audit::ConversationIdentity {
    match journey {
        ChatTurnJourney::Flow2InputAndApproval => crate::audit::ConversationIdentity {
            conversation_id: "conversation.retry-review".to_owned(),
            transcript_id: "transcript.retry-review".to_owned(),
            session_id: "session.retry-review.18".to_owned(),
            turn_id: "turn.retry-review.18".to_owned(),
            user_task: FLOW2_TASK.to_owned(),
            visible_origin: FLOW2_ORIGIN.to_owned(),
            submitted_prompt: Some(FLOW2_SUBMITTED_PROMPT.to_owned()),
            paused_activity_id: Some(FLOW2_ACTIVITY_ID.to_owned()),
            later_draft: Some(FLOW2_LATER_DRAFT.to_owned()),
        },
        ChatTurnJourney::Flow3SessionReturn => crate::audit::ConversationIdentity {
            conversation_id: "conversation.configuration".to_owned(),
            transcript_id: "transcript.configuration".to_owned(),
            session_id: "session.retry-policy-review".to_owned(),
            turn_id: "turn.configuration.1".to_owned(),
            user_task: FLOW3_RESUME_TASK.to_owned(),
            visible_origin: FLOW3_CURRENT_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: Some("activity.retry-policy-review.latest".to_owned()),
            later_draft: Some("Add the final review note.".to_owned()),
        },
        ChatTurnJourney::Flow3WorkStyleReturn => crate::audit::ConversationIdentity {
            conversation_id: "conversation.configuration".to_owned(),
            transcript_id: "transcript.configuration".to_owned(),
            session_id: "session.configuration.1".to_owned(),
            turn_id: "turn.configuration.1".to_owned(),
            user_task: "Choose a model and how Code should work.".to_owned(),
            visible_origin: FLOW3_CURRENT_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: None,
            later_draft: Some("Continue the retry conversation.".to_owned()),
        },
        ChatTurnJourney::Flow5Recovery => crate::audit::ConversationIdentity {
            conversation_id: "conversation.recovery".to_owned(),
            transcript_id: "transcript.recovery".to_owned(),
            session_id: "session.recovery.1".to_owned(),
            turn_id: "turn.recovery.1".to_owned(),
            user_task: FLOW5_TASK.to_owned(),
            visible_origin: FLOW5_ORIGIN.to_owned(),
            submitted_prompt: Some(
                "Summarize the remaining release notes and the still-open risks.".to_owned(),
            ),
            paused_activity_id: Some("pending.network.release-notes".to_owned()),
            later_draft: Some(FLOW5_DRAFT.to_owned()),
        },
        ChatTurnJourney::Flow6MissionReturn => crate::audit::ConversationIdentity {
            conversation_id: "conversation.mission".to_owned(),
            transcript_id: "transcript.mission".to_owned(),
            session_id: "session.mission.1".to_owned(),
            turn_id: "turn.mission.1".to_owned(),
            user_task: FLOW6_TASK.to_owned(),
            visible_origin: FLOW6_ORIGIN.to_owned(),
            submitted_prompt: Some(
                "Turn the release work into a mission and walk me through it.".to_owned(),
            ),
            paused_activity_id: Some("pending.mission.launch".to_owned()),
            later_draft: Some("Keep the risk note short before we publish.".to_owned()),
        },
        ChatTurnJourney::Flow1Daily
        | ChatTurnJourney::ToolInterrupt
        | ChatTurnJourney::StreamingInterrupt => crate::audit::ConversationIdentity {
            conversation_id: "conversation.daily-work".to_owned(),
            transcript_id: "transcript.daily-work".to_owned(),
            session_id: "session.daily-work.1".to_owned(),
            turn_id: "turn.daily-work.18".to_owned(),
            user_task: DAILY_WORK_TASK.to_owned(),
            visible_origin: DAILY_WORK_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: None,
            later_draft: None,
        },
    }
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let mut canvas = Canvas::new();
    let work_style_return = work_style_for_fixture(fixture_id);

    // For screens that share chat-turn, bind origin/task before content so a
    // misclassified fixture cannot paint Daily work chrome over another journey.
    let chat_turn_journey = if screen_id == "chat-turn" {
        Some(classify_chat_turn_fixture(fixture_id)?)
    } else {
        None
    };
    let (visible_origin, user_task) = match chat_turn_journey {
        Some(journey) => chat_turn_origin_and_task(journey),
        None => match screen_id {
            "tool-task" | "tool-task-expanded" => (
                DAILY_WORK_ORIGIN,
                "Audit the auth module and add the rate-limit tests.",
            ),
            "todo-list" => (
                DAILY_WORK_ORIGIN,
                "Implement rate limiting on the login endpoint.",
            ),
            "shell" => (
                DAILY_WORK_ORIGIN,
                "Check the branch, then run the focused tests.",
            ),
            "compact" => (
                DAILY_WORK_ORIGIN,
                "Continue from the compacted conversation summary.",
            ),
            "chat-long-scroll" => chat_long_scroll_origin_and_task(fixture_id),
            // Other non-chat-turn conversation screens stay on the daily journey frame.
            _ => (DAILY_WORK_ORIGIN, DAILY_WORK_TASK),
        },
    };
    canvas.write(2, 4, visible_origin, TextStyle::muted())?;
    canvas.write(3, 4, user_task, TextStyle::bright())?;

    let (candidates, events, transitions) = match screen_id {
        "home" => render_home(&mut canvas, screen_id, fixture_id)?,
        "chat-turn" => match chat_turn_journey.expect("chat-turn journey classified") {
            ChatTurnJourney::StreamingInterrupt
                if fixture_id.starts_with("case.queued-interrupt-streaming") =>
            {
                render_chat_turn_interaction(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::ToolInterrupt
                if fixture_id.starts_with("case.queued-interrupt-tool") =>
            {
                render_chat_turn_interaction(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::ToolInterrupt => {
                render_tool_interrupted_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::StreamingInterrupt => {
                render_interrupted_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-persistent-scope") =>
            {
                render_permission_persistent_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-shell-unsafe-byte") =>
            {
                render_permission_unsafe_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-cancel")
                    || fixture_id.starts_with("case.permission-file-edit-cancel") =>
            {
                render_permission_cancel_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-deny-send-reason")
                    || fixture_id.starts_with("case.permission-deny-skip-reason") =>
            {
                render_permission_deny_reason_result(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-file-edit-allow-once") =>
            {
                render_diff_applied_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.permission-file-edit-persistent-scope")
                    && !fixture_id.contains("outside") =>
            {
                render_permission_file_edit_applied_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.staging-finish")
                    || fixture_id.starts_with("case.diff-apply-all")
                    || fixture_id.starts_with("case.staging-apply") =>
            {
                render_diff_applied_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if fixture_id.starts_with("case.diff-cancel") =>
            {
                render_diff_cancel_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval
                if matches!(
                    fixture_id,
                    "flow2.input-and-approval.stable"
                        | "case.staging-finish.tick.1.flow2.input-and-approval.stable"
                ) =>
            {
                render_flow2_resumed_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow2InputAndApproval => {
                // Remaining flow2-storyboard / flow2.* aliases use the default
                // resumed anatomy while keeping Input-and-approval identity.
                render_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow3SessionReturn => {
                render_session_resumed_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow3WorkStyleReturn
                if fixture_id.starts_with("case.skills-")
                    || fixture_id.starts_with("case.subagent-")
                    || fixture_id.starts_with("case.variant-")
                    || fixture_id.starts_with("case.theme-") =>
            {
                render_config_picker_returned_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow3WorkStyleReturn => {
                render_work_style_returned_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow5Recovery => {
                render_recovery_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow6MissionReturn => {
                render_mission_return_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow1Daily if fixture_id == "conversation-completed" => {
                render_daily_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow1Daily if chat_turn_interaction_mode(fixture_id) != "baseline" => {
                render_chat_turn_interaction(&mut canvas, screen_id, fixture_id)?
            }
            ChatTurnJourney::Flow1Daily => {
                render_chat_turn(&mut canvas, screen_id, fixture_id)?
            }
        },
        "streaming" => {
            if streaming_mode(fixture_id) != "baseline" {
                render_streaming_interaction(&mut canvas, screen_id, fixture_id)?
            } else {
                render_streaming(&mut canvas, screen_id, fixture_id)?
            }
        }
        "reasoning-collapsed" => render_reasoning_collapsed(&mut canvas, screen_id, fixture_id)?,
        "reasoning" => render_reasoning(&mut canvas, screen_id, fixture_id)?,
        "tool-running" => render_tool_running(&mut canvas, screen_id, fixture_id)?,
        "tool-success" => render_tool_success(&mut canvas, screen_id, fixture_id)?,
        "tool-success-expanded" => {
            render_tool_success_expanded(&mut canvas, screen_id, fixture_id)?
        }
        "tool-calls" => render_tool_calls(&mut canvas, screen_id, fixture_id)?,
        "tool-task" | "tool-task-expanded" if tool_task_mode(fixture_id) != "baseline" => {
            render_tool_task_controls(&mut canvas, screen_id, fixture_id)?
        }
        "tool-task" => render_tool_task(&mut canvas, screen_id, fixture_id)?,
        "tool-task-expanded" => render_tool_task_expanded(&mut canvas, screen_id, fixture_id)?,
        "rich-response" => render_rich_response(&mut canvas, screen_id, fixture_id)?,
        "shell" => render_shell(&mut canvas, screen_id, fixture_id)?,
        "todo-list" => render_todo_list(&mut canvas, screen_id, fixture_id)?,
        "compact" => render_compact(&mut canvas, screen_id, fixture_id)?,
        "chat-long-scroll" => render_chat_long_scroll(&mut canvas, screen_id, fixture_id)?,
        _ => return Err(format!("unknown conversation pilot screen: {screen_id}")),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    let conversation = match chat_turn_journey {
        Some(journey) => Some(chat_turn_conversation_identity(journey, fixture_id)),
        None => Some(crate::audit::ConversationIdentity {
            conversation_id: "conversation.daily-work".to_owned(),
            transcript_id: "transcript.daily-work".to_owned(),
            session_id: "session.daily-work.1".to_owned(),
            turn_id: match screen_id {
                "tool-task" | "tool-task-expanded" => "turn.daily-work.task-audit.22",
                "todo-list" => "turn.daily-work.todo.20",
                "shell" => "turn.daily-work.shell.21",
                "compact" => "turn.daily-work.compact.19",
                "chat-long-scroll" => chat_long_scroll_turn_id(fixture_id),
                _ => "turn.daily-work.18",
            }
            .to_owned(),
            user_task: user_task.to_owned(),
            visible_origin: visible_origin.to_owned(),
            submitted_prompt: None,
            paused_activity_id: None,
            later_draft: None,
        }),
    };

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: work_style_return,
        model_choice: None,
        conversation,
    })
}

fn work_style_for_fixture(fixture_id: &str) -> Option<WorkStyleEvidence> {
    if !fixture_id.starts_with("flow3-work-style") {
        return None;
    }
    let (applied, access_scope) = if fixture_id.contains("unchanged") {
        ("Quick implementation", "asks before editing project files")
    } else if fixture_id.contains("refused-return") {
        ("Careful edits", "asks before editing project files")
    } else {
        (
            "Autonomous changes",
            "can edit project files without asking each time, in this project",
        )
    };
    Some(WorkStyleEvidence {
        applied: applied.to_owned(),
        pending: None,
        access_scope: access_scope.to_owned(),
    })
}

fn render_config_picker_returned_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let (title, summary, detail) = if fixture_id.contains("skills-apply")
        || fixture_id.contains("skills-access-confirm")
    {
        (
            "Code · Session skills updated",
            "Applied skills · Commit cleanly, Design reviews, Review images",
            if fixture_id.contains("access-confirm") {
                "Research the web is on for this project · access was confirmed."
            } else {
                "The pending skill set was applied once and is now active."
            },
        )
    } else if fixture_id.contains("skills-cancel") {
        (
            "Code · Session skills unchanged",
            "Applied skills · prior set restored",
            "Cancel discarded the pending skill toggles without changing access.",
        )
    } else if fixture_id.contains("subagent-apply") {
        (
            "Code · Task helper started",
            "Helper · Explore auth boundaries · this task only",
            "Delegation stays task-local. Work style stays Careful edits.",
        )
    } else if fixture_id.contains("subagent-cancel") {
        (
            "Code · Task helper unchanged",
            "No helper delegated for this task.",
            "Cancel returned to the paused conversation without starting a helper.",
        )
    } else if fixture_id.contains("variant-apply") {
        (
            "Code · Response variant updated",
            "Applied variant · Thorough on Claude Opus 4.8",
            "Only the response variant changed. The base model and work style stay the same.",
        )
    } else if fixture_id.contains("variant-cancel") {
        (
            "Code · Response variant unchanged",
            "Applied variant · Balanced on Claude Opus 4.8",
            "Cancel kept the prior variant and base model relationship.",
        )
    } else if fixture_id.contains("theme-apply") {
        (
            "Code · Theme updated",
            "Applied theme · Light",
            "The temporary preview was committed. Non-theme content stays unchanged.",
        )
    } else if fixture_id.contains("theme-cancel") {
        (
            "Code · Theme unchanged",
            "Applied theme · Dark",
            "Cancel restored the previously applied palette and discarded the preview.",
        )
    } else {
        return Err(format!(
            "unknown config picker return fixture: {fixture_id}"
        ));
    };
    canvas.write(5, 4, title, TextStyle::bright())?;
    canvas.write(8, 4, summary, TextStyle::bright())?;
    canvas.write(10, 4, detail, TextStyle::text())?;
    canvas.write(
        13,
        4,
        "Your transcript and draft are unchanged.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "Continue the paused conversation when you are ready.",
        TextStyle::muted(),
    )?;
    composer(canvas, "Continue the retry conversation.", Some("Send reply"))?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "configuration-result",
            "picker-return",
            reference(5, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "applied-summary",
            "picker-summary",
            reference(8, 4, 70),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "origin-preserved",
            reference(13, 4, 46),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.continue",
            reference(34, 4, 24),
            "focused",
            Some(action(
                "action.chat-turn.continue",
                "Send reply",
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
        vec![transition(
            "action.chat-turn.continue",
            "chat-turn",
            "chat-turn",
            "direct",
            "the returned configuration summary stays in the same paused conversation",
        )],
    ))
}

fn render_work_style_returned_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let work_style = work_style_for_fixture(fixture_id)
        .ok_or_else(|| "work-style return fixture needs applied configuration".to_owned())?;
    let unchanged = fixture_id.contains("refused-return");
    canvas.write(
        5,
        4,
        if unchanged {
            "Code · Work configuration unchanged"
        } else {
            "Code · Work configuration updated"
        },
        TextStyle::bright(),
    )?;
    canvas.write(
        8,
        4,
        &format!("Applied work style · {}", work_style.applied),
        TextStyle::bright(),
    )?;
    let access = if work_style.applied == "Autonomous changes" {
        "Access · Can edit project files without asking each time · this project"
    } else {
        "Access · Asks before editing project files"
    };
    canvas.write(10, 4, access, TextStyle::text())?;
    canvas.write(
        13,
        4,
        "Your transcript and draft are unchanged.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        if unchanged {
            "The rejected access change is closed. Careful edits remains the only applied choice."
        } else {
            "The pending selection is now applied. No other configuration changed."
        },
        TextStyle::text(),
    )?;
    canvas.write(
        20,
        4,
        "Continue the paused conversation when you are ready.",
        TextStyle::muted(),
    )?;
    composer(
        canvas,
        "Continue the retry conversation.",
        Some("Send reply"),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "configuration-result",
            if unchanged {
                "work-style-unchanged"
            } else {
                "work-style-applied"
            },
            reference(5, 4, 42),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "applied-work-style",
            reference(8, 4, 61),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "access-scope",
            "applied-access-scope",
            reference(10, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "conversation-and-draft",
            reference(13, 4, 45),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 40),
            "focused",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
        ),
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the preserved draft sends from the same conversation under the applied work style",
        )],
    ))
}

fn render_flow2_resumed_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    // Keep the continuous You-phrase (same as banner task), then the immutable body.
    canvas.write(5, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(
        7,
        4,
        "Check the retry path in @src/net/retry_policy.rs",
        TextStyle::text(),
    )?;
    canvas.write(
        8,
        4,
        "Then explain the failure path that still needs coverage before the merge;",
        TextStyle::text(),
    )?;
    canvas.write(
        9,
        6,
        "include a concise test plan with the visible recovery behavior.",
        TextStyle::text(),
    )?;
    canvas.write(
        10,
        4,
        "Keep the summary short before we merge.",
        TextStyle::text(),
    )?;
    canvas.write(13, 4, "Code · Same turn resumed", TextStyle::bright())?;
    canvas.write(
        15,
        4,
        "Applied 1 included hunk to src/net/retry_policy.rs.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "One review hunk remains pending · Git index unchanged",
        TextStyle::text(),
    )?;
    // Product surface uses plain English; exact session/turn/activity IDs stay in audit JSON.
    canvas.write(18, 4, FLOW2_PAUSED_ACTIVITY_PLAIN, TextStyle::muted())?;
    canvas.write(19, 4, FLOW2_SESSION_PLAIN, TextStyle::muted())?;
    canvas.write(20, 4, FLOW2_TURN_PLAIN, TextStyle::muted())?;
    canvas.write(23, 4, "Later draft · not sent", TextStyle::bright())?;
    canvas.write(25, 4, FLOW2_LATER_DRAFT, TextStyle::text())?;
    composer(canvas, FLOW2_LATER_DRAFT, Some("Send reply"))?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "immutable-submitted-prompt",
            reference(5, 4, FLOW2_YOU_LINE.chars().count() as u16),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "resume",
            "action.chat-turn.resume-request",
            reference(13, 4, 28),
            "summary",
            Some(action(
                "action.chat-turn.resume-request",
                "Resume the paused conversation",
                true,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "activity",
            FLOW2_ACTIVITY_ID,
            reference(18, 4, FLOW2_PAUSED_ACTIVITY_PLAIN.chars().count() as u16 + 4),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "session",
            "session.retry-review.18",
            reference(19, 4, FLOW2_SESSION_PLAIN.chars().count() as u16 + 4),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "turn",
            "turn.retry-review.18",
            reference(20, 4, FLOW2_TURN_PLAIN.chars().count() as u16 + 4),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "later-unsent-draft",
            reference(23, 4, 26),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 62),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
                "action.chat-turn.resume-request",
                "chat-turn",
                "chat-turn",
                "summarized",
                "the exact paused activity resumes in the same turn with the submitted prompt immutable and the later draft unsent",
            ),
            transition(
                "action.chat-turn.send-reply",
                "chat-turn",
                "streaming",
                "direct",
                "the later draft begins a new response without changing the completed request",
            ),
        ],
    ))
}

fn render_diff_applied_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let diff = diff_evidence_for_fixture(fixture_id)
        .ok_or_else(|| "applied diff result is missing exact file evidence".to_owned())?;
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        &format!(
            "Applied {} {} to src/net/retry_policy.rs.",
            diff.applied_hunks.len(),
            if diff.applied_hunks.len() == 1 {
                "hunk"
            } else {
                "hunks"
            }
        ),
        TextStyle::text(),
    )?;
    if diff.excluded_hunks.is_empty() {
        canvas.write(11, 4, "No hunks remain pending.", TextStyle::text())?;
    } else {
        canvas.write(
            11,
            4,
            &format!(
                "{} remains pending and was not written.",
                diff.excluded_hunks.join(", ")
            ),
            TextStyle::text(),
        )?;
    }
    canvas.write(
        13,
        4,
        "The paused activity resumed with the same patch revision.",
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        "Working tree updated once · Git index unchanged",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        &format!("Before SHA-256: {}", diff.working_tree_before),
        TextStyle::muted(),
    )?;
    canvas.write(
        18,
        4,
        &format!("After SHA-256: {}", diff.working_tree_after),
        TextStyle::muted(),
    )?;
    canvas.write(20, 4, FLOW2_SESSION_TURN_PLAIN, TextStyle::muted())?;
    canvas.write(
        22,
        4,
        if diff.excluded_hunks.is_empty() {
            "Later draft · Add a short note about the completed retry changes."
        } else {
            "Later draft · Add a short note about the remaining test hunk."
        },
        TextStyle::muted(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "applied-hunk-result",
            reference(9, 4, 53),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            if diff.excluded_hunks.is_empty() {
                "applied-hunks"
            } else {
                "pending-hunk"
            },
            &if diff.excluded_hunks.is_empty() {
                "no-pending-hunks".to_owned()
            } else {
                diff.excluded_hunks.join(",")
            },
            reference(11, 4, 53),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "session",
            "session.retry-review.18",
            reference(20, 4, FLOW2_SESSION_TURN_PLAIN.chars().count() as u16 + 4),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the accurate edit result remains inline before the next response",
        )],
    ))
}

fn render_diff_cancel_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    permission_outcome_transcript(
        canvas,
        "Pending edit unchanged",
        "Cancel wrote nothing and returned to the paused conversation.",
        "Patch revision patch-retry-18 remains pending for a later decision.",
        "Your draft and transcript are unchanged.",
        "No file or Git index change was made.",
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "pending-edit-cancelled",
            reference(9, 4, 72),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-edit",
            "patch-retry-18",
            reference(11, 4, 68),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the unchanged paused conversation remains available",
        )],
    ))
}

fn render_home(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(6, 4, "Recent work", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "You traced the login timeout and added a safer session check.",
        TextStyle::text(),
    )?;
    canvas.write(
        10,
        4,
        "The session check is ready to continue from here.",
        TextStyle::muted(),
    )?;
    composer(canvas, "Ask Code to continue", None)?;

    let surface = semantic(
        screen_id,
        fixture_id,
        "surface",
        "conversation-shell",
        reference(6, 4, 15),
        "summary",
        None,
    );
    let recent_work = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "recent-work",
        reference(8, 4, 78),
        "summary",
        None,
    );
    let branding = semantic(
        screen_id,
        fixture_id,
        "entry-brand",
        "absent",
        reference(6, 4, 15),
        "summary",
        None,
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.home.start-request",
        reference(COMPOSER_TOP, 4, 37),
        "summary",
        Some(action(
            "action.home.start-request",
            "Continue conversation",
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
    let pairs = [surface, recent_work, branding, composer, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.home.start-request",
            "home",
            "chat-turn",
            "direct",
            "submitting the composer resumes the current conversation",
        )],
    ))
}

fn render_permission_persistent_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    permission_outcome_transcript(
        canvas,
        "Permission saved for this project",
        "The command ran once, then saved one bounded rule for this session.",
        "Matcher: git diff -- src/net/* · duration: this session",
        "Project root: ~/project · work style: Careful edits",
        "Children inherit only read-only checks · cap: same project root",
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "permission-persistent-result",
            reference(9, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "persistent-scope",
            "exact-project-session-scope",
            reference(13, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the scoped permission result returns to the same conversation",
        )],
    ))
}

fn render_permission_unsafe_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    permission_outcome_transcript(
        canvas,
        "Allowed once · unsafe bytes were escaped",
        "The exact escaped command ran one time; no reusable rule was saved.",
        "Command bytes: printf 'retry\\x00'\\x0a\\x1b[31m",
        "Working directory: ~/project · scope: this command only",
        "SHA-256: 000b9fc6d315b99043535ebc08b2c29dc3596f7583ef7ed8b1c8a4afd13a09fb",
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "unsafe-command-result",
            reference(9, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "escaped-command",
            "unsafe-byte-notation",
            reference(13, 4, 72),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the escaped command result returns to the same conversation",
        )],
    ))
}

fn render_permission_cancel_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let file_edit = fixture_id.contains("file-edit");
    permission_outcome_transcript(
        canvas,
        "Permission request still pending",
        if file_edit {
            "Cancel kept the request open. The file write did not run and nothing was denied."
        } else {
            "Cancel kept the request open. The command was not run and nothing was denied."
        },
        "Return to the paused turn when you are ready to choose.",
        if file_edit {
            "Target: src/net/retry_policy.rs · scope: this edit only"
        } else {
            "Working directory: ~/project · scope: this command only"
        },
        "Draft and transcript are unchanged",
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "cancel-result",
            reference(9, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-permission",
            "request-remains-pending",
            reference(11, 4, 74),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the unchanged paused turn can continue after the pending permission choice",
        )],
    ))
}

fn render_permission_deny_reason_result(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let sent = fixture_id.contains("send-reason");
    permission_outcome_transcript(
        canvas,
        if sent {
            "Denial reason sent"
        } else {
            "Denial reason skipped"
        },
        if sent {
            "Denied · command was not run · reason added to the denied activity once."
        } else {
            "Denied · command was not run · no reason was invented or uploaded."
        },
        if sent {
            "Reason: timeout risk on this shell check."
        } else {
            "Skip restored the same paused transcript without leaving reason text."
        },
        "Draft and origin match the moment the permission dock opened.",
        "No invocation occurred and Allow/Deny are no longer active.",
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            if sent {
                "denial-reason-sent"
            } else {
                "denial-reason-skipped"
            },
            reference(9, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "denial-reason",
            if sent {
                "timeout-risk"
            } else {
                "no-reason"
            },
            reference(13, 4, 72),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "send and skip each resume the same paused conversation once",
        )],
    ))
}

fn render_permission_file_edit_applied_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let persistent = fixture_id.contains("persistent-scope")
        && !fixture_id.contains("outside");
    if fixture_id.contains("allow-once") {
        // Diff application evidence owns the allow-once destination anatomy.
        return render_diff_applied_turn(canvas, screen_id, fixture_id);
    }
    permission_outcome_transcript(
        canvas,
        if persistent {
            "Permission saved for this file edit"
        } else {
            "File edit applied once"
        },
        if persistent {
            "The disclosed patch wrote once and one bounded rule remains for this session."
        } else {
            "The disclosed patch was written exactly once to src/net/retry_policy.rs."
        },
        if persistent {
            "Matcher: src/net/retry_policy.rs · duration: this session"
        } else {
            "Working tree updated once · Git index unchanged"
        },
        if persistent {
            "Project root: ~/project · work style: Careful edits"
        } else {
            "Draft and paused activity restored with the completed write"
        },
        if persistent {
            "Children inherit only read-only checks · cap: no write elevation"
        } else {
            "No reusable permission was stored for other files"
        },
        "Reply to Code",
    )?;
    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            if persistent {
                "file-edit-persistent-result"
            } else {
                "file-edit-applied-once"
            },
            reference(9, 4, 78),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            if persistent {
                "persistent-scope"
            } else {
                "applied-edit"
            },
            if persistent {
                "exact-file-session-scope"
            } else {
                "src/net/retry_policy.rs"
            },
            reference(13, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the file-edit outcome returns to the same paused conversation",
        )],
    ))
}

fn permission_outcome_transcript(
    canvas: &mut Canvas,
    heading: &str,
    result: &str,
    detail_one: &str,
    detail_two: &str,
    detail_three: &str,
    composer_label: &str,
) -> Result<(), String> {
    canvas.write(4, 4, FLOW2_YOU_LINE, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, heading, TextStyle::text())?;
    canvas.write(11, 4, result, TextStyle::text())?;
    canvas.write(13, 4, detail_one, TextStyle::muted())?;
    canvas.write(15, 4, detail_two, TextStyle::muted())?;
    canvas.write(17, 4, detail_three, TextStyle::muted())?;
    composer(canvas, composer_label, Some("Send reply"))?;
    Ok(())
}

fn render_mission_return_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let completed = fixture_id.contains("mission.complete") || fixture_id.contains("approve");
    canvas.write(
        5,
        4,
        "You · Turn the release work into a mission and walk me through it.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "I can run that as a small mission with three connected tasks.",
        TextStyle::text(),
    )?;
    if completed {
        canvas.write(
            10,
            4,
            "Mission complete · release is ready for the next conversation step.",
            TextStyle::bright(),
        )?;
        canvas.write(
            12,
            4,
            "A concise result summary stays in the transcript. Details stay available on request.",
            TextStyle::text(),
        )?;
        canvas.write(
            14,
            4,
            "[d] Inspect mission details",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "The normal composer is restored. Continue whenever you are ready.",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            10,
            4,
            "Mission planning stayed in this conversation. Nothing started while cancelled.",
            TextStyle::text(),
        )?;
        canvas.write(
            12,
            4,
            "Your draft and mission goal remain available.",
            TextStyle::muted(),
        )?;
    }
    composer(
        canvas,
        "Keep the risk note short before we publish.",
        Some("Send reply"),
    )?;
    canvas.write(
        COMPOSER_TOP + 2,
        6,
        "Keep the risk note short before we publish.",
        TextStyle::text(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "mission-conversation",
            reference(2, 4, 34),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            if completed {
                "mission-complete-summary"
            } else {
                "mission-cancel-summary"
            },
            reference(10, 4, 70),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "composer",
            "restored-composer",
            reference(COMPOSER_TOP + 1, 4, 48),
            "summary",
            Some(action("action.composer.send", "Send reply", false)),
        ),
    ];
    let transitions = vec![transition(
        "action.composer.send",
        screen_id,
        "streaming",
        "direct",
        "continuing the mission-origin conversation uses the restored composer",
    )];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn render_recovery_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(5, 4, FLOW5_USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, FLOW5_PARTIAL, TextStyle::text())?;
    canvas.write(11, 4, FLOW5_COMPLETED, TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "The provider attempt succeeded. Work stayed in this conversation.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "Draft, queue, and completed work are still available.",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "Ready for your next release-note prompt.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Checkpoint · Before risk-note expansion remains reversible.",
        TextStyle::muted(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;
    // Show the preserved later draft under the restored composer label.
    canvas.write(COMPOSER_TOP + 2, 6, FLOW5_DRAFT, TextStyle::text())?;
    canvas.write(
        COMPOSER_TOP + 3,
        6,
        "[u] Undo last checkpoint",
        TextStyle::muted(),
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "release-notes-prompt",
        reference(5, 4, 70),
        "summary",
        None,
    );
    let partial = semantic(
        screen_id,
        fixture_id,
        "partial-reply",
        "kept-answer",
        reference(9, 4, 95),
        "focused",
        None,
    );
    let resume = semantic(
        screen_id,
        fixture_id,
        "continuation",
        "recovery-restored",
        reference(13, 4, 68),
        "focused",
        None,
    );
    let composer_ready = semantic(
        screen_id,
        fixture_id,
        "composer",
        "restored-composer",
        reference(COMPOSER_TOP + 1, 4, 28),
        "summary",
        Some(action(
            "action.chat-turn.compose",
            "Reply to Code",
            false,
        )),
    );
    let undo = semantic(
        screen_id,
        fixture_id,
        "secondary-action",
        "action.chat-turn.open-checkpoint-undo",
        reference(COMPOSER_TOP + 3, 6, 28),
        "summary",
        Some(action(
            "action.chat-turn.open-checkpoint-undo",
            "Undo last checkpoint",
            false,
        )),
    );
    let statusline = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    Ok((
        vec![
            request.0,
            partial.0,
            resume.0,
            composer_ready.0,
            undo.0,
            statusline.0,
        ],
        vec![
            request.1,
            partial.1,
            resume.1,
            composer_ready.1,
            undo.1,
            statusline.1,
        ],
        vec![
            transition(
                "action.chat-turn.compose",
                "chat-turn",
                "chat-turn",
                "direct",
                "the restored conversation remains available for the next prompt",
            ),
            transition(
                "action.chat-turn.open-checkpoint-undo",
                "chat-turn",
                "checkpoint-undo",
                "direct",
                "the reversible checkpoint action opens beside the same recovered conversation",
            ),
        ],
    ))
}

fn render_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    // Daily-work gate 02 sits immediately before same-turn streaming (03). It must not
    // claim a terminal answer or "next prompt" handoff for the open rate-limit request.
    let open_turn_prestream = fixture_id == "flow1.daily-work.stable"
        || fixture_id.starts_with("flow1-storyboard.chat-turn");
    // Some Flow2 alias fixtures reuse this anatomy; their You-line must still match the
    // continuous pause-window task (no Daily rate-limit bleed).
    let flow2_resume = matches!(
        fixture_id,
        "flow2-storyboard.chat-turn" | "flow2-storyboard.resumed"
    ) || fixture_id.starts_with("flow2.");
    let user_line = if flow2_resume {
        FLOW2_YOU_LINE
    } else {
        daily_user_request(fixture_id)
    };
    canvas.write(4, 4, user_line, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;

    if open_turn_prestream {
        canvas.write(
            9,
            4,
            "Earlier · Updated retry_policy.rs and checked the current session.",
            TextStyle::muted(),
        )?;
        canvas.write(
            11,
            4,
            "That work is complete. The rate-limit request is still open.",
            TextStyle::text(),
        )?;
        canvas.write(
            14,
            4,
            "Send this request to start the reply in this same turn.",
            TextStyle::muted(),
        )?;
        canvas.write(
            17,
            4,
            "Checked  ·  retry_policy.rs  ·  current session",
            TextStyle::text(),
        )?;
        canvas.write(
            20,
            4,
            "Queued request · waiting for send.",
            TextStyle::muted(),
        )?;
        composer(canvas, "Reply to Code", Some("Send reply"))?;

        let request = semantic(
            screen_id,
            fixture_id,
            "transcript",
            "user-request",
            reference(4, 4, 50),
            "summary",
            None,
        );
        let prior_context = semantic(
            screen_id,
            fixture_id,
            "transcript",
            "prior-complete-work",
            reference(9, 4, 64),
            "summary",
            None,
        );
        let open_request = semantic(
            screen_id,
            fixture_id,
            "turn-state",
            "open-request-pending-send",
            reference(11, 4, 60),
            "focused",
            None,
        );
        let send_guidance = semantic(
            screen_id,
            fixture_id,
            "continuation",
            "send-to-start-reply",
            reference(14, 4, 61),
            "summary",
            None,
        );
        let checked = semantic(
            screen_id,
            fixture_id,
            "content",
            "checked-file-scope",
            reference(17, 4, 48),
            "summary",
            None,
        );
        let pending = semantic(
            screen_id,
            fixture_id,
            "turn-state",
            "queued-request-waiting",
            reference(20, 4, 37),
            "summary",
            None,
        );
        let composer = semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
        );
        let focus = semantic(
            screen_id,
            fixture_id,
            "focus",
            "conversation-input",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            None,
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
            request,
            prior_context,
            open_request,
            send_guidance,
            checked,
            pending,
            composer,
            focus,
            status,
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            vec![transition(
                "action.chat-turn.send-reply",
                "chat-turn",
                "streaming",
                "direct",
                "the open rate-limit request begins streaming in this same turn",
            )],
        ));
    }

    canvas.write(9, 4, "Updated retry_policy.rs.", TextStyle::text())?;
    canvas.write(
        11,
        4,
        "The retry check now verifies the current session before continuing.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "Older sessions are asked to sign in again after credentials change.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "The request is complete; a new draft stays separate.",
        TextStyle::muted(),
    )?;
    canvas.write(18, 4, "Result", TextStyle::bright())?;
    canvas.write(
        20,
        4,
        "retry_policy.rs now protects the current session without changing your draft.",
        TextStyle::muted(),
    )?;
    canvas.write(
        22,
        4,
        "No files outside this check were changed.",
        TextStyle::text(),
    )?;
    canvas.write(
        24,
        4,
        "You can ask a follow-up without reopening the completed request.",
        TextStyle::muted(),
    )?;
    canvas.write(
        26,
        4,
        "Checked  ·  retry_policy.rs  ·  current session",
        TextStyle::text(),
    )?;
    canvas.write(28, 4, "Ready for your next question.", TextStyle::muted())?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 50),
        "summary",
        flow2_resume.then(|| {
            action(
                "action.chat-turn.resume-request",
                "Resume the paused conversation",
                true,
            )
        }),
    );
    let continuation_status = semantic(
        screen_id,
        fixture_id,
        "continuation",
        "conversation-ready",
        reference(15, 4, 53),
        "focused",
        None,
    );
    let answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "retry-policy-result",
        reference(9, 4, 26),
        "summary",
        None,
    );
    let session_check = semantic(
        screen_id,
        fixture_id,
        "content",
        "session-check-explanation",
        reference(11, 4, 69),
        "summary",
        None,
    );
    let session_expiry = semantic(
        screen_id,
        fixture_id,
        "content",
        "session-expiry-explanation",
        reference(13, 4, 70),
        "summary",
        None,
    );
    let result_summary = semantic(
        screen_id,
        fixture_id,
        "content",
        "result-summary",
        reference(20, 4, 82),
        "focused",
        None,
    );
    let scope = semantic(
        screen_id,
        fixture_id,
        "content",
        "change-scope",
        reference(22, 4, 38),
        "summary",
        None,
    );
    let follow_up = semantic(
        screen_id,
        fixture_id,
        "continuation",
        "follow-up-ready",
        reference(24, 4, 67),
        "summary",
        None,
    );
    let checked = semantic(
        screen_id,
        fixture_id,
        "content",
        "checked-file-scope",
        reference(26, 4, 48),
        "summary",
        None,
    );
    let ready = semantic(
        screen_id,
        fixture_id,
        "continuation",
        "ready-for-follow-up",
        reference(28, 4, 28),
        "summary",
        None,
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.chat-turn.send-reply",
        reference(COMPOSER_TOP, 4, 30),
        "summary",
        Some(action("action.chat-turn.send-reply", "Send reply", false)),
    );
    let focus = semantic(
        screen_id,
        fixture_id,
        "focus",
        "conversation-input",
        reference(COMPOSER_TOP, 4, 30),
        "summary",
        None,
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
        request,
        continuation_status,
        answer,
        session_check,
        session_expiry,
        result_summary,
        scope,
        follow_up,
        checked,
        ready,
        composer,
        focus,
        status,
    ];
    let mut transitions = vec![transition(
        "action.chat-turn.send-reply",
        "chat-turn",
        "streaming",
        "direct",
        "the next prompt begins a new response in the same conversation",
    )];
    if flow2_resume {
        transitions.push(transition(
            "action.chat-turn.resume-request",
            "chat-turn",
            "chat-turn",
            "summarized",
            "the applied edit is summarized inline before the paused conversation continues",
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

fn render_session_resumed_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        6,
        4,
        "Resumed conversation · Retry policy review",
        TextStyle::bright(),
    )?;
    canvas.write(9, 4, "Latest stable work", TextStyle::bright())?;
    canvas.write(
        11,
        4,
        "Tests pass; the retry policy review is ready.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "Saved draft · Add the final review note.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "How Code works · Careful edits · asks before editing project files.",
        TextStyle::muted(),
    )?;
    canvas.write(
        20,
        4,
        "The source conversation and its history are unchanged.",
        TextStyle::muted(),
    )?;
    canvas.write(
        23,
        4,
        "Historical points still fork into a separate copy.",
        TextStyle::muted(),
    )?;
    composer(canvas, "Add the final review note.", Some("Send reply"))?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "session-result",
            "retry-policy-review-resumed",
            reference(6, 4, 49),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "resume-destination",
            "latest-stable-work",
            reference(9, 4, 22),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "selected-session-draft",
            reference(14, 4, 46),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "careful-edits",
            reference(17, 4, 72),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "source-history-unchanged",
            reference(20, 4, 59),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 32),
            "focused",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the resumed session sends from its restored draft in the selected conversation",
        )],
    ))
}

fn render_daily_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "A password change invalidates tokens issued before the change.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "The next request sees the newer session version and asks the person to sign in again.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "That keeps an older browser session from continuing after credentials changed.",
        TextStyle::text(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "user-request",
            reference(4, 4, 69),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "resume",
            "current-conversation",
            reference(4, 4, 69),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "completed-answer",
            reference(9, 4, 67),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-turn.send-reply",
            reference(COMPOSER_TOP, 4, 30),
            "summary",
            Some(action("action.chat-turn.send-reply", "Send reply", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "focus",
            "conversation-input",
            reference(COMPOSER_TOP, 4, 30),
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
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the next reply begins inline in this same transcript",
        )],
    ))
}

fn render_streaming(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The refactor moves token checks into one shared session guard.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "That keeps sign-in, refresh, and sign-out on the same safety rule.",
        TextStyle::text(),
    )?;
    let live = ActivityRow::new(
        "Writing reply",
        ActivityState::Running,
        "24s elapsed · 2 sections received",
    )
    .with_interrupt("Stop reply")
    .compact();
    write_component(canvas, 13, 4, &live)?;
    canvas.write(
        15,
        4,
        "When this reply is complete, Code will automatically search project files.",
        TextStyle::muted(),
    )?;
    composer(
        canvas,
        "Type a follow-up, it will wait until this reply finishes",
        None,
    )?;

    let partial_answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "partial-answer",
        reference(9, 4, 68),
        "summary",
        None,
    );
    let live_state = semantic(
        screen_id,
        fixture_id,
        "live-state",
        "active-response",
        reference(13, 4, 77),
        "summary",
        Some(action("action.streaming.interrupt", "Stop reply", false)),
    );
    let queued_composer = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(COMPOSER_TOP, 4, 70),
        "summary",
        None,
    );
    let tool_handoff = semantic(
        screen_id,
        fixture_id,
        "automatic-transition",
        "action.streaming.start-requested-tool",
        reference(15, 4, 82),
        "summary",
        Some(action(
            "action.streaming.start-requested-tool",
            "Start requested tool",
            true,
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
        partial_answer,
        live_state,
        queued_composer,
        tool_handoff,
        status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.streaming.interrupt",
                "streaming",
                "chat-turn",
                "direct",
                "stopping locally preserves the partial answer and queued follow-up",
            ),
            transition(
                "action.streaming.start-requested-tool",
                "streaming",
                "tool-running",
                "automatic",
                "the reply automatically starts the requested project search in this same transcript",
            ),
        ],
    ))
}

fn render_interrupted_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The refactor moves token checks into one shared session guard.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "It keeps sign-in, refresh, and sign-out on the same safety rule.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "● Reply stopped safely. The partial answer is saved.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "Queued follow-up: Also list the tests that should cover the change.",
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        4,
        "[attachment: auth-notes.md] remains queued with this follow-up.",
        TextStyle::muted(),
    )?;
    composer_with_draft(
        canvas,
        "Add a short note about the auth refactor.",
        "auth-notes.md",
        Some("Send reply"),
    )?;

    let partial_answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "interrupted-partial-answer",
        reference(9, 4, 68),
        "summary",
        None,
    );
    let queued_follow_up = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(17, 4, 73),
        "summary",
        None,
    );
    let draft = semantic(
        screen_id,
        fixture_id,
        "draft",
        "restored-draft",
        reference(COMPOSER_TOP + 1, 4, 43),
        "focused",
        None,
    );
    let attachment = semantic(
        screen_id,
        fixture_id,
        "attachment",
        "auth-notes.md",
        reference(COMPOSER_TOP + 2, 4, 33),
        "summary",
        None,
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.chat-turn.send-reply",
        reference(COMPOSER_TOP, 4, 30),
        "summary",
        Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        partial_answer,
        queued_follow_up,
        draft,
        attachment,
        composer,
        status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the saved conversation can continue with the restored composer",
        )],
    ))
}

fn render_tool_running(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I checked the session guard first. It stays complete while the search continues.",
        TextStyle::text(),
    )?;
    let completed = ActivityRow::new(
        "Checked session rules",
        ActivityState::Succeeded,
        "found the shared guard",
    )
    .with_elapsed("0.2s")
    .compact();
    write_component(canvas, 12, 4, &completed)?;
    let active = ActivityRow::new(
        "Search project code",
        ActivityState::Running,
        "128 of 400 files · 3 matches so far",
    )
    .with_elapsed("4s elapsed")
    .with_interrupt("Stop search")
    .compact();
    write_component(canvas, 15, 4, &active)?;
    composer(
        canvas,
        "Type a follow-up, it will wait until this search finishes",
        None,
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let completed_activity = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "completed-session-check",
        reference(12, 4, 75),
        "summary",
        None,
    );
    let active_activity = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-search",
        reference(15, 4, 109),
        "summary",
        Some(action(
            "action.tool-running.interrupt",
            "Stop search",
            false,
        )),
    );
    let queued_composer = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(COMPOSER_TOP, 4, 67),
        "summary",
        None,
    );
    let automatic_completion = semantic(
        screen_id,
        fixture_id,
        "completion",
        "tool-search",
        reference(15, 4, 109),
        "summary",
        Some(action(
            "action.tool-running.complete",
            "Search completed",
            true,
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
        request,
        completed_activity,
        active_activity,
        queued_composer,
        automatic_completion,
        status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.tool-running.interrupt",
                "tool-running",
                "chat-turn",
                "direct",
                "stopping this search preserves the completed check and queued follow-up",
            ),
            transition(
                "action.tool-running.complete",
                "tool-running",
                "tool-success",
                "automatic",
                "the one active search replaces its running row with one terminal result",
            ),
        ],
    ))
}

fn render_tool_success(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    // Gate / Flow 1 frames keep the compact pilot chrome. Focused expand detail
    // is available on dedicated disclosure fixtures so approved gate hashes stay stable.
    let disclosure_fixture = fixture_id.contains("disclosure")
        || fixture_id.contains("tool-success-expand")
        || fixture_id.contains("tool-success-collapse")
        || fixture_id.starts_with("case.tool-success-expand")
        || fixture_id.starts_with("case.tool-success-collapse");

    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The result shows the shared guard applied by both login and API routes.",
        TextStyle::text(),
    )?;
    let success = ActivityRow::new(
        "Search project code",
        ActivityState::Succeeded,
        "3 matches in 400 files",
    )
    .with_elapsed("0.4s")
    .compact();
    write_component(canvas, 12, 4, &success)?;
    // Same open turn continues into the completed answer; not a next-prompt handoff.
    canvas.write(
        15,
        4,
        "Tool finished · continue this same response with the full answer.",
        TextStyle::muted(),
    )?;
    if disclosure_fixture {
        canvas.write(
            17,
            4,
            "› Result details · 3 focused matches  [enter] Expand",
            TextStyle::text(),
        )?;
    }
    composer(canvas, "Reply to Code", Some("Continue response"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let result = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-search",
        reference(12, 4, 83),
        "summary",
        None,
    );
    let summary = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "result-summary",
        reference(9, 4, 76),
        "summary",
        None,
    );
    let continue_phase = semantic(
        screen_id,
        fixture_id,
        "turn-state",
        "continue-same-response",
        reference(15, 4, 66),
        "summary",
        None,
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.tool-success.continue",
        reference(COMPOSER_TOP, 4, 33),
        "summary",
        Some(action(
            "action.tool-success.continue",
            "Continue response",
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
    let mut candidates = vec![
        request.0.clone(),
        result.0.clone(),
        summary.0.clone(),
        continue_phase.0.clone(),
        composer.0.clone(),
        status.0.clone(),
    ];
    let mut events = vec![
        request.1.clone(),
        result.1.clone(),
        summary.1.clone(),
        continue_phase.1.clone(),
        composer.1.clone(),
        status.1.clone(),
    ];
    let mut transitions = vec![transition(
        "action.tool-success.continue",
        "tool-success",
        "rich-response",
        "direct",
        "the open turn continues from the tool result into the completed answer",
    )];
    if disclosure_fixture {
        let expand = semantic(
            screen_id,
            fixture_id,
            "disclosure",
            "tool-search-detail",
            reference(17, 4, 56),
            "summary",
            Some(action("action.tool-success.expand", "Expand", false)),
        );
        candidates.insert(4, expand.0);
        events.insert(4, expand.1);
        transitions.push(transition(
            "action.tool-success.expand",
            "tool-success",
            "tool-success-expanded",
            "direct",
            "focused matches open under the same completed search activity",
        ));
    }
    Ok((candidates, events, transitions))
}

fn render_rich_response(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The shared request guard applies the limit before route work begins.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "Both login and API routes reuse it, so the same safety rule stays in place.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "Where                 How the guard applies it",
        TextStyle::bright(),
    )?;
    canvas.write(
        15,
        4,
        "Login route           Stops an invalid request before handler work.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "API routes            Reuses the same check for authenticated requests.",
        TextStyle::text(),
    )?;
    canvas.write(19, 4, "Code", TextStyle::bright())?;
    canvas.write(
        20,
        6,
        "fn authorize_request(request: &Request) -> Result<Session, AuthError> {",
        TextStyle::text(),
    )?;
    canvas.write(
        21,
        6,
        "    let session = shared_guard(request)?;",
        TextStyle::text(),
    )?;
    canvas.write(22, 6, "    apply_rate_limit(&session)?;", TextStyle::text())?;
    canvas.write(23, 6, "    Ok(session)", TextStyle::text())?;
    canvas.write(24, 6, "}", TextStyle::text())?;
    canvas.write(26, 6, "[c] Copy code", TextStyle::accent())?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let prose = semantic(
        screen_id,
        fixture_id,
        "content",
        "answer-prose",
        reference(9, 4, 77),
        "summary",
        None,
    );
    let table = semantic(
        screen_id,
        fixture_id,
        "content",
        "route-summary-table",
        reference(14, 4, 55),
        "focused",
        None,
    );
    let code = semantic(
        screen_id,
        fixture_id,
        "content",
        "shared-guard-code",
        reference(20, 6, 84),
        "focused",
        None,
    );
    let copy = semantic(
        screen_id,
        fixture_id,
        "content-action",
        "action.rich-response.copy-code",
        reference(26, 6, 19),
        "focused",
        Some(action("action.rich-response.copy-code", "Copy code", false)),
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.rich-response.continue",
        reference(COMPOSER_TOP, 4, 29),
        "summary",
        Some(action("action.rich-response.continue", "Send reply", false)),
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
    let pairs = [request, prose, table, code, copy, composer, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.rich-response.continue",
            "rich-response",
            "chat-turn",
            "direct",
            "the completed answer keeps its composer available in the same conversation",
        )],
    ))
}

fn render_tool_interrupted_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    let completed = ActivityRow::new(
        "Checked session rules",
        ActivityState::Succeeded,
        "found the shared guard",
    )
    .with_elapsed("0.2s")
    .compact();
    write_component(canvas, 10, 4, &completed)?;
    canvas.write(
        13,
        4,
        "● Search stopped safely. Previous result and draft are saved.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "Queued follow-up: Also list the tests that should cover the search.",
        TextStyle::text(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let completed_activity = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "completed-session-check",
        reference(10, 4, 75),
        "summary",
        None,
    );
    let interrupted = semantic(
        screen_id,
        fixture_id,
        "recovery",
        "stopped-tool-search",
        reference(13, 4, 66),
        "summary",
        None,
    );
    let queued_follow_up = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(16, 4, 72),
        "summary",
        None,
    );
    let composer = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.chat-turn.send-reply",
        reference(COMPOSER_TOP, 4, 30),
        "summary",
        Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
        request,
        completed_activity,
        interrupted,
        queued_follow_up,
        composer,
        status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the same turn resumes with its preserved completed activity and draft",
        )],
    ))
}

fn render_reasoning_collapsed(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    let disclosure = DisclosureBlock::new(
        "Why this answer · checked the shared rate-limit guard path",
        DisclosureState::Collapsed,
    )
    .with_action_labels("Expand rationale", "Collapse");
    write_component(canvas, 9, 4, &disclosure)?;
    canvas.write(
        12,
        4,
        "The shared request guard applies the limit before route work begins.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "Login and API routes reuse it, so the same safety rule stays in place.",
        TextStyle::text(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let rationale = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "reasoning-rationale",
        reference(9, 4, 78),
        "summary",
        Some(action(
            "action.reasoning.expand",
            "Expand rationale",
            false,
        )),
    );
    let answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "final-answer",
        reference(12, 4, 76),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.reasoning.continue",
        reference(COMPOSER_TOP, 4, 29),
        "summary",
        Some(action("action.reasoning.continue", "Send reply", false)),
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
    let pairs = [request, rationale, answer, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.reasoning.expand",
            "reasoning-collapsed",
            "reasoning",
            "direct",
            "expanded rationale opens beside the same turn and answer",
        )],
    ))
}

fn render_reasoning(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    let disclosure = DisclosureBlock::new(
        "Why this answer · checked the shared rate-limit guard path",
        DisclosureState::Expanded,
    )
    .with_action_labels("Expand rationale", "Collapse")
    .with_expanded_lines([
        "Limit checks run once before any route handler.",
        "Login and API both call the same shared guard.",
        "That keeps one safety rule instead of per-route copies.",
    ]);
    write_component(canvas, 9, 4, &disclosure)?;
    canvas.write(
        15,
        4,
        "The shared request guard applies the limit before route work begins.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "Login and API routes reuse it, so the same safety rule stays in place.",
        TextStyle::text(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let rationale = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "reasoning-rationale",
        reference(9, 4, 74),
        "focused",
        Some(action(
            "action.reasoning.collapse",
            "Collapse",
            false,
        )),
    );
    let answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "final-answer",
        reference(15, 4, 76),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.reasoning.continue",
        reference(COMPOSER_TOP, 4, 29),
        "summary",
        Some(action("action.reasoning.continue", "Send reply", false)),
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
    let pairs = [request, rationale, answer, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.reasoning.collapse",
            "reasoning",
            "reasoning-collapsed",
            "direct",
            "collapse restores the same transcript position, answer, and draft",
        )],
    ))
}

fn render_tool_success_expanded(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "The result shows the shared guard applied by both login and API routes.",
        TextStyle::text(),
    )?;
    let success = ActivityRow::new(
        "Search project code",
        ActivityState::Expanded,
        "3 matches in 400 files",
    )
    .with_elapsed("0.4s")
    .compact();
    write_component(canvas, 12, 4, &success)?;
    canvas.write(
        14,
        4,
        "▾ Result details · 3 focused matches  [enter] Collapse",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "  middleware/rate_limit.rs:42 · shared enforce() entry",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "  routes/login.rs:88 · login quota path",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "  routes/api.rs:17 · API quota path",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Tool finished · continue this same response with the full answer.",
        TextStyle::muted(),
    )?;
    composer(canvas, "Reply to Code", Some("Continue response"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let result = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-search",
        reference(12, 4, 83),
        "summary",
        None,
    );
    let detail = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "tool-search-detail",
        reference(14, 4, 56),
        "focused",
        Some(action(
            "action.tool-success.collapse",
            "Collapse",
            false,
        )),
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.tool-success.continue",
        reference(COMPOSER_TOP, 4, 33),
        "summary",
        Some(action(
            "action.tool-success.continue",
            "Continue response",
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
    let pairs = [request, result, detail, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.tool-success.collapse",
            "tool-success-expanded",
            "tool-success",
            "direct",
            "collapse restores the compact terminal result without mutating matches",
        )],
    ))
}

fn render_tool_calls(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I am checking the shared rules, then the failing suite, while scanning code.",
        TextStyle::text(),
    )?;
    let completed = ActivityRow::new(
        "Read session rules",
        ActivityState::Succeeded,
        "found the shared guard",
    )
    .with_elapsed("0.2s")
    .compact();
    write_component(canvas, 12, 4, &completed)?;
    let failed = ActivityRow::new(
        "Run rate-limit tests",
        ActivityState::Failed,
        "2 of 38 tests failed",
    )
    .with_elapsed("41s")
    .compact();
    write_component(canvas, 14, 4, &failed)?;
    canvas.write(
        15,
        6,
        "Tests assert the wrong recovery message. [r] Retry tests",
        TextStyle::text(),
    )?;
    let running = ActivityRow::new(
        "Search project code",
        ActivityState::Running,
        "118 of 240 files · 1 match so far",
    )
    .with_elapsed("12s elapsed")
    .compact();
    write_component(canvas, 17, 4, &running)?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 57),
        "summary",
        None,
    );
    let a1 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-read-rules",
        reference(12, 4, 70),
        "summary",
        None,
    );
    let a2 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-run-tests",
        reference(14, 4, 66),
        "summary",
        None,
    );
    let recovery = semantic(
        screen_id,
        fixture_id,
        "recovery",
        "tool-run-tests",
        reference(15, 6, 62),
        "summary",
        Some(action(
            "action.tool-calls.retry-tests",
            "Retry tests",
            false,
        )),
    );
    let a3 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "tool-search",
        reference(17, 4, 84),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.tool-calls.continue",
        reference(COMPOSER_TOP, 4, 29),
        "summary",
        Some(action("action.tool-calls.continue", "Send reply", false)),
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
    let pairs = [request, a1, a2, recovery, a3, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.tool-calls.retry-tests",
            "tool-calls",
            "tool-running",
            "direct",
            "retry only restarts the failed tests while completed sibling results stay",
        )],
    ))
}

fn render_tool_task(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Audit the auth module and add the rate-limit tests.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I split the audit from the test work so findings can guide the patch.",
        TextStyle::text(),
    )?;
    let audit_task = ActivityRow::new(
        "Audit the auth module",
        ActivityState::Succeeded,
        "6 findings ready",
    )
    .with_elapsed("3m12s")
    .compact();
    write_component(canvas, 12, 4, &audit_task)?;
    canvas.write(
        13,
        4,
        "› Task details · expand findings  [enter] Expand",
        TextStyle::text(),
    )?;
    let tests = ActivityRow::new(
        "Add rate-limit tests",
        ActivityState::Running,
        "2 of 3 files · in progress",
    )
    .with_elapsed("48s elapsed")
    .compact();
    write_component(canvas, 15, 4, &tests)?;
    let queued = ActivityRow::new(
        "Build the test suite",
        ActivityState::Waiting,
        "queued after the test files",
    )
    .compact();
    write_component(canvas, 17, 4, &queued)?;
    let blocked = ActivityRow::new(
        "Review refresh session risk",
        ActivityState::Failed,
        "Blocked · needs a decision",
    )
    .compact();
    write_component(canvas, 19, 4, &blocked)?;
    composer(
        canvas,
        "Type a follow-up, it will wait until this turn finishes",
        None,
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 58),
        "summary",
        None,
    );
    let t1 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-audit-auth",
        reference(12, 4, 68),
        "summary",
        None,
    );
    let expand = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "task-audit-auth",
        reference(13, 4, 50),
        "summary",
        Some(action("action.tool-task.expand", "Expand", false)),
    );
    let t2 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-add-tests",
        reference(15, 4, 74),
        "summary",
        None,
    );
    let t3 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-build-suite",
        reference(17, 4, 70),
        "summary",
        None,
    );
    let t4 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-refresh-risk",
        reference(19, 4, 74),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(COMPOSER_TOP, 4, 63),
        "summary",
        None,
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
    let pairs = [request, t1, expand, t2, t3, t4, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.tool-task.expand",
            "tool-task",
            "tool-task-expanded",
            "direct",
            "findings open under the completed audit task while sibling states stay visible",
        )],
    ))
}

fn render_tool_task_expanded(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Audit the auth module and add the rate-limit tests.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I split the audit from the test work so findings can guide the patch.",
        TextStyle::text(),
    )?;
    let audit_task = ActivityRow::new(
        "Audit the auth module",
        ActivityState::Expanded,
        "6 findings ready",
    )
    .with_elapsed("3m12s")
    .compact();
    write_component(canvas, 12, 4, &audit_task)?;
    canvas.write(
        13,
        4,
        "▾ Task details · findings for this audit  [enter] Collapse",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "  ✗ Needs attention · Refresh tokens skip revoked sessions.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "  ✗ Needs attention · Test helper creates a session without token state.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "  ● Review · Login retries share no per-user limit.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "  ✓ Complete · Logout already invalidates the active token.",
        TextStyle::text(),
    )?;
    let tests = ActivityRow::new(
        "Add rate-limit tests",
        ActivityState::Running,
        "2 of 3 files · in progress",
    )
    .with_elapsed("48s elapsed")
    .compact();
    write_component(canvas, 19, 4, &tests)?;
    let queued = ActivityRow::new(
        "Build the test suite",
        ActivityState::Waiting,
        "queued after the test files",
    )
    .compact();
    write_component(canvas, 21, 4, &queued)?;
    let blocked = ActivityRow::new(
        "Review refresh session risk",
        ActivityState::Failed,
        "Blocked · needs a decision",
    )
    .compact();
    write_component(canvas, 23, 4, &blocked)?;
    composer(
        canvas,
        "Type a follow-up, it will wait until this turn finishes",
        None,
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 58),
        "summary",
        None,
    );
    let t1 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-audit-auth",
        reference(12, 4, 70),
        "focused",
        None,
    );
    let findings = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "task-audit-auth",
        reference(13, 4, 60),
        "focused",
        Some(action("action.tool-task.collapse", "Collapse", false)),
    );
    let t2 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-add-tests",
        reference(19, 4, 74),
        "summary",
        None,
    );
    let t3 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-build-suite",
        reference(21, 4, 70),
        "summary",
        None,
    );
    let t4 = semantic(
        screen_id,
        fixture_id,
        "activity",
        "task-refresh-risk",
        reference(23, 4, 74),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(COMPOSER_TOP, 4, 63),
        "summary",
        None,
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
    let pairs = [request, t1, findings, t2, t3, t4, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.tool-task.collapse",
            "tool-task-expanded",
            "tool-task",
            "direct",
            "collapse keeps attributed findings available under the same audit task",
        )],
    ))
}

fn chat_long_scroll_mode(fixture_id: &str) -> &'static str {
    if fixture_id.contains("compact")
        || fixture_id.contains("history-restore")
        || fixture_id.contains("collapse-history")
        || fixture_id.contains("expand-history")
        || fixture_id == "session.disclosure.compact"
    {
        "compact-restore"
    } else if fixture_id.contains("jump-earlier")
        || fixture_id.contains("earlier")
        || fixture_id.ends_with(".earlier")
    {
        "earlier"
    } else if fixture_id.contains("return-latest")
        || fixture_id.contains("latest")
        || fixture_id.ends_with(".latest")
    {
        "latest"
    } else {
        // Canonical default: mid-history view with earlier-turn signal + return-to-latest.
        "earlier"
    }
}

fn chat_long_scroll_origin_and_task(fixture_id: &str) -> (&'static str, &'static str) {
    match chat_long_scroll_mode(fixture_id) {
        "compact-restore" => (
            DAILY_WORK_ORIGIN,
            "Continue from the compacted conversation summary.",
        ),
        _ => (
            DAILY_WORK_ORIGIN,
            "Review earlier turns, then return to the latest work.",
        ),
    }
}

fn chat_long_scroll_turn_id(fixture_id: &str) -> &'static str {
    match chat_long_scroll_mode(fixture_id) {
        "compact-restore" => "turn.daily-work.compact.19",
        _ => "turn.daily-work.history.23",
    }
}

fn shell_mode(fixture_id: &str) -> &'static str {
    if fixture_id.contains("large-output") || fixture_id.contains("large_output") {
        if fixture_id.contains("search") {
            "large-full-search"
        } else if fixture_id.contains("close") || fixture_id.contains("compact") {
            "large-compact"
        } else if fixture_id.contains("open") || fixture_id.contains("full") {
            "large-full"
        } else {
            "large-compact"
        }
    } else if fixture_id.contains("mixed")
        || fixture_id.contains("foreground")
        || fixture_id.contains("background")
    {
        if fixture_id.contains("background-stop") || fixture_id.contains("background_stop") {
            "mixed-background-stopped"
        } else if fixture_id.contains("foreground") && fixture_id.contains("interrupt") {
            "mixed-foreground-interrupted"
        } else {
            "mixed"
        }
    } else if fixture_id.contains("queued-interrupt") {
        "queued-interrupt"
    } else if fixture_id.contains("interrupt") {
        "interrupted"
    } else if fixture_id.contains("complete") {
        "completed"
    } else {
        "running"
    }
}

fn streaming_mode(fixture_id: &str) -> &'static str {
    if fixture_id.contains("agent-question") {
        if fixture_id.contains("submit") {
            "agent-question-submit"
        } else if fixture_id.contains("cancel") {
            "agent-question-cancel"
        } else {
            "agent-question"
        }
    } else if fixture_id.contains("reading-anchor") || fixture_id.contains("new-output") {
        if fixture_id.contains("jump") {
            "reading-anchor-latest"
        } else {
            "reading-anchor"
        }
    } else if fixture_id.contains("queued-interrupt") {
        "queued-interrupt"
    } else {
        "baseline"
    }
}

fn chat_turn_interaction_mode(fixture_id: &str) -> &'static str {
    if fixture_id.contains("transcript-search-no-results") {
        "search-no-results"
    } else if fixture_id.contains("transcript-search-next") {
        "search-next"
    } else if fixture_id.contains("transcript-search-close") {
        "search-closed"
    } else if fixture_id.contains("transcript-search") {
        "search-open"
    } else if fixture_id.contains("transcript-copy") {
        "copy-confirmed"
    } else if fixture_id.contains("agent-question-submit") {
        "agent-question-resumed"
    } else if fixture_id.contains("queued-interrupt") {
        "queued-preserved"
    } else {
        "baseline"
    }
}

fn tool_task_mode(fixture_id: &str) -> &'static str {
    if fixture_id.contains("controls-expanded") || fixture_id.contains("task.controls-expanded") {
        "controls-expanded"
    } else if fixture_id.contains("stop-selected") {
        "stop-selected"
    } else if fixture_id.contains("retry-selected") {
        "retry-selected"
    } else if fixture_id.contains("cancel-selected") {
        "cancel-selected"
    } else if fixture_id.contains("controls") || fixture_id.contains("task.controls") {
        "controls"
    } else {
        "baseline"
    }
}

fn todo_mode(fixture_id: &str) -> &'static str {
    // Only the named progress-advance scenario and its resulting audit fixture
    // render the post-advance counts. Baseline / canonical stay at 3 of 5.
    if fixture_id.contains("todo-progress")
        || fixture_id.contains("advanced")
        || fixture_id.ends_with(".after")
    {
        "advanced"
    } else {
        "baseline"
    }
}

fn render_shell(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mode = shell_mode(fixture_id);
    if matches!(
        mode,
        "mixed"
            | "mixed-foreground-interrupted"
            | "mixed-background-stopped"
            | "large-compact"
            | "large-full"
            | "large-full-search"
            | "queued-interrupt"
    ) {
        return render_shell_interaction(canvas, screen_id, fixture_id, mode);
    }
    canvas.write(
        4,
        4,
        "You · Check the branch, then run the focused tests.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I will check the branch state first, then run the focused suite.",
        TextStyle::text(),
    )?;

    // Completed command with captured output kept inline under the turn.
    let completed = ActivityRow::new(
        "Shell · git status --short --branch",
        ActivityState::Succeeded,
        "exit 0 · 0.2s",
    )
    .compact();
    write_component(canvas, 11, 4, &completed)?;
    canvas.write(
        12,
        4,
        "  ⎿ ## master...origin/master [ahead 1]",
        TextStyle::muted(),
    )?;
    canvas.write(
        13,
        4,
        "  ⎿ M crates/server/src/models/session.rs",
        TextStyle::muted(),
    )?;
    canvas.write(
        14,
        4,
        "  ⎿ ?? crates/server/tests/login_regression.rs",
        TextStyle::muted(),
    )?;

    match mode {
        "interrupted" => {
            let interrupted = ActivityRow::new(
                "Shell · cargo test --workspace",
                ActivityState::Failed,
                "Interrupted · exit status kept",
            )
            .with_elapsed("4s")
            .compact();
            write_component(canvas, 16, 4, &interrupted)?;
            canvas.write(
                17,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                18,
                4,
                "  ⎿ Finished test profile in 3.1s",
                TextStyle::muted(),
            )?;
            canvas.write(
                19,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                21,
                4,
                "Command stopped · captured output kept · composer restored.",
                TextStyle::muted(),
            )?;
            composer(canvas, "Reply to Code", Some("Send reply"))?;
        }
        "completed" => {
            let finished = ActivityRow::new(
                "Shell · cargo test --workspace",
                ActivityState::Succeeded,
                "exit 0 · 4.2s",
            )
            .compact();
            write_component(canvas, 16, 4, &finished)?;
            canvas.write(
                17,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                18,
                4,
                "  ⎿ Finished test profile in 3.1s",
                TextStyle::muted(),
            )?;
            canvas.write(
                19,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                20,
                4,
                "  ⎿ test routes::api::quota_enforced ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                22,
                4,
                "Both commands finished · exit 0 preserved · continue in this turn.",
                TextStyle::muted(),
            )?;
            composer(canvas, "Reply to Code", Some("Send reply"))?;
        }
        _ => {
            let running = ActivityRow::new(
                "Shell · cargo test --workspace",
                ActivityState::Running,
                "running · 4s elapsed",
            )
            .with_interrupt("Stop command")
            .compact();
            write_component(canvas, 16, 4, &running)?;
            canvas.write(
                17,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                18,
                4,
                "  ⎿ Finished test profile in 3.1s",
                TextStyle::muted(),
            )?;
            canvas.write(
                19,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                20,
                4,
                "  ⎿ test routes::api::quota_enforced ... █",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Type a follow-up, it will wait until this command finishes",
                None,
            )?;
        }
    }

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 56),
        "summary",
        None,
    );
    let first_cmd = semantic(
        screen_id,
        fixture_id,
        "activity",
        "shell-git-status",
        reference(11, 4, 70),
        "summary",
        None,
    );
    let first_out = semantic(
        screen_id,
        fixture_id,
        "activity",
        "shell-git-status-output",
        reference(12, 4, 48),
        "summary",
        None,
    );
    let second_cmd = semantic(
        screen_id,
        fixture_id,
        "activity",
        match mode {
            "interrupted" => "shell-cargo-test-interrupted",
            "completed" => "shell-cargo-test-complete",
            _ => "shell-cargo-test-running",
        },
        reference(16, 4, 78),
        "summary",
        match mode {
            "running" => Some(action(
                "action.shell.interrupt",
                "Stop command",
                false,
            )),
            "completed" => Some(action(
                "action.shell.complete",
                "Command finished",
                true,
            )),
            _ => None,
        },
    );
    let automatic_complete = if mode == "running" {
        Some(semantic(
            screen_id,
            fixture_id,
            "completion",
            "shell-cargo-test",
            reference(16, 4, 78),
            "summary",
            Some(action(
                "action.shell.complete",
                "Command finished",
                true,
            )),
        ))
    } else {
        None
    };
    let second_out = semantic(
        screen_id,
        fixture_id,
        "activity",
        "shell-cargo-test-output",
        reference(17, 4, 42),
        "summary",
        None,
    );
    let composer_ev = match mode {
        "running" => semantic(
            screen_id,
            fixture_id,
            "queued-input",
            "follow-up",
            reference(COMPOSER_TOP, 4, 68),
            "summary",
            None,
        ),
        _ => semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.shell.continue",
            reference(COMPOSER_TOP, 4, 29),
            "summary",
            Some(action("action.shell.continue", "Send reply", false)),
        ),
    };
    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    let mut candidates = vec![
        request.0.clone(),
        first_cmd.0.clone(),
        first_out.0.clone(),
        second_cmd.0.clone(),
        second_out.0.clone(),
        composer_ev.0.clone(),
        status.0.clone(),
    ];
    let mut events = vec![
        request.1.clone(),
        first_cmd.1.clone(),
        first_out.1.clone(),
        second_cmd.1.clone(),
        second_out.1.clone(),
        composer_ev.1.clone(),
        status.1.clone(),
    ];
    if let Some(complete) = automatic_complete {
        candidates.push(complete.0);
        events.push(complete.1);
    }
    let transitions = match mode {
        "running" => vec![
            transition(
                "action.shell.interrupt",
                "shell",
                "shell",
                "direct",
                "interrupt stops the live command, keeps captured output, and restores the composer",
            ),
            transition(
                "action.shell.complete",
                "shell",
                "shell",
                "automatic",
                "completion replaces the running row with exit 0 and restores the composer",
            ),
        ],
        "interrupted" => vec![transition(
            "action.shell.continue",
            "shell",
            "chat-turn",
            "direct",
            "after interrupt the same turn continues with preserved output",
        )],
        _ => vec![transition(
            "action.shell.continue",
            "shell",
            "chat-turn",
            "direct",
            "after completion the same turn continues with exit status and output preserved",
        )],
    };
    Ok((candidates, events, transitions))
}

fn render_todo_list(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let advanced = todo_mode(fixture_id) == "advanced";
    canvas.write(
        4,
        4,
        "You · Implement rate limiting on the login endpoint.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, "Updated plan", TextStyle::bright())?;

    canvas.write(
        11,
        4,
        "✓ Read the auth middleware            reviewed the login handler · done",
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        4,
        "✓ Add a token-bucket limiter          created the request guard · done",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "✓ Wire the POST /login route          attached the limiter · done",
        TextStyle::text(),
    )?;

    if advanced {
        canvas.write(
            14,
            4,
            "✓ Add tests/login_ratelimit.rs        cover the fifth attempt · done",
            TextStyle::text(),
        )?;
        canvas.write(
            15,
            4,
            "● Run cargo test and cargo fmt        verify the full change · in progress",
            TextStyle::bright(),
        )?;
        canvas.write(
            17,
            4,
            "4 of 5 done · 1 in progress · 0 pending",
            TextStyle::text(),
        )?;
        canvas.write(
            19,
            4,
            "● Running the focused suite · 22s · next work is this verification step",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            14,
            4,
            "● Add tests/login_ratelimit.rs        cover the fifth attempt · in progress",
            TextStyle::bright(),
        )?;
        canvas.write(
            15,
            4,
            "○ Run cargo test and cargo fmt        verify the full change · pending",
            TextStyle::text(),
        )?;
        canvas.write(
            16,
            4,
            "  ● Writing tests… 22s · 1 file · next work is this item",
            TextStyle::muted(),
        )?;
        canvas.write(
            18,
            4,
            "3 of 5 done · 1 in progress · 1 pending",
            TextStyle::text(),
        )?;
    }

    composer(
        canvas,
        "Type a follow-up, it runs after this turn…",
        None,
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 58),
        "summary",
        None,
    );
    let item1 = semantic(
        screen_id,
        fixture_id,
        "todo-item",
        "todo-done-auth-middleware",
        reference(11, 4, 72),
        "summary",
        None,
    );
    let item2 = semantic(
        screen_id,
        fixture_id,
        "todo-item",
        "todo-done-token-bucket",
        reference(12, 4, 72),
        "summary",
        None,
    );
    let item3 = semantic(
        screen_id,
        fixture_id,
        "todo-item",
        "todo-done-login-route",
        reference(13, 4, 70),
        "summary",
        None,
    );
    let (item4, item5) = if advanced {
        (
            semantic(
                screen_id,
                fixture_id,
                "todo-item",
                "todo-done-login-tests",
                reference(14, 4, 74),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "todo-item",
                "todo-current-verify-suite",
                reference(15, 4, 78),
                "summary",
                Some(action(
                    "action.todo.advance",
                    "Mark current done",
                    true,
                )),
            ),
        )
    } else {
        (
            semantic(
                screen_id,
                fixture_id,
                "todo-item",
                "todo-current-login-tests",
                reference(14, 4, 78),
                "summary",
                Some(action(
                    "action.todo.advance",
                    "Mark current done",
                    true,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "todo-item",
                "todo-pending-verify-suite",
                reference(15, 4, 74),
                "summary",
                None,
            ),
        )
    };
    let summary = semantic(
        screen_id,
        fixture_id,
        "todo-summary",
        "todo-progress-summary",
        reference(if advanced { 17 } else { 18 }, 4, 42),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "queued-input",
        "follow-up",
        reference(COMPOSER_TOP, 4, 48),
        "summary",
        None,
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
        request, item1, item2, item3, item4, item5, summary, composer_ev, status,
    ];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.todo.advance",
            "todo-list",
            "todo-list",
            "automatic",
            "only affected task states and the progress summary change while the origin turn and composer stay put",
        )],
    ))
}

fn render_compact(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, "You · /compact", TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(9, 4, "Conversation compacted", TextStyle::bright())?;
    canvas.write(
        11,
        4,
        "→ Found the shared rate-limit guard and the two route call sites.",
        TextStyle::text(),
    )?;
    canvas.write(
        12,
        4,
        "→ Confirmed login and API reuse one safety rule.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "→ Next: write a short regression note, then run the focused suite.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "The full transcript is preserved in this session.",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "› 18 earlier turns stay available  [enter] Expand full history",
        TextStyle::text(),
    )?;
    composer(canvas, "Resume from the compacted summary", Some("Send reply"))?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 16),
        "summary",
        None,
    );
    let summary = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "compaction-summary",
        reference(9, 4, 23),
        "summary",
        None,
    );
    let preserve = semantic(
        screen_id,
        fixture_id,
        "preservation",
        "full-history",
        reference(15, 4, 53),
        "summary",
        None,
    );
    let expand = semantic(
        screen_id,
        fixture_id,
        "disclosure",
        "history-restore",
        reference(17, 4, 63),
        "summary",
        Some(action(
            "action.compact.expand-history",
            "Expand full history",
            false,
        )),
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.compact.continue",
        reference(COMPOSER_TOP, 4, 43),
        "summary",
        Some(action("action.compact.continue", "Send reply", false)),
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
    let pairs = [request, summary, preserve, expand, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.compact.expand-history",
            "compact",
            "chat-long-scroll",
            "direct",
            "full history restores earlier turns in order with the same draft",
        )],
    ))
}

fn render_chat_long_scroll(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mode = chat_long_scroll_mode(fixture_id);

    if mode == "compact-restore" {
        canvas.write(4, 4, "Earlier history restored · same session", TextStyle::muted())?;
        canvas.write(
            6,
            4,
            "Turn 1 · You · Find the shared rate-limit guard.",
            TextStyle::bright(),
        )?;
        canvas.write(
            7,
            4,
            "Turn 1 · Code · Found middleware/rate_limit.rs as the single owner.",
            TextStyle::text(),
        )?;
        canvas.write(
            9,
            4,
            "Turn 8 · You · Confirm login and API both call it.",
            TextStyle::bright(),
        )?;
        canvas.write(
            10,
            4,
            "Turn 8 · Code · Yes. Both reuse one safety rule.",
            TextStyle::text(),
        )?;
        canvas.write(
            12,
            4,
            "Turn 18 · You · Find where the server applies the rate limit and explain the safe path.",
            TextStyle::bright(),
        )?;
        canvas.write(
            13,
            4,
            "Turn 18 · Code · The shared request guard applies the limit before route work begins.",
            TextStyle::text(),
        )?;
        canvas.write(
            15,
            4,
            "▾ Full history open  [enter] Return to compact summary",
            TextStyle::text(),
        )?;
        canvas.write(
            17,
            4,
            "18 turns restored in order · draft and reading position kept.",
            TextStyle::muted(),
        )?;
        composer(canvas, "Resume from the compacted summary", Some("Send reply"))?;

        let restore = semantic(
            screen_id,
            fixture_id,
            "disclosure",
            "history-restore",
            reference(15, 4, 56),
            "focused",
            Some(action(
                "action.compact.collapse-history",
                "Return to compact summary",
                false,
            )),
        );
        let turns = semantic(
            screen_id,
            fixture_id,
            "transcript",
            "restored-history",
            reference(6, 4, 50),
            "focused",
            None,
        );
        let composer_ev = semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.chat-long-scroll.continue",
            reference(COMPOSER_TOP, 4, 43),
            "summary",
            Some(action(
                "action.chat-long-scroll.continue",
                "Send reply",
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
        let pairs = [turns, restore, composer_ev, status];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            vec![transition(
                "action.compact.collapse-history",
                "chat-long-scroll",
                "compact",
                "direct",
                "return restores the same compact summary and draft without reordering turns",
            )],
        ));
    }

    // Long-history navigation modes (earlier viewport vs latest).
    let draft = "Add a short note about the remaining risk.";
    if mode == "latest" {
        canvas.write(
            4,
            4,
            "Near the latest activity · draft kept",
            TextStyle::muted(),
        )?;
        canvas.write(
            6,
            4,
            "↑ 12 earlier turns · [ctrl+u] Jump to earlier",
            TextStyle::text(),
        )?;
        canvas.write(
            8,
            4,
            "You · why keep the old token around for five minutes after rotation?",
            TextStyle::bright(),
        )?;
        canvas.write(10, 4, "Code", TextStyle::bright())?;
        canvas.write(
            12,
            4,
            "The grace window absorbs racing requests so a client that fired two",
            TextStyle::text(),
        )?;
        canvas.write(
            13,
            4,
            "calls in parallel can still present the outgoing token briefly.",
            TextStyle::text(),
        )?;
        canvas.write(
            14,
            4,
            "Within that window enforce() accepts the old token for reads only.",
            TextStyle::text(),
        )?;
        canvas.write(
            16,
            4,
            "● In short · rotation is immediate for writes; reads get a five-minute grace window.",
            TextStyle::text(),
        )?;
        canvas.write(
            18,
            4,
            "● Latest turn · scroll position at newest work",
            TextStyle::muted(),
        )?;
        canvas.write(
            20,
            4,
            "↑ Earlier turns stay available without covering this answer.",
            TextStyle::muted(),
        )?;
        composer(canvas, draft, Some("Send reply"))?;
    } else {
        // earlier
        canvas.write(
            4,
            4,
            "↑ 12 earlier turns · [ctrl+u] Jump to earlier · reading older work",
            TextStyle::text(),
        )?;
        canvas.write(
            6,
            4,
            "You · where do refresh tokens get persisted?",
            TextStyle::bright(),
        )?;
        canvas.write(8, 4, "Code", TextStyle::bright())?;
        canvas.write(
            10,
            4,
            "They live in the sessions table, one row per device. TokenService",
            TextStyle::text(),
        )?;
        canvas.write(
            11,
            4,
            "writes them through store::persist() at issue time and rotates the",
            TextStyle::text(),
        )?;
        canvas.write(
            12,
            4,
            "row in place on every refresh.",
            TextStyle::text(),
        )?;
        canvas.write(
            14,
            4,
            "You · add an integration test that proves rotation invalidates the old token",
            TextStyle::bright(),
        )?;
        canvas.write(16, 4, "Code", TextStyle::bright())?;
        canvas.write(
            18,
            4,
            "Added rotation_invalidates_previous_token to crates/server/tests/auth.rs.",
            TextStyle::text(),
        )?;
        canvas.write(
            19,
            4,
            "It signs in, refreshes once, then replays the pre-refresh token.",
            TextStyle::text(),
        )?;
        canvas.write(
            21,
            4,
            "› Jump to latest  [enter] Return to the newest turn and composer",
            TextStyle::bright(),
        )?;
        canvas.write(
            23,
            4,
            "Scroll position · earlier-turn-band · draft preserved",
            TextStyle::muted(),
        )?;
        composer(canvas, draft, Some("Send reply"))?;
    }

    let indicator = semantic(
        screen_id,
        fixture_id,
        "scroll",
        "earlier-turns-indicator",
        reference(if mode == "latest" { 6 } else { 4 }, 4, 62),
        "summary",
        Some(action(
            "action.chat-long-scroll.jump-earlier",
            "Jump to earlier",
            false,
        )),
    );
    let transcript = semantic(
        screen_id,
        fixture_id,
        "transcript",
        if mode == "latest" {
            "latest-visible-turn"
        } else {
            "earlier-visible-turns"
        },
        reference(if mode == "latest" { 8 } else { 6 }, 4, 60),
        "summary",
        None,
    );
    let nav = if mode == "latest" {
        semantic(
            screen_id,
            fixture_id,
            "scroll",
            "history-position-latest",
            reference(18, 4, 50),
            "summary",
            None,
        )
    } else {
        semantic(
            screen_id,
            fixture_id,
            "scroll",
            "return-to-latest",
            reference(21, 4, 68),
            "summary",
            Some(action(
                "action.chat-long-scroll.return-latest",
                "Jump to latest",
                false,
            )),
        )
    };
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.chat-long-scroll.continue",
        reference(COMPOSER_TOP, 4, 48),
        "summary",
        Some(action(
            "action.chat-long-scroll.continue",
            "Send reply",
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
    let pairs = [indicator, transcript, nav, composer_ev, status];
    let transitions = if mode == "latest" {
        vec![
            transition(
                "action.chat-long-scroll.jump-earlier",
                "chat-long-scroll",
                "chat-long-scroll",
                "direct",
                "jump earlier preserves the draft and lands at the earlier-turn-band anchor",
            ),
            transition(
                "action.chat-long-scroll.continue",
                "chat-long-scroll",
                "streaming",
                "direct",
                "reply continues from the latest turn without losing the draft",
            ),
        ]
    } else {
        vec![
            transition(
                "action.chat-long-scroll.return-latest",
                "chat-long-scroll",
                "chat-long-scroll",
                "direct",
                "return to latest restores the prior draft and lands near the exact latest activity",
            ),
            transition(
                "action.chat-long-scroll.jump-earlier",
                "chat-long-scroll",
                "chat-long-scroll",
                "direct",
                "jump earlier preserves the draft and lands at the earlier-turn-band anchor",
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

fn daily_user_request(fixture_id: &str) -> &'static str {
    if fixture_id == "canonical.chat-turn" {
        "You · Check retry_policy.rs before we merge."
    } else if fixture_id == "conversation-completed" {
        "You · Why are stale sessions rejected after a password change?"
    } else if fixture_id.contains("task") {
        "You · Audit the auth module and add the rate-limit tests."
    } else {
        "You · Find where the server applies the rate limit and explain the safe path."
    }
}

fn composer(
    canvas: &mut Canvas,
    placeholder: &str,
    action_label: Option<&str>,
) -> Result<(), String> {
    canvas.write(
        COMPOSER_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_TOP + 1,
        2,
        &format!("│ › {placeholder}"),
        TextStyle::bright(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    if let Some(action_label) = action_label {
        canvas.write(
            COMPOSER_BOTTOM - 1,
            4,
            &format!("[enter] {action_label}"),
            TextStyle::accent(),
        )?;
    }
    Ok(())
}

fn composer_with_draft(
    canvas: &mut Canvas,
    draft: &str,
    attachment: &str,
    action_label: Option<&str>,
) -> Result<(), String> {
    canvas.write(
        COMPOSER_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_TOP + 1,
        2,
        &format!("│ › {draft}"),
        TextStyle::bright(),
    )?;
    canvas.write(
        COMPOSER_TOP + 2,
        2,
        &format!("│   [attachment: {attachment}]"),
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    if let Some(action_label) = action_label {
        canvas.write(
            COMPOSER_BOTTOM - 1,
            4,
            &format!("[enter] {action_label}"),
            TextStyle::accent(),
        )?;
    }
    Ok(())
}

fn write_component(
    canvas: &mut Canvas,
    first_row: u16,
    column: u16,
    component: &impl Component,
) -> Result<(), String> {
    for (index, line) in component.lines().into_iter().enumerate() {
        canvas.write(first_row + index as u16, column, &line.text, line.style)?;
    }
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

fn render_streaming_interaction(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mode = streaming_mode(fixture_id);
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;

    match mode {
        "agent-question" | "agent-question-cancel" => {
            canvas.write(
                9,
                4,
                "I need one choice before continuing the recovery note.",
                TextStyle::text(),
            )?;
            canvas.write(
                11,
                4,
                "Which recovery path should we keep for the shared request guard?",
                TextStyle::bright(),
            )?;
            canvas.write(
                13,
                4,
                "● Paused activity · waiting for your answer",
                TextStyle::text(),
            )?;
            canvas.write(
                COMPOSER_TOP,
                2,
                &format!("╭{}╮", "─".repeat(114)),
                TextStyle::muted(),
            )?;
            canvas.write(
                COMPOSER_TOP + 1,
                2,
                "│ Answer needed · pick one option, then submit once",
                TextStyle::bright(),
            )?;
            canvas.write(
                COMPOSER_TOP + 2,
                2,
                "│ › Keep the shared guard · continue this activity",
                TextStyle::bright(),
            )?;
            canvas.write(
                COMPOSER_TOP + 3,
                2,
                "│   Switch to per-route limits · change the plan",
                TextStyle::text(),
            )?;
            canvas.write(
                COMPOSER_TOP + 4,
                2,
                "│ [enter] Submit answer   [esc] Cancel · keep paused",
                TextStyle::accent(),
            )?;
            canvas.write(
                COMPOSER_BOTTOM,
                2,
                &format!("╰{}╯", "─".repeat(114)),
                TextStyle::muted(),
            )?;
            let question = semantic(
                screen_id,
                fixture_id,
                "transcript",
                "agent-question",
                reference(11, 4, 78),
                "focused",
                None,
            );
            let dock = semantic(
                screen_id,
                fixture_id,
                "action-dock",
                "agent-question-dock",
                reference(COMPOSER_TOP + 1, 4, 64),
                "focused",
                Some(action(
                    "action.agent-question.submit",
                    "Submit answer",
                    false,
                )),
            );
            let cancel = semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "agent-question-cancel",
                reference(COMPOSER_TOP + 4, 34, 68),
                "summary",
                Some(action("action.agent-question.cancel", "Cancel", false)),
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
            let pairs = [question, dock, cancel, status];
            return Ok((
                pairs
                    .iter()
                    .map(|(candidate, _)| candidate.clone())
                    .collect(),
                pairs.iter().map(|(_, event)| event.clone()).collect(),
                vec![
                    transition(
                        "action.agent-question.submit",
                        "streaming",
                        "chat-turn",
                        "direct",
                        "submit resumes that exact activity once and restores the prior composer draft",
                    ),
                    transition(
                        "action.agent-question.cancel",
                        "streaming",
                        "streaming",
                        "direct",
                        "cancel keeps the activity paused with the prior draft intact",
                    ),
                ],
            ));
        }
        "reading-anchor" => {
            canvas.write(
                9,
                4,
                "Earlier · the rate-limit guard still defines the safe path.",
                TextStyle::text(),
            )?;
            canvas.write(
                11,
                4,
                "That earlier detail stays in view while new lines arrive below.",
                TextStyle::text(),
            )?;
            let live = ActivityRow::new(
                "Writing reply",
                ActivityState::Running,
                "new output waiting below",
            )
            .with_interrupt("Stop reply")
            .compact();
            write_component(canvas, 14, 4, &live)?;
            canvas.write(
                16,
                4,
                "› New output · jump to newest without forcing scroll  [enter]",
                TextStyle::accent(),
            )?;
            composer(
                canvas,
                "Note the remaining risk after this section.",
                None,
            )?;
            let anchor = semantic(
                screen_id,
                fixture_id,
                "transcript",
                "reading-anchor",
                reference(9, 4, 70),
                "focused",
                None,
            );
            let affordance = semantic(
                screen_id,
                fixture_id,
                "live-state",
                "new-output",
                reference(16, 4, 72),
                "summary",
                Some(action("action.streaming.new-output", "New output", false)),
            );
            let draft = semantic(
                screen_id,
                fixture_id,
                "queued-input",
                "follow-up",
                reference(COMPOSER_TOP, 4, 52),
                "summary",
                None,
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
            let pairs = [anchor, affordance, draft, status];
            return Ok((
                pairs
                    .iter()
                    .map(|(candidate, _)| candidate.clone())
                    .collect(),
                pairs.iter().map(|(_, event)| event.clone()).collect(),
                vec![transition(
                    "action.streaming.new-output",
                    "streaming",
                    "streaming",
                    "direct",
                    "New output jumps to the newest lines without changing draft or activity state",
                )],
            ));
        }
        "reading-anchor-latest" => {
            canvas.write(
                9,
                4,
                "Newest · below the earlier rate-limit guard section.",
                TextStyle::text(),
            )?;
            canvas.write(
                11,
                4,
                "The latest paragraph landed here after Jump to newest.",
                TextStyle::text(),
            )?;
            let live = ActivityRow::new(
                "Writing reply",
                ActivityState::Running,
                "latest output visible",
            )
            .with_interrupt("Stop reply")
            .compact();
            write_component(canvas, 14, 4, &live)?;
            canvas.write(
                16,
                4,
                "At newest output · draft unchanged · activity still running",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Note the remaining risk after this section.",
                None,
            )?;
        }
        _ => {
            // queued-interrupt streaming before state.
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            let live = ActivityRow::new(
                "Writing reply",
                ActivityState::Running,
                "24s · two queued follow-ups waiting",
            )
            .with_interrupt("Stop reply")
            .compact();
            write_component(canvas, 12, 4, &live)?;
            canvas.write(
                14,
                4,
                "Queued 1 · Also list unit tests. [attachment: queue-notes-a.md]",
                TextStyle::muted(),
            )?;
            canvas.write(
                15,
                4,
                "Queued 2 · Then summarize risks. [attachment: queue-notes-b.md]",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Hold these two follow-ups until streaming ends.",
                None,
            )?;
        }
    }

    let partial = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "partial-answer",
        reference(9, 4, 68),
        "summary",
        None,
    );
    let live_state = semantic(
        screen_id,
        fixture_id,
        "live-state",
        "active-response",
        reference(12, 4, 70),
        "summary",
        Some(action("action.streaming.interrupt", "Stop reply", false)),
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
    let pairs = [partial, live_state, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.streaming.interrupt",
            "streaming",
            "chat-turn",
            "direct",
            "interrupt keeps both queued prompts and attachments in original order",
        )],
    ))
}

fn render_chat_turn_interaction(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mode = chat_turn_interaction_mode(fixture_id);
    canvas.write(4, 4, daily_user_request(fixture_id), TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;

    match mode {
        "queued-preserved" => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                "● Reply stopped safely. Queued follow-ups retained in order.",
                TextStyle::text(),
            )?;
            canvas.write(
                14,
                4,
                "Queued 1 · queue-item-1 rev=1 · Also list unit tests. [attachment: queue-notes-a.md]",
                TextStyle::text(),
            )?;
            canvas.write(
                15,
                4,
                "Queued 2 · queue-item-2 rev=2 · Then summarize risks. [attachment: queue-notes-b.md]",
                TextStyle::text(),
            )?;
            composer(canvas, "Reply to Code", Some("Send reply"))?;
        }
        "agent-question-resumed" => {
            canvas.write(
                9,
                4,
                "Answer submitted once. Activity resumed with the prior draft restored.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                "Keep the shared guard · continue this activity",
                TextStyle::muted(),
            )?;
            composer_with_draft(
                canvas,
                "Keep the risk note short before we publish.",
                "risk-notes.md",
                Some("Send reply"),
            )?;
        }
        "search-open" | "search-next" => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                if mode == "search-next" {
                    "Search · 3 matches for \"guard\" · match 2 of 3"
                } else {
                    "Search · 3 matches for \"guard\" · match 1 of 3"
                },
                TextStyle::bright(),
            )?;
            canvas.write(
                14,
                4,
                "› match · shared request guard",
                TextStyle::accent(),
            )?;
            canvas.write(
                16,
                4,
                "[n] Next match   [esc] Close search · return to origin",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Add a short follow-up about the guard.",
                None,
            )?;
        }
        "search-no-results" => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                "Search · No matches for \"xyzzy-not-found\"",
                TextStyle::bright(),
            )?;
            canvas.write(
                14,
                4,
                "Try another term, or close search to return to the prior position.",
                TextStyle::muted(),
            )?;
            canvas.write(
                16,
                4,
                "[esc] Close search",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Add a short follow-up about the guard.",
                None,
            )?;
        }
        "search-closed" => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                "Search closed · returned to the originating reading position.",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Add a short follow-up about the guard.",
                Some("Send reply"),
            )?;
        }
        "copy-confirmed" => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            canvas.write(
                12,
                4,
                "Copied · selected message only · no hidden rationale",
                TextStyle::text(),
            )?;
            canvas.write(
                14,
                4,
                "copy confirmed · turn count unchanged",
                TextStyle::muted(),
            )?;
            composer(
                canvas,
                "Add a short follow-up about the guard.",
                Some("Send reply"),
            )?;
        }
        _ => {
            canvas.write(
                9,
                4,
                "The shared request guard applies the limit before route work begins.",
                TextStyle::text(),
            )?;
            composer(canvas, "Reply to Code", Some("Send reply"))?;
        }
    }

    let body = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "interaction-body",
        reference(9, 4, 70),
        "summary",
        None,
    );
    let composer_ev = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.chat-turn.continue",
        reference(COMPOSER_TOP, 4, 36),
        "summary",
        Some(action("action.chat-turn.send-reply", "Send reply", false)),
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
    let pairs = [body, composer_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.chat-turn.send-reply",
            "chat-turn",
            "streaming",
            "direct",
            "the conversation continues from the interaction state",
        )],
    ))
}

fn render_shell_interaction(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    mode: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Check the branch, then run the focused tests.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I will check the branch, run the suite, and keep long output bounded.",
        TextStyle::text(),
    )?;

    match mode {
        "queued-interrupt" => {
            write_component(
                canvas,
                12,
                4,
                &ActivityRow::new(
                    "Shell · cargo test --workspace",
                    ActivityState::Failed,
                    "Interrupted · queue retained",
                )
                .with_elapsed("4s")
                .compact(),
            )?;
            canvas.write(
                13,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                15,
                4,
                "Queued 1 · queue-item-1 rev=1 · Also list unit tests. [attachment: queue-notes-a.md]",
                TextStyle::text(),
            )?;
            canvas.write(
                16,
                4,
                "Queued 2 · queue-item-2 rev=2 · Then summarize risks. [attachment: queue-notes-b.md]",
                TextStyle::text(),
            )?;
            composer(canvas, "Reply to Code", Some("Send reply"))?;
        }
        "mixed" | "mixed-foreground-interrupted" | "mixed-background-stopped" => {
            let fg_state = if mode == "mixed-foreground-interrupted" {
                ActivityState::Failed
            } else {
                ActivityState::Running
            };
            let fg_detail = if mode == "mixed-foreground-interrupted" {
                "Foreground · Interrupted · exit kept"
            } else {
                "Foreground · running · 4s elapsed"
            };
            let mut fg = ActivityRow::new(
                "Shell · cargo test --workspace",
                fg_state,
                fg_detail,
            )
            .compact();
            if mode == "mixed" {
                fg = fg.with_interrupt("Stop command");
            }
            write_component(canvas, 12, 4, &fg)?;
            canvas.write(
                13,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                14,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;

            let bg_state = if mode == "mixed-background-stopped" {
                ActivityState::Failed
            } else {
                ActivityState::Running
            };
            let bg_detail = if mode == "mixed-background-stopped" {
                "Background · Stopped · captured output kept"
            } else {
                "Background · running · watch active"
            };
            write_component(
                canvas,
                17,
                4,
                &ActivityRow::new("Shell · cargo watch -x check", bg_state, bg_detail).compact(),
            )?;
            canvas.write(
                18,
                4,
                "  ⎿ watch: file .../session.rs changed",
                TextStyle::muted(),
            )?;
            if mode != "mixed-background-stopped" {
                canvas.write(
                    20,
                    4,
                    "› [s] Stop background · ends only this background command",
                    TextStyle::text(),
                )?;
            } else {
                canvas.write(
                    20,
                    4,
                    "Background stopped · unrelated activities preserved",
                    TextStyle::muted(),
                )?;
            }
            if mode == "mixed" {
                composer(
                    canvas,
                    "Type a follow-up, it will wait until foreground work finishes",
                    None,
                )?;
            } else {
                composer(canvas, "Reply to Code", Some("Send reply"))?;
            }
        }
        "large-compact" => {
            write_component(
                canvas,
                12,
                4,
                &ActivityRow::new(
                    "Shell · cargo test --workspace",
                    ActivityState::Succeeded,
                    "exit 0 · 12 of 240 lines · 228 omitted",
                )
                .compact(),
            )?;
            canvas.write(
                13,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                14,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                15,
                4,
                "  ⎿ … 228 lines omitted · full detail keeps every line",
                TextStyle::muted(),
            )?;
            canvas.write(
                17,
                4,
                "› [enter] Full detail · searchable complete output",
                TextStyle::accent(),
            )?;
            canvas.write(
                18,
                4,
                "  [c] Copy summary   Return stays on this compact activity",
                TextStyle::muted(),
            )?;
            composer(canvas, "Also list tests after this suite output.", None)?;
        }
        "large-full" | "large-full-search" => {
            write_component(
                canvas,
                11,
                4,
                &ActivityRow::new(
                    "Shell · cargo test --workspace",
                    ActivityState::Expanded,
                    "full detail · 240 lines · lossless",
                )
                .compact(),
            )?;
            canvas.write(
                12,
                4,
                "  ⎿ Compiling cortex-server v0.1.0",
                TextStyle::muted(),
            )?;
            canvas.write(
                13,
                4,
                "  ⎿ test routes::login::rate_limited ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                14,
                4,
                "  ⎿ test routes::api::quota_enforced ... ok",
                TextStyle::muted(),
            )?;
            canvas.write(
                15,
                4,
                "  ⎿ showing lines 1-12 of 240 · scroll for more · lossless",
                TextStyle::muted(),
            )?;
            if mode == "large-full-search" {
                canvas.write(
                    16,
                    4,
                    "Search output · match 1 for rate_limited",
                    TextStyle::bright(),
                )?;
                canvas.write(
                    17,
                    4,
                    "› output-match-1 · test routes::login::rate_limited ... ok",
                    TextStyle::accent(),
                )?;
            } else {
                canvas.write(
                    16,
                    4,
                    "Search output · type to filter · complete hash retained",
                    TextStyle::text(),
                )?;
            }
            canvas.write(
                19,
                4,
                "[c] Copy   [esc] Return · restore compact 12 of 240 · 228 omitted",
                TextStyle::muted(),
            )?;
            composer(canvas, "Also list tests after this suite output.", None)?;
        }
        _ => {}
    }

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 56),
        "summary",
        None,
    );
    let activity = semantic(
        screen_id,
        fixture_id,
        "activity",
        match mode {
            "mixed-background-stopped" => "shell-background-stopped",
            "mixed-foreground-interrupted" => "shell-foreground-interrupted",
            "large-full" | "large-full-search" => "shell-large-full",
            "large-compact" => "shell-large-summary",
            "queued-interrupt" => "shell-cargo-test-interrupted",
            _ => "shell-mixed-foreground",
        },
        reference(12, 4, 78),
        "summary",
        Some(action(
            match mode {
                "mixed-background-stopped" => "action.shell.background-stop",
                "mixed-foreground-interrupted" => "action.shell.foreground-interrupt",
                "large-full" | "large-full-search" => "action.shell.large-output-search",
                "large-compact" => "action.shell.large-output-open",
                "queued-interrupt" => "action.shell.interrupt",
                _ => "action.shell.foreground-interrupt",
            },
            match mode {
                "mixed-background-stopped" => "Stop background",
                "large-compact" => "Full detail",
                "large-full" | "large-full-search" => "Search output",
                "queued-interrupt" => "Stop command",
                _ => "Stop command",
            },
            false,
        )),
    );
    let secondary = match mode {
        "mixed" | "mixed-foreground-interrupted" => Some(semantic(
            screen_id,
            fixture_id,
            "activity",
            "shell-mixed-background",
            reference(17, 4, 70),
            "summary",
            Some(action(
                "action.shell.background-stop",
                "Stop background",
                false,
            )),
        )),
        "large-full" | "large-full-search" => Some(semantic(
            screen_id,
            fixture_id,
            "activity",
            "shell-large-return",
            reference(19, 4, 70),
            "summary",
            Some(action(
                "action.shell.large-output-close",
                "Return",
                false,
            )),
        )),
        _ => None,
    };
    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    let mut candidates = vec![request.0.clone(), activity.0.clone(), status.0.clone()];
    let mut events = vec![request.1.clone(), activity.1.clone(), status.1.clone()];
    if let Some(sec) = secondary {
        candidates.push(sec.0);
        events.push(sec.1);
    }
    let transitions = match mode {
        "large-compact" => vec![transition(
            "action.shell.large-output-open",
            "shell",
            "shell",
            "direct",
            "full detail opens searchable complete output without clipping silently",
        )],
        "large-full" | "large-full-search" => vec![
            transition(
                "action.shell.large-output-search",
                "shell",
                "shell",
                "direct",
                "search inside complete output preserves the lossless payload",
            ),
            transition(
                "action.shell.large-output-close",
                "shell",
                "shell",
                "direct",
                "return restores the compact activity with bounded omitted count",
            ),
        ],
        "mixed-background-stopped" => vec![transition(
            "action.shell.background-stop",
            "shell",
            "shell",
            "direct",
            "Stop background command terminates only that process and keeps captures",
        )],
        "queued-interrupt" => vec![transition(
            "action.shell.interrupt",
            "shell",
            "shell",
            "direct",
            "shell interrupt preserves queued prompts, attachments, and captured output",
        )],
        "mixed-foreground-interrupted" => vec![
            transition(
                "action.shell.foreground-interrupt",
                "shell",
                "shell",
                "direct",
                "interrupt stops only the selected foreground process",
            ),
            transition(
                "action.shell.background-stop",
                "shell",
                "shell",
                "direct",
                "Stop background command terminates only that process and keeps captures",
            ),
        ],
        _ => vec![
            transition(
                "action.shell.foreground-interrupt",
                "shell",
                "shell",
                "direct",
                "interrupt stops only the selected foreground process",
            ),
            transition(
                "action.shell.background-stop",
                "shell",
                "shell",
                "direct",
                "Stop background command terminates only that process and keeps captures",
            ),
        ],
    };
    Ok((candidates, events, transitions))
}

fn render_tool_task_controls(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let mode = tool_task_mode(fixture_id);
    canvas.write(
        4,
        4,
        "You · Audit the auth module and add the rate-limit tests.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I split the audit from the test work so findings can guide the patch.",
        TextStyle::text(),
    )?;

    write_component(
        canvas,
        12,
        4,
        &ActivityRow::new(
            "Audit the auth module",
            if mode == "controls-expanded" {
                ActivityState::Expanded
            } else {
                ActivityState::Succeeded
            },
            "6 findings ready",
        )
        .with_elapsed("3m12s")
        .compact(),
    )?;
    if mode == "controls-expanded" {
        canvas.write(
            13,
            4,
            "  ✗ Needs attention · Refresh tokens skip revoked sessions.",
            TextStyle::text(),
        )?;
    }

    let tests_state = if mode == "stop-selected" {
        ActivityState::Failed
    } else {
        ActivityState::Running
    };
    let tests_detail = if mode == "stop-selected" {
        "Stopped · only this task · 2 of 3 files kept"
    } else {
        "2 of 3 files · in progress"
    };
    write_component(
        canvas,
        15,
        4,
        &ActivityRow::new("Add rate-limit tests", tests_state, tests_detail)
            .with_elapsed("48s")
            .compact(),
    )?;
    let stop_focused = matches!(mode, "stop-selected" | "controls" | "controls-expanded");
    canvas.write(
        16,
        4,
        if stop_focused {
            "› [s] Stop this task · ends only the selected test work"
        } else {
            "  [s] Stop this task · ends only the selected test work"
        },
        if stop_focused {
            TextStyle::text()
        } else {
            TextStyle::muted()
        },
    )?;

    let build_state = if mode == "cancel-selected" {
        ActivityState::Failed
    } else {
        ActivityState::Waiting
    };
    let build_detail = if mode == "cancel-selected" {
        "Cancelled · only the queued build dropped"
    } else {
        "queued after the test files"
    };
    write_component(
        canvas,
        18,
        4,
        &ActivityRow::new("Build the test suite", build_state, build_detail).compact(),
    )?;
    canvas.write(
        19,
        4,
        if mode == "cancel-selected" {
            "› [c] Cancel this task · drops only the queued build"
        } else {
            "  [c] Cancel this task · drops only the queued build"
        },
        TextStyle::muted(),
    )?;

    let refresh_state = if mode == "retry-selected" {
        ActivityState::Running
    } else {
        ActivityState::Failed
    };
    let refresh_detail = if mode == "retry-selected" {
        "Retrying · only this blocked review"
    } else {
        "Blocked · needs a decision"
    };
    write_component(
        canvas,
        21,
        4,
        &ActivityRow::new("Review refresh session risk", refresh_state, refresh_detail)
            .compact(),
    )?;
    canvas.write(
        22,
        4,
        if mode == "retry-selected" {
            "› [r] Retry this task · restarts only the blocked refresh review"
        } else {
            "  [r] Retry this task · restarts only the blocked refresh review"
        },
        TextStyle::muted(),
    )?;
    composer(
        canvas,
        "Type a follow-up, it will wait until this turn finishes",
        None,
    )?;

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 58),
        "summary",
        None,
    );
    let stop_ev = semantic(
        screen_id,
        fixture_id,
        "activity",
        if mode == "stop-selected" {
            "task-add-tests-stopped"
        } else {
            "task-add-tests"
        },
        reference(15, 4, 74),
        "focused",
        Some(action(
            "action.tool-task.stop-selected",
            "Stop this task · ends only the selected test work",
            false,
        )),
    );
    let cancel_ev = semantic(
        screen_id,
        fixture_id,
        "activity",
        if mode == "cancel-selected" {
            "task-build-suite-cancelled"
        } else {
            "task-build-suite"
        },
        reference(18, 4, 70),
        "summary",
        Some(action(
            "action.tool-task.cancel-selected",
            "Cancel this task · drops only the queued build",
            false,
        )),
    );
    let retry_ev = semantic(
        screen_id,
        fixture_id,
        "activity",
        if mode == "retry-selected" {
            "task-refresh-risk-retrying"
        } else {
            "task-refresh-risk"
        },
        reference(21, 4, 74),
        "summary",
        Some(action(
            "action.tool-task.retry-selected",
            "Retry this task · restarts only the blocked refresh review",
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
    let pairs = [request, stop_ev, cancel_ev, retry_ev, status];
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.tool-task.stop-selected",
                screen_id,
                "tool-task",
                "direct",
                "stop applies only to the selected task and states the consequence first",
            ),
            transition(
                "action.tool-task.retry-selected",
                screen_id,
                "tool-task",
                "direct",
                "retry applies only to the selected blocked task",
            ),
            transition(
                "action.tool-task.cancel-selected",
                screen_id,
                "tool-task",
                "direct",
                "cancel removes only the selected queued task",
            ),
        ],
    ))
}
