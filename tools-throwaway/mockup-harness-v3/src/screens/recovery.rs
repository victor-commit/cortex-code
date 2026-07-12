//! Flow 5 pilot states for turn errors and network recovery.
//!
//! Turn failures keep partial replies inline with one adjacent primary recovery.
//! Network failures preserve work, hide attempts behind evidence, distinguish
//! automatic bounded backoff from manual retry, and route connection inspection
//! without losing recovery origin or submitting more than once.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
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
const PENDING_NETWORK: &str = "pending.network.release-notes";

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

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "turn-error" | "error-network" => Surface::ActionDock,
        _ => return Err(format!("unknown Flow 5 recovery screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "turn-error" => render_turn_error(&mut canvas, screen_id, fixture_id)?,
        "error-network" => render_error_network(&mut canvas, screen_id, fixture_id)?,
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
            conversation_id: "conversation.recovery".to_owned(),
            transcript_id: "transcript.recovery".to_owned(),
            session_id: "session.recovery.1".to_owned(),
            turn_id: "turn.recovery.1".to_owned(),
            user_task: RECOVERY_TASK.to_owned(),
            visible_origin: RECOVERY_ORIGIN.to_owned(),
            submitted_prompt: Some(USER_PROMPT.trim_start_matches("You · ").to_owned()),
            paused_activity_id: Some(PENDING_NETWORK.to_owned()),
            later_draft: Some(DRAFT.to_owned()),
        }),
    })
}

fn render_turn_error(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, RECOVERY_ORIGIN, TextStyle::muted())?;
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
        "› [enter] Retry reply",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [c] Continue from partial reply",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "  [esc] Cancel · keep failure record",
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
            reference(17, 4, 58),
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
    canvas.write(2, 4, RECOVERY_ORIGIN, TextStyle::muted())?;
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
                "› Evidence  [enter] Expand technical details",
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
                "› [enter] Review network setting",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                "  [esc] Cancel · keep paused request",
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
                "  [esc] Cancel",
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
                    "secondary-action",
                    "action.error-network.wait-retry",
                    reference(19, 4, 48),
                    "summary",
                    Some(action(
                        "action.error-network.wait-retry",
                        "Wait for the retry window",
                        false,
                    )),
                ));
                transitions.push(transition(
                    "action.error-network.wait-retry",
                    "error-network",
                    "rate-limit",
                    "direct",
                    "a bounded automatic wait moves to the rate limit state",
                ));
                transitions.push(transition(
                    "action.error-network.backoff-tick",
                    "error-network",
                    "error-network",
                    "automatic",
                    "the single backoff owner advances one deterministic attempt",
                ));
            } else {
                transitions.push(transition(
                    "action.error-network.backoff-tick",
                    "error-network",
                    "error-network",
                    "automatic",
                    "the single backoff owner advances one deterministic attempt",
                ));
            }
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
                "› [enter] Retry request",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                "  [d] Check connection",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                "  [esc] Cancel · keep paused request",
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
                "› [enter] Retry request",
                TextStyle::accent(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 4,
                "  [d] Check connection",
                TextStyle::muted(),
            )?;
            dock_line(
                canvas,
                DOCK_TOP + 5,
                if matches!(variant, NetworkVariant::Evidence) {
                    "  [esc] Hide evidence"
                } else {
                    "  [esc] Cancel · keep paused request"
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
