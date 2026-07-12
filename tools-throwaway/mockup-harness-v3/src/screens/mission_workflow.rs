//! Flow 6 pilot states for goal-led mission proposal, preflight, launch,
//! monitoring, failure repair, review, and human completion.
//!
//! Missions stay conversation-originated and exception-first. The default mission card
//! shows only the human goal, stage, completed/remaining counts, an optional blocker,
//! and one primary action. Preflight runs automatically; internals stay undisclosed.
//! Failure creates a visible fix task, always re-enters meaningful Validating, and
//! completion requires fresh validation plus explicit human approval.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{Component, MissionCard, ReviewBlock},
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = 31;
const MISSION_TASK: &str = "Prepare the release safely and review the result.";
const MISSION_ORIGIN: &str = "Mission · current conversation";
const MISSION_GOAL: &str = "Prepare the release safely and review the result.";
const MISSION_OUTCOME: &str =
    "Ship a reviewable release package with known risks and a clear next step.";
const USER_PROMPT: &str = "You · Turn the release work into a mission and walk me through it.";
const LATER_DRAFT: &str = "Keep the risk note short before we publish.";
const MISSING_INPUT: &str = "Release checklist note is missing";
const TASK_1: &str = "Review the release notes for open risks";
const TASK_2: &str = "Update the next-step note after fixes";
const TASK_3: &str = "Check the safe publish path";
const FIX_TASK: &str = "Repair the safe publish path checks";
const DEP_LINE: &str = "Depends on · review notes and next-step note";
const FAILURE_CAUSE: &str = "Publish path validation failed the safe release checks";
const REVIEW_REASON_CHANGES: &str =
    "Next-step note still omits the open risk and rollback order";
const REVIEW_REASON_APPROVED: &str = "Final validation passed on the post-repair revision";
const AFFECTED_RESULT: &str = "Safe publish path";
const PROPOSAL_SUBJECT: &str = "mission.proposal.release";
const MISSION_SUBJECT: &str = "mission.release.1";
const PENDING_LAUNCH: &str = "pending.mission.launch";
const FIX_SUBJECT: &str = "task.fix.safe-publish";
const VALIDATION_SUBJECT: &str = "validation.release.2";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchVariant {
    Ready,
    Proposal,
    PreflightRunning,
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "workflow-launch" | "workflow-launch-blocked" | "workflow-monitor-failure"
        | "reviewer-approved" => Surface::ActionDock,
        "workflow-monitor" => Surface::ConversationShell,
        "reviewer-output" => Surface::TemporaryDetailView,
        _ => return Err(format!("unknown Flow 6 mission screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "workflow-launch" => render_workflow_launch(&mut canvas, screen_id, fixture_id)?,
        "workflow-launch-blocked" => {
            render_workflow_launch_blocked(&mut canvas, screen_id, fixture_id)?
        }
        "workflow-monitor" => render_workflow_monitor(&mut canvas, screen_id, fixture_id)?,
        "workflow-monitor-failure" => {
            render_workflow_monitor_failure(&mut canvas, screen_id, fixture_id)?
        }
        "reviewer-output" => render_reviewer_output(&mut canvas, screen_id, fixture_id)?,
        "reviewer-approved" => render_reviewer_approved(&mut canvas, screen_id, fixture_id)?,
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
            conversation_id: "conversation.mission".to_owned(),
            transcript_id: "transcript.mission".to_owned(),
            session_id: "session.mission.1".to_owned(),
            turn_id: "turn.mission.1".to_owned(),
            user_task: MISSION_TASK.to_owned(),
            visible_origin: MISSION_ORIGIN.to_owned(),
            submitted_prompt: Some(USER_PROMPT.trim_start_matches("You · ").to_owned()),
            paused_activity_id: Some(PENDING_LAUNCH.to_owned()),
            later_draft: Some(LATER_DRAFT.to_owned()),
        }),
    })
}

fn render_workflow_launch(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    match launch_variant(fixture_id) {
        LaunchVariant::Proposal => render_proposal(canvas, screen_id, fixture_id),
        LaunchVariant::PreflightRunning => render_preflight_running(canvas, screen_id, fixture_id),
        LaunchVariant::Ready => render_launch_ready(canvas, screen_id, fixture_id),
    }
}

fn render_proposal(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    canvas.write(10, 4, "Mission proposal", TextStyle::muted())?;
    canvas.write(12, 4, MISSION_GOAL, TextStyle::bright())?;
    canvas.write(13, 4, MISSION_OUTCOME, TextStyle::text())?;
    canvas.write(15, 4, "Plan", TextStyle::bright())?;
    canvas.write(16, 4, &format!("1. {TASK_1}"), TextStyle::text())?;
    canvas.write(17, 4, &format!("2. {TASK_2}"), TextStyle::text())?;
    canvas.write(18, 4, &format!("3. {TASK_3}"), TextStyle::text())?;
    canvas.write(19, 4, &format!("   {DEP_LINE}"), TextStyle::muted())?;
    canvas.write(
        21,
        4,
        "Accepting the plan starts automatic preflight before any launch decision.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Review the mission plan", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Human tasks stay in this conversation. No generated files are shown.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Accept plan",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Keep discussing",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No launch control is available yet.",
        TextStyle::muted(),
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
            "mission-proposal",
            PROPOSAL_SUBJECT,
            reference(12, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            PROPOSAL_SUBJECT,
            reference(12, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "task-plan",
            "task.review-notes",
            reference(16, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "task-dependency",
            "task.check-publish",
            reference(19, 4, 52),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "accept-plan",
            reference(DOCK_TOP + 3, 4, 26),
            "summary",
            Some(action("action.mission.accept-plan", "Accept plan", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "keep-discussing",
            reference(DOCK_TOP + 4, 4, 28),
            "summary",
            Some(action(
                "action.mission.proposal-cancel",
                "Keep discussing",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.accept-plan",
            screen_id,
            "workflow-launch",
            "automatic",
            "accepting the proposal starts automatic preflight before launch",
        ),
        transition(
            "action.mission.proposal-cancel",
            screen_id,
            "chat-turn",
            "cancel",
            "leaving the proposal keeps the draft conversation ready to revise",
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

fn render_preflight_running(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    write_mission_summary(canvas, 10, "Preflight", 0, 3, None)?;
    canvas.write(
        14,
        4,
        "● Preflight is running automatically after the plan was accepted.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "Launch stays unavailable until the automatic checks finish.",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "Successful checks collapse. Only missing inputs expand.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Automatic preflight", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Code is checking launch readiness. No manual preflight control.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Waiting for automatic checks…",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  No Start mission action yet",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "  [esc] Cancel · return to conversation",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "mission-card",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preflight",
            "preflight.auto.running",
            reference(14, 4, 64),
            "summary",
            Some(action(
                "action.mission.preflight-auto",
                "Automatic preflight",
                true,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "cancel-preflight",
            reference(DOCK_TOP + 5, 4, 42),
            "summary",
            Some(action(
                "action.mission.preflight-cancel",
                "Cancel",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.preflight-auto",
            screen_id,
            "workflow-launch",
            "automatic",
            "automatic preflight resolves to ready or blocked without a manual start",
        ),
        transition(
            "action.mission.preflight-cancel",
            screen_id,
            "chat-turn",
            "cancel",
            "cancel restores the proposing conversation before launch",
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

fn render_launch_ready(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    write_mission_summary(canvas, 10, "Ready to launch", 0, 3, None)?;
    canvas.write(
        13,
        4,
        "✓ Preflight passed. Detailed checks stay collapsed.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "No missing inputs. Starting will run the three planned tasks.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "Goal, stage, and counts are the only mission fields on the card.",
        TextStyle::muted(),
    )?;
    canvas.write(
        17,
        4,
        "[d] View preflight evidence",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(canvas, DOCK_TOP + 1, "Start this mission?", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Start runs the planned tasks and opens compact progress monitoring.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Start mission",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · return to conversation",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Cancel keeps the proposal and starts no work.",
        TextStyle::muted(),
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
            "mission-card",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preflight-success",
            "preflight.auto.passed",
            reference(13, 4, 52),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "disclosure",
            "preflight.evidence",
            reference(17, 4, 32),
            "summary",
            Some(action(
                "action.mission.view-preflight-evidence",
                "View preflight evidence",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "start-mission",
            reference(DOCK_TOP + 3, 4, 28),
            "summary",
            Some(action("action.mission.start", "Start mission", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "cancel-launch",
            reference(DOCK_TOP + 4, 4, 42),
            "summary",
            Some(action("action.mission.launch-cancel", "Cancel", false)),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.start",
            screen_id,
            "workflow-monitor",
            "direct",
            "explicit launch decision enters compact healthy monitoring",
        ),
        transition(
            "action.mission.launch-cancel",
            screen_id,
            "chat-turn",
            "cancel",
            "cancel returns to the proposing conversation without starting the mission",
        ),
        transition(
            "action.mission.view-preflight-evidence",
            screen_id,
            "workflow-launch",
            "direct",
            "preflight evidence discloses successfully collapsed checks only on request",
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

fn render_workflow_launch_blocked(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    write_mission_summary(canvas, 10, "Preflight", 0, 3, Some(MISSING_INPUT))?;
    canvas.write(
        14,
        4,
        "One missing input is blocking launch readiness.",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "Provide the release checklist note, then preflight reruns automatically.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "The launch decision stays unavailable until that automatic rerun passes.",
        TextStyle::muted(),
    )?;
    canvas.write(
        18,
        4,
        "No manual preflight control is offered.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Resolve the missing input",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Add the release checklist note required by preflight.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Add release checklist",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Cancel · return to conversation",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Launch remains omitted while blocked.",
        TextStyle::muted(),
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
            "mission-card",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "blocker",
            "blocker.release-checklist",
            reference(12, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "add-release-checklist",
            reference(DOCK_TOP + 3, 4, 36),
            "summary",
            Some(action(
                "action.mission.resolve-missing-input",
                "Add release checklist",
                true,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "cancel-blocked",
            reference(DOCK_TOP + 4, 4, 42),
            "summary",
            Some(action(
                "action.mission.blocked-cancel",
                "Cancel",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.resolve-missing-input",
            screen_id,
            "workflow-launch",
            "automatic",
            "resolving the missing input reruns automatic preflight before launch",
        ),
        transition(
            "action.mission.blocked-cancel",
            screen_id,
            "chat-turn",
            "cancel",
            "cancel returns to the proposing conversation without launching",
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

fn render_workflow_monitor(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    let lower = fixture_id.to_ascii_lowercase();
    let expanded = lower.contains("progress");
    let validating = lower.contains("validating");
    let fix_planned = lower.contains("fix-planned") || lower.contains("fix_planned");
    let fix_working = lower.contains("fix-working") || lower.contains("fix_working");

    if validating {
        // Automatic validation stage: zero primary actions (VAL-V3-F6-052).
        write_mission_summary(canvas, 10, "Validating", 1, 1, None)?;
        canvas.write(
            13,
            4,
            "● Fresh validation is running on the repaired result.",
            TextStyle::text(),
        )?;
        canvas.write(
            14,
            4,
            "Review and approval stay closed until this run finishes.",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "Validation is automatic. There is no primary repair action yet.",
            TextStyle::muted(),
        )?;
        canvas.write(18, 4, "› composer ready", TextStyle::muted())?;
        canvas.write(19, 4, &format!("  {LATER_DRAFT}"), TextStyle::text())?;

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
                "mission-card",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "mission-goal",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "progress-stage",
                "stage.validating",
                reference(11, 4, 40),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "validation-run",
                VALIDATION_SUBJECT,
                reference(13, 4, 56),
                "summary",
                Some(action(
                    "action.mission.validation-running",
                    "Fresh validation",
                    true,
                )),
            ),
        ];
        let transitions = vec![transition(
            "action.mission.validation-running",
            screen_id,
            "workflow-monitor",
            "automatic",
            "fresh validation ends at review on success or Needs attention on failure",
        )];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    if fix_planned || fix_working {
        let (stage, completed, remaining, primary) = if fix_working {
            ("Running", 1usize, 2usize, "View progress")
        } else {
            // Planned fix adds explicit work (completed stays 1; remaining includes the new fix).
            ("Running", 1usize, 3usize, "View progress")
        };
        let card = MissionCard::new(MISSION_GOAL, stage, completed, remaining, None, primary);
        write_component(canvas, 10, 4, &card)?;
        if fix_working {
            canvas.write(15, 4, "Working", TextStyle::bright())?;
            canvas.write(16, 4, &format!("  ● {FIX_TASK}"), TextStyle::text())?;
            canvas.write(
                17,
                4,
                "    Uses the validation failure as input",
                TextStyle::muted(),
            )?;
            canvas.write(19, 4, "Recently completed", TextStyle::bright())?;
            canvas.write(20, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        } else {
            canvas.write(15, 4, "Needs attention", TextStyle::bright())?;
            canvas.write(
                16,
                4,
                &format!("  ○ fix planned · {FIX_TASK}"),
                TextStyle::text(),
            )?;
            canvas.write(
                17,
                4,
                "    Depends on · failed publish path validation",
                TextStyle::muted(),
            )?;
            canvas.write(19, 4, "Recently completed", TextStyle::bright())?;
            canvas.write(20, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        }
        canvas.write(22, 4, "› composer ready", TextStyle::muted())?;
        canvas.write(23, 4, &format!("  {LATER_DRAFT}"), TextStyle::text())?;

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
                "mission-card",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "mission-goal",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "progress-stage",
                if fix_working {
                    "stage.fix-working"
                } else {
                    "stage.fix-planned"
                },
                reference(11, 4, 40),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "fix-task",
                FIX_SUBJECT,
                reference(16, 4, 48),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "view-progress",
                reference(12, 4, 28),
                "summary",
                Some(action(
                    "action.mission.view-progress",
                    "View progress",
                    false,
                )),
            ),
        ];
        let mut pairs = pairs.to_vec();
        let mut transitions = vec![transition(
            "action.mission.view-progress",
            screen_id,
            "workflow-monitor",
            "direct",
            "progress disclosure returns to the same compact monitor",
        )];
        if fixture_id.contains("gate.mission.workflow-monitor")
            && !fixture_id.contains("failure")
        {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "bridge-note",
                "action.storyboard.open-mission-failure",
                reference(19, 4, 48),
                "summary",
                Some(action(
                    "action.storyboard.open-mission-failure",
                    "Open the validation failure",
                    false,
                )),
            ));
            transitions.push(transition(
                "action.storyboard.open-mission-failure",
                screen_id,
                "workflow-monitor-failure",
                "illustrative",
                "a summarized visible validation stage surfaces only the actionable failure for review",
            ));
        }
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    let card = MissionCard::new(MISSION_GOAL, "Running", 1, 2, None, "View progress");
    write_component(canvas, 10, 4, &card)?;
    if expanded {
        // Healthy expanded progress omits the empty Needs attention group.
        canvas.write(15, 4, "Working", TextStyle::bright())?;
        canvas.write(16, 4, &format!("  ● {TASK_2}"), TextStyle::text())?;
        canvas.write(18, 4, "Recently completed", TextStyle::bright())?;
        canvas.write(19, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        canvas.write(21, 4, "[esc] Collapse progress", TextStyle::muted())?;
    } else {
        canvas.write(
            15,
            4,
            "Progress stays calm until you ask for a task summary.",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "No blocker. Open progress only when you need the details.",
            TextStyle::muted(),
        )?;
        canvas.write(18, 4, "› composer ready", TextStyle::muted())?;
        canvas.write(
            19,
            4,
            "  Keep the risk note short before we publish.",
            TextStyle::text(),
        )?;
    }

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
            "mission-card",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "progress-stage",
            "stage.running",
            reference(11, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "view-progress",
            reference(12, 4, 28),
            "summary",
            Some(action(
                "action.mission.view-progress",
                "View progress",
                false,
            )),
        ),
    ];
    let transitions = vec![transition(
        "action.mission.view-progress",
        screen_id,
        "workflow-monitor",
        "direct",
        "progress disclosure returns to the same compact monitor",
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

fn render_workflow_monitor_failure(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    // Compact card fields only; primary action lives in the dock.
    write_mission_summary(
        canvas,
        10,
        "Needs attention",
        1,
        2,
        Some(FAILURE_CAUSE),
    )?;
    canvas.write(
        14,
        4,
        "Needs attention",
        TextStyle::bright(),
    )?;
    canvas.write(
        15,
        4,
        &format!("  ✗ {TASK_3}"),
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        &format!("    {FAILURE_CAUSE}"),
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        4,
        "Recently completed",
        TextStyle::bright(),
    )?;
    canvas.write(
        19,
        4,
        &format!("  ✓ {TASK_1}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        21,
        4,
        "Completed work stays preserved. Create a fix task to repair.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Repair the validation failure",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Create planned fix work from this failure and re-run validation.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Create a fix task",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [d] Open evidence",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "  [esc] Resume conversation",
        TextStyle::muted(),
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
            "mission-card",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            MISSION_SUBJECT,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "blocker",
            "blocker.validation-failed",
            reference(12, 4, 64),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "failure",
            "failure.publish-path",
            reference(16, 4, 64),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "create-fix-task",
            reference(DOCK_TOP + 3, 4, 34),
            "summary",
            Some(action(
                "action.mission.create-fix-task",
                "Create a fix task",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "open-evidence",
            reference(DOCK_TOP + 4, 4, 24),
            "summary",
            Some(action(
                "action.mission.open-evidence",
                "Open evidence",
                false,
            )),
        ),
    ];
    let mut pairs = pairs.to_vec();
    let mut transitions = vec![
        transition(
            "action.mission.create-fix-task",
            screen_id,
            "workflow-monitor",
            "direct",
            "creating a fix task records the failure as planned repair work",
        ),
        transition(
            "action.mission.open-evidence",
            screen_id,
            "workflow-monitor-failure",
            "direct",
            "evidence disclosure returns to the same failure state",
        ),
    ];
    if fixture_id.contains("gate.mission.workflow-monitor-failure") {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "action.storyboard.summarize-fix-to-review",
            reference(19, 4, 48),
            "summary",
            Some(action(
                "action.storyboard.summarize-fix-to-review",
                "Create a fix task",
                false,
            )),
        ));
        transitions.push(transition(
            "action.storyboard.summarize-fix-to-review",
            screen_id,
            "reviewer-output",
            "summarized",
            "fix planning, fix working, and fresh revalidation complete before the review outcome opens",
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

fn render_reviewer_output(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    // Temporary detail view for verdict-first review that needs attention.
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, "Mission review", TextStyle::bright())?;
    canvas.write(
        4,
        4,
        "Review needs attention before completion. Mission card stays compact.",
        TextStyle::muted(),
    )?;

    write_mission_summary(canvas, 7, "Review", 2, 1, None)?;

    let review = ReviewBlock::new(
        "Changes requested",
        REVIEW_REASON_CHANGES,
        "Create a fix task",
    )
    .with_affected_result(AFFECTED_RESULT);
    write_component(canvas, 12, 4, &review)?;
    canvas.write(
        17,
        4,
        "Checklist and deeper evidence stay collapsed until you open them.",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "[d] View validation summary",
        TextStyle::muted(),
    )?;
    canvas.write(
        21,
        4,
        "[esc] Return to mission monitor",
        TextStyle::muted(),
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
            "mission-card",
            MISSION_SUBJECT,
            reference(7, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "mission-goal",
            MISSION_SUBJECT,
            reference(7, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "review-outcome",
            "review.changes-requested",
            reference(12, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "create-fix-from-review",
            reference(14, 4, 32),
            "summary",
            Some(action(
                "action.mission.review-create-fix",
                "Create a fix task",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "return-monitor",
            reference(21, 4, 36),
            "summary",
            Some(action(
                "action.mission.review-return",
                "Return to mission monitor",
                false,
            )),
        ),
    ];
    let mut pairs = pairs.to_vec();
    let mut transitions = vec![
        transition(
            "action.mission.review-create-fix",
            screen_id,
            "workflow-monitor",
            "direct",
            "requested changes become an explicit fix task followed by fresh validation",
        ),
        transition(
            "action.mission.review-return",
            screen_id,
            "workflow-monitor",
            "cancel",
            "leaving review returns to the originating monitor without approving",
        ),
    ];
    if fixture_id.contains("gate.mission.reviewer-output") {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "action.storyboard.summarize-review-to-approval",
            reference(19, 4, 48),
            "summary",
            Some(action(
                "action.storyboard.summarize-review-to-approval",
                "Continue through review until human completion is ready",
                false,
            )),
        ));
        transitions.push(transition(
            "action.storyboard.summarize-review-to-approval",
            screen_id,
            "reviewer-approved",
            "summarized",
            "requested changes re-enter fix and revalidation; once review is ready, the approval frame opens for human completion",
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

fn render_reviewer_approved(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    let lower = fixture_id.to_ascii_lowercase();
    let request_changes_dock = lower.contains("request-changes") && !lower.contains("cancel");

    // Compact MissionCard fields only (goal, final stage, counts). No primary on card.
    write_mission_summary(canvas, 10, "Ready for completion", 3, 0, None)?;

    let outcome = ReviewBlock::new(
        "Approved",
        REVIEW_REASON_APPROVED,
        "Approve completion",
    )
    .with_affected_result(AFFECTED_RESULT)
    .outcome_only();
    write_component(canvas, 14, 4, &outcome)?;
    canvas.write(
        18,
        4,
        "Detailed checks stay collapsed. Human approval still required.",
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    if request_changes_dock {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "What should change?",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Describe the remaining fix. This updates planned repair work.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Submit reason",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [esc] Cancel · keep approval gate",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Cancel restores Approve completion unchanged.",
            TextStyle::muted(),
        )?;
    } else {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Approve mission completion?",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Only the human approval finishes the mission and restores chat.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Approve completion",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [d] Request changes",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "  [esc] Stay on this review",
            TextStyle::muted(),
        )?;
    }

    let pairs = if request_changes_dock {
        vec![
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
                "mission-card",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "review-outcome",
                "review.approved",
                reference(14, 4, 24),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "human-gate",
                "awaiting_human_approval",
                reference(DOCK_TOP + 1, 4, 32),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "submit-change-reason",
                reference(DOCK_TOP + 3, 4, 32),
                "summary",
                Some(action(
                    "action.mission.request-changes-submit",
                    "Submit reason",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "cancel-change-reason",
                reference(DOCK_TOP + 4, 4, 40),
                "summary",
                Some(action(
                    "action.mission.request-changes-cancel",
                    "Cancel",
                    false,
                )),
            ),
        ]
    } else {
        vec![
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
                "mission-card",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "mission-goal",
                MISSION_SUBJECT,
                reference(10, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "review-outcome",
                "review.approved",
                reference(14, 4, 24),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "human-gate",
                "awaiting_human_approval",
                reference(DOCK_TOP + 1, 4, 36),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "approve-completion",
                reference(DOCK_TOP + 3, 4, 36),
                "summary",
                Some(action(
                    "action.mission.approve-completion",
                    "Approve completion",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "request-changes",
                reference(DOCK_TOP + 4, 4, 28),
                "summary",
                Some(action(
                    "action.mission.request-changes",
                    "Request changes",
                    false,
                )),
            ),
        ]
    };
    let transitions = if request_changes_dock {
        vec![
            transition(
                "action.mission.request-changes-submit",
                screen_id,
                "workflow-monitor",
                "direct",
                "submitted human feedback creates or updates a fix task then revalidation",
            ),
            transition(
                "action.mission.request-changes-cancel",
                screen_id,
                "reviewer-approved",
                "cancel",
                "cancel restores the unchanged human approval gate",
            ),
        ]
    } else {
        vec![
            transition(
                "action.mission.approve-completion",
                screen_id,
                "chat-turn",
                "direct",
                "explicit human approval marks the mission complete and returns to conversation",
            ),
            transition(
                "action.mission.request-changes",
                screen_id,
                "reviewer-approved",
                "direct",
                "request changes opens an anchored reason dock on the same gate",
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

fn write_conversation_header(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "I can run that as a small mission with three connected tasks.",
        TextStyle::text(),
    )?;
    Ok(())
}

/// Compact mission payload without the card's action row.
/// Dock screens keep the primary action in the bottom decision dock only.
fn write_mission_summary(
    canvas: &mut Canvas,
    first_row: u16,
    stage: &str,
    completed: usize,
    remaining: usize,
    blocker: Option<&str>,
) -> Result<(), String> {
    canvas.write(first_row, 4, MISSION_GOAL, TextStyle::bright())?;
    canvas.write(
        first_row + 1,
        4,
        &format!("{stage} · {completed} complete · {remaining} remaining"),
        TextStyle::text(),
    )?;
    if let Some(blocker) = blocker {
        canvas.write(
            first_row + 2,
            4,
            &format!("Needs attention: {blocker}"),
            TextStyle::text(),
        )?;
    }
    Ok(())
}

fn launch_variant(fixture_id: &str) -> LaunchVariant {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("proposal") {
        LaunchVariant::Proposal
    } else if lower.contains("preflight-running")
        || lower.contains("preflight_running")
        || lower.contains("preflight-run")
    {
        LaunchVariant::PreflightRunning
    } else {
        LaunchVariant::Ready
    }
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
