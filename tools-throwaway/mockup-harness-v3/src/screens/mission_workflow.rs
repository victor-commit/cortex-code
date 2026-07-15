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
/// Default bound for tasks shown per progress group (VAL-V3-F6-056).
const PROGRESS_BOUND: usize = 3;
const PROGRESS_NEEDS_VISIBLE: [&str; 3] = [
    "Check the safe publish path",
    "Confirm rollback wording",
    "Resolve blocked publish gate",
];
const PROGRESS_NEEDS_FULL: [&str; 6] = [
    "Check the safe publish path",
    "Confirm rollback wording",
    "Resolve blocked publish gate",
    "Reopen failed path evidence",
    "Note remaining security hold",
    "Clear stale approval on the path",
];
const PROGRESS_WORKING_VISIBLE: [&str; 3] = [
    "Update the next-step note after fixes",
    "Draft the remaining risk callouts",
    "Align the publish checklist order",
];
const PROGRESS_WORKING_FULL: [&str; 6] = [
    "Update the next-step note after fixes",
    "Draft the remaining risk callouts",
    "Align the publish checklist order",
    "Prepare the release summary blurb",
    "Document the owner handoff",
    "Tighten the final soft-landing copy",
];
const PROGRESS_COMPLETED_VISIBLE: [&str; 3] = [
    "Review the release notes for open risks",
    "Collect the known open questions",
    "Tag the reviewed checklist items",
];
const PROGRESS_COMPLETED_FULL: [&str; 6] = [
    "Review the release notes for open risks",
    "Collect the known open questions",
    "Tag the reviewed checklist items",
    "Archive the accepted planning note",
    "Close the preflight review thread",
    "Record the healthy start snapshot",
];
const REVIEW_REASON_CHANGES: &str =
    "Next-step note still omits the open risk and rollback order";
const REVIEW_REASON_APPROVED: &str = "Final validation passed on the post-repair revision";
const AFFECTED_RESULT: &str = "Safe publish path";
const PROPOSAL_SUBJECT: &str = "mission.proposal.release";
const MISSION_SUBJECT: &str = "mission.release.1";
const PENDING_LAUNCH: &str = "pending.mission.launch";
const FIX_SUBJECT: &str = "task.fix.safe-publish";
const VALIDATION_SUBJECT: &str = "validation.release.2";
const SAFE_BOUNDARY_ID: &str = "boundary.after-task.note-update";
const INVOCATION_ID: &str = "invocation.mission.release.1";
const COMPLETED_TASK_ID: &str = "task.review-notes";
const REPLAN_ADDED_TASK: &str = "Document the owner handoff checklist";
const REPLAN_REMOVED_TASK: &str = "Update the next-step note after fixes";
const REPLAN_REPLACED_FROM: &str = "Check the safe publish path";
const REPLAN_REPLACED_TO: &str = "Recheck the safe publish path after the path evidence";
const REPLAN_DEFERRED_TASK: &str = "Align the publish checklist order";
const STOPPED_REMAINING: &str = "Remaining work was not run · abandoned on stop";

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
    let lower_fixture = fixture_id.to_ascii_lowercase();
    let surface = match screen_id {
        "workflow-launch" | "workflow-launch-blocked" | "workflow-monitor-failure"
        | "reviewer-approved" => Surface::ActionDock,
        // Intervention/lifecycle docks replace the temporary detail with ActionDock.
        "workflow-monitor" if monitor_uses_action_dock(&lower_fixture) => Surface::ActionDock,
        // Healthy / progress / validating monitors are temporary detail views: the
        // mission card owns the canvas (no bottom composer). Declaring
        // conversation_shell while painting mid-screen "› composer ready" fails
        // VAL-V3-DS-009/010.
        "workflow-monitor" => Surface::TemporaryDetailView,
        "reviewer-output"
        | "team-board"
        | "verification-ladder"
        | "intent-scorer-pass"
        | "intent-scorer-fail"
        | "concurrency-metrics"
        | "tool-envelope" => Surface::TemporaryDetailView,
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
        "team-board" => render_team_board(&mut canvas, screen_id, fixture_id)?,
        "reviewer-output" => render_reviewer_output(&mut canvas, screen_id, fixture_id)?,
        "reviewer-approved" => render_reviewer_approved(&mut canvas, screen_id, fixture_id)?,
        "verification-ladder" => {
            render_verification_ladder(&mut canvas, screen_id, fixture_id)?
        }
        "intent-scorer-pass" => render_intent_scorer_pass(&mut canvas, screen_id, fixture_id)?,
        "intent-scorer-fail" => render_intent_scorer_fail(&mut canvas, screen_id, fixture_id)?,
        "concurrency-metrics" => {
            render_concurrency_metrics(&mut canvas, screen_id, fixture_id)?
        }
        "tool-envelope" => render_tool_envelope(&mut canvas, screen_id, fixture_id)?,
        _ => unreachable!(),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;
    // Keep the surface event 1:1 with the composition-declared arrangement for this
    // fixture. Registry alone can lag dock overrides (intervene/pause/stop) and
    // ensure_core would otherwise inherit the wrong default.
    let (candidates, events) =
        ensure_surface_event(screen_id, fixture_id, surface, candidates, events);
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

/// Injects or rewrites the single surface event so arrange/paint never diverge.
fn ensure_surface_event(
    screen_id: &str,
    fixture_id: &str,
    surface: Surface,
    mut candidates: Vec<SemanticCandidate>,
    mut events: Vec<SemanticEvent>,
) -> (Vec<SemanticCandidate>, Vec<SemanticEvent>) {
    let subject = surface.as_str();
    let present = events
        .iter()
        .filter(|event| event.identity.family == "surface")
        .count();
    if present == 1 {
        // Normalize subject when paint paths emit the surface early.
        for event in &mut events {
            if event.identity.family == "surface" && event.identity.subject_id != subject {
                event.identity.subject_id = subject.to_owned();
                event.identity.candidate_id = format!("candidate.surface.{subject}");
                event.identity.event_id = format!("event.surface.{subject}");
            }
        }
        for candidate in &mut candidates {
            if candidate.identity.family == "surface" && candidate.identity.subject_id != subject {
                candidate.identity.subject_id = subject.to_owned();
                candidate.identity.candidate_id = format!("candidate.surface.{subject}");
                candidate.identity.event_id = format!("event.surface.{subject}");
            }
        }
        return (candidates, events);
    }
    // Drop any stale duplicate surface events and add exactly one.
    candidates.retain(|candidate| candidate.identity.family != "surface");
    events.retain(|event| event.identity.family != "surface");
    let (candidate, event) = semantic(
        screen_id,
        fixture_id,
        "surface",
        subject,
        reference(0, 0, 1),
        "summary",
        None,
    );
    candidates.insert(0, candidate);
    events.insert(0, event);
    (candidates, events)
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
    let lower = fixture_id.to_ascii_lowercase();
    let replan = lower.contains("replan");

    if replan {
        canvas.write(10, 4, "Mission proposal · revised plan", TextStyle::muted())?;
        canvas.write(12, 4, MISSION_GOAL, TextStyle::bright())?;
        canvas.write(13, 4, MISSION_OUTCOME, TextStyle::text())?;
        canvas.write(15, 4, "Completed kept", TextStyle::bright())?;
        canvas.write(16, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        canvas.write(18, 4, "Plan changes", TextStyle::bright())?;
        canvas.write(
            19,
            4,
            &format!("  + 1 added · {REPLAN_ADDED_TASK}"),
            TextStyle::text(),
        )?;
        canvas.write(
            20,
            4,
            &format!("  - 1 removed · {REPLAN_REMOVED_TASK}"),
            TextStyle::text(),
        )?;
        canvas.write(
            21,
            4,
            &format!("  ~ 1 replaced · {REPLAN_REPLACED_FROM} → {REPLAN_REPLACED_TO}"),
            TextStyle::text(),
        )?;
        canvas.write(
            22,
            4,
            &format!("  … 1 deferred · {REPLAN_DEFERRED_TASK}"),
            TextStyle::text(),
        )?;
        canvas.write(
            24,
            4,
            "Completed tasks stay outside mutation counts. Accept runs fresh preflight first.",
            TextStyle::muted(),
        )?;

        write_dock_border(canvas)?;
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Accept the revised plan?",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Accept applies exact deltas, then automatic preflight before any resume.",
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
            "No execution starts until preflight finishes.",
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
                "task.replan.release",
                reference(18, 4, 48),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "replan-counts",
                "replan.deltas.1-1-1-1",
                reference(19, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "completed-work",
                COMPLETED_TASK_ID,
                reference(16, 4, 40),
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
                "accepting the revised plan runs automatic preflight before execution resumes",
            ),
            transition(
                "action.mission.proposal-cancel",
                screen_id,
                "chat-turn",
                "cancel",
                "leaving the replan proposal keeps the draft conversation ready to revise",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

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
        "  Launch decision stays closed until checks finish",
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

fn monitor_uses_action_dock(lower: &str) -> bool {
    lower.contains("intervene")
        || lower.contains("pause-confirm")
        || lower.contains("pause_confirm")
        || lower.contains("pause-open")
        || lower.contains("pause_open")
        || lower.contains("stop-confirm")
        || lower.contains("stop_confirm")
        || lower.contains("stop-open")
        || lower.contains("stop_open")
        || lower.contains(".paused")
        || lower.contains("monitor.paused")
        || lower.contains("monitor-paused")
        || lower.contains(".stopped")
        || lower.contains("monitor.stopped")
        || lower.contains("monitor-stopped")
        || lower.contains("restart-paused")
        || lower.contains("restart-stopped")
        || lower.contains("restart-running")
        || lower.contains("restart-blocked")
        || lower.contains("restart-validating")
        || lower.contains("restart-reviewing")
        || lower.contains("restart-awaiting")
}

fn render_workflow_monitor(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    let lower = fixture_id.to_ascii_lowercase();
    let expanded = lower.contains("progress");
    let validating = lower.contains("validating") && !lower.contains("restart-validating");
    let fix_planned = lower.contains("fix-planned") || lower.contains("fix_planned");
    let fix_working = lower.contains("fix-working") || lower.contains("fix_working");

    // Intervention and lifecycle docks (VAL-V3-F6-016/037/040/054/055).
    if lower.contains("intervene")
        || lower.contains("pause")
        || lower.contains("stop")
        || lower.contains("paused")
        || lower.contains("stopped")
        || lower.contains("resume")
        || lower.contains("restart-")
    {
        return render_monitor_lifecycle(canvas, screen_id, fixture_id, &lower);
    }

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
        // Temporary-detail anatomy: no mid-screen composer pretence (DS-009/010).
        canvas.write(
            18,
            4,
            "Later draft is preserved in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(20, 4, "[esc] Resume conversation", TextStyle::muted())?;

        let pairs = [
            semantic(
                screen_id,
                fixture_id,
                "surface",
                "temporary-detail-view",
                reference(10, 4, 28),
                "summary",
                None,
            ),
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
            semantic(
                screen_id,
                fixture_id,
                "return-action",
                "monitor-return",
                reference(20, 4, 34),
                "summary",
                Some(action(
                    "action.mission.monitor-return",
                    "Resume conversation",
                    false,
                )),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.validation-running",
                screen_id,
                "workflow-monitor",
                "automatic",
                "fresh validation ends at review on success or Needs attention on failure",
            ),
            transition(
                "action.mission.monitor-return",
                screen_id,
                "chat-turn",
                "cancel",
                "return restores the originating conversation with preserved draft",
            ),
        ];
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
        canvas.write(22, 4, "[i] Intervene", TextStyle::muted())?;
        canvas.write(23, 4, "[esc] Resume conversation", TextStyle::muted())?;

        let pairs = [
            semantic(
                screen_id,
                fixture_id,
                "surface",
                "temporary-detail-view",
                reference(10, 4, 28),
                "summary",
                None,
            ),
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
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "open-intervene",
                reference(22, 4, 18),
                "summary",
                Some(action(
                    "action.mission.open-intervene",
                    "Intervene",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "return-action",
                "monitor-return",
                reference(23, 4, 34),
                "summary",
                Some(action(
                    "action.mission.monitor-return",
                    "Resume conversation",
                    false,
                )),
            ),
        ];
        let mut pairs = pairs.to_vec();
        let mut transitions = vec![
            transition(
                "action.mission.view-progress",
                screen_id,
                "workflow-monitor",
                "direct",
                "progress disclosure returns to the same compact monitor",
            ),
            transition(
                "action.mission.open-intervene",
                screen_id,
                "workflow-monitor",
                "direct",
                "intervene opens a focused consequence-aware dock under the monitor",
            ),
            transition(
                "action.mission.monitor-return",
                screen_id,
                "chat-turn",
                "cancel",
                "return restores the originating conversation with preserved draft",
            ),
        ];
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

    let attention = lower.contains("attention");
    let bounded = lower.contains("bounded");
    let expand_working = lower.contains("expand-working") || lower.contains("expand_working");

    if expanded || attention || bounded || expand_working {
        return render_progress_disclosure(
            canvas,
            screen_id,
            fixture_id,
            attention,
            bounded,
            expand_working,
        );
    }

    let card = MissionCard::new(MISSION_GOAL, "Running", 1, 2, None, "View progress");
    write_component(canvas, 10, 4, &card)?;
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
    // Temporary detail keeps later-draft intent as a quiet preservation note,
    // not a mid-screen conversation-shell composer (VAL-V3-DS-009/010).
    canvas.write(
        18,
        4,
        "Later draft stays ready in the originating conversation.",
        TextStyle::muted(),
    )?;
    // Secondary intervention entry stays subordinate to View progress (VAL-V3-F6-016).
    canvas.write(20, 4, "[i] Intervene", TextStyle::muted())?;
    // Dedicated return control for temporary-detail surface anatomy.
    canvas.write(21, 4, "[esc] Resume conversation", TextStyle::muted())?;
    // Advanced runtime/concurrency opens only from a disclosure-safe surface.
    // Approved gate compact frames omit this paint so product digests stay stable;
    // non-gate and runtime-open fixtures paint a distinct 1:1 open control.
    let runtime_open = wants_runtime_open(fixture_id);
    if runtime_open {
        canvas.write(
            22,
            4,
            "[a] View advanced detail",
            TextStyle::muted(),
        )?;
    }

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "surface",
            "temporary-detail-view",
            reference(10, 4, 28),
            "summary",
            None,
        ),
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
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "open-intervene",
            reference(20, 4, 18),
            "summary",
            Some(action(
                "action.mission.open-intervene",
                "Intervene",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "return-action",
            "monitor-return",
            reference(21, 4, 34),
            "summary",
            Some(action(
                "action.mission.monitor-return",
                "Resume conversation",
                false,
            )),
        ),
    ];
    let mut transitions = vec![
        transition(
            "action.mission.view-progress",
            screen_id,
            "workflow-monitor",
            "direct",
            "progress disclosure returns to the same compact monitor",
        ),
        transition(
            "action.mission.open-intervene",
            screen_id,
            "workflow-monitor",
            "direct",
            "intervene opens a focused consequence-aware dock under the healthy monitor",
        ),
        transition(
            "action.mission.monitor-return",
            screen_id,
            "chat-turn",
            "cancel",
            "return restores the originating conversation with preserved draft",
        ),
    ];
    if runtime_open {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "view-runtime-detail",
            reference(22, 4, 40),
            "summary",
            Some(action(
                "action.mission.view-runtime-detail",
                "View advanced detail",
                false,
            )),
        ));
        transitions.push(transition(
            "action.mission.view-runtime-detail",
            screen_id,
            "concurrency-metrics",
            "direct",
            "concurrency metrics open only from an explicit advanced runtime action",
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

/// Pause, intervene, stop, resume, restart, and stopped inspection docks.
fn render_monitor_lifecycle(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    lower: &str,
) -> Result<ScreenEvidence, String> {
    let paused = lower.contains("paused")
        || lower.contains("restart-paused")
        || (lower.contains("pause")
            && !lower.contains("pause-open")
            && !lower.contains("pause_open")
            && !lower.contains("pause-confirm")
            && !lower.contains("pause_confirm")
            && !lower.contains("resume")
            && !lower.contains("intervene"));
    let pause_open = lower.contains("pause-open") || lower.contains("pause_open");
    let pause_confirm = lower.contains("pause-confirm") || lower.contains("pause_confirm");
    let stopped = lower.contains("stopped") || lower.contains("restart-stopped");
    let stop_open = lower.contains("stop-open") || lower.contains("stop_open");
    let stop_confirm = lower.contains("stop-confirm") || lower.contains("stop_confirm");
    let restart_running = lower.contains("restart-running");
    let restart_blocked = lower.contains("restart-blocked");
    let restart_validating = lower.contains("restart-validating");
    let restart_reviewing = lower.contains("restart-reviewing");
    let restart_awaiting = lower.contains("restart-awaiting");
    let resume_running =
        lower.contains("resumed") || lower.contains("after-resume") || lower.contains("resume");

    if restart_reviewing {
        write_mission_summary(canvas, 10, "Review", 2, 1, None)?;
        canvas.write(
            14,
            4,
            "Restored reviewing stage for this mission. No auto-advance.",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "Later draft stays ready in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(17, 4, "[esc] Resume conversation", TextStyle::muted())?;
        return lifecycle_passive(
            screen_id,
            fixture_id,
            "stage.reviewing",
            "Review",
            "mission restored at review without auto-advance",
        );
    }
    if restart_awaiting {
        write_mission_summary(canvas, 10, "Ready for completion", 3, 0, None)?;
        canvas.write(
            14,
            4,
            "Restored awaiting human completion. Approval required.",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "Later draft stays ready in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(17, 4, "[esc] Resume conversation", TextStyle::muted())?;
        return lifecycle_passive(
            screen_id,
            fixture_id,
            "stage.awaiting-human",
            "Ready for completion",
            "mission restored at human completion without auto-approval",
        );
    }
    if restart_validating {
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
            "Restored validating stage. No auto-advance past validation.",
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            "Later draft stays ready in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(17, 4, "[esc] Resume conversation", TextStyle::muted())?;
        return lifecycle_passive(
            screen_id,
            fixture_id,
            "stage.validating",
            "Validating",
            "mission restored mid-validation without replaying finished work",
        );
    }
    if restart_blocked {
        write_mission_summary(
            canvas,
            10,
            "Needs attention",
            1,
            2,
            Some(FAILURE_CAUSE),
        )?;
        canvas.write(14, 4, "Needs attention", TextStyle::bright())?;
        canvas.write(15, 4, &format!("  ✗ {TASK_3}"), TextStyle::text())?;
        canvas.write(
            16,
            4,
            "Restored blocked stage with the same cause and completed work.",
            TextStyle::muted(),
        )?;
        canvas.write(
            18,
            4,
            "Later draft stays ready in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(19, 4, "[esc] Resume conversation", TextStyle::muted())?;
        return lifecycle_passive(
            screen_id,
            fixture_id,
            "stage.blocked",
            "Needs attention",
            "mission restored at blocked stage without duplicate repair",
        );
    }
    if restart_running || (resume_running && !paused && !pause_open && !pause_confirm) {
        let card = MissionCard::new(MISSION_GOAL, "Running", 1, 2, None, "View progress");
        write_component(canvas, 10, 4, &card)?;
        canvas.write(
            15,
            4,
            if resume_running && !restart_running {
                "Resumed once from the saved safe boundary. Finished work was not rerun."
            } else {
                "Restored running stage. Mission identity and counts unchanged."
            },
            TextStyle::muted(),
        )?;
        canvas.write(
            16,
            4,
            &format!("Safe boundary · {SAFE_BOUNDARY_ID}"),
            TextStyle::muted(),
        )?;
        canvas.write(
            18,
            4,
            "Later draft stays ready in the originating conversation.",
            TextStyle::muted(),
        )?;
        canvas.write(20, 4, "[i] Intervene", TextStyle::muted())?;
        canvas.write(21, 4, "[esc] Resume conversation", TextStyle::muted())?;
        let pairs = [
            semantic(
                screen_id,
                fixture_id,
                "surface",
                if monitor_uses_action_dock(&fixture_id.to_ascii_lowercase()) {
                    "action-dock"
                } else {
                    "temporary-detail-view"
                },
                reference(10, 4, 28),
                "summary",
                None,
            ),
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
                "progress-stage",
                "stage.running",
                reference(11, 4, 36),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "safe-boundary",
                SAFE_BOUNDARY_ID,
                reference(16, 4, 48),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "invocation",
                INVOCATION_ID,
                reference(15, 4, 56),
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
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "open-intervene",
                reference(20, 4, 18),
                "summary",
                Some(action(
                    "action.mission.open-intervene",
                    "Intervene",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "return-action",
                "monitor-return",
                reference(21, 4, 34),
                "summary",
                Some(action(
                    "action.mission.monitor-return",
                    "Resume conversation",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "restart-restore",
                "mission.restart",
                reference(15, 4, 56),
                "summary",
                Some(action(
                    "action.mission.restart-restore",
                    "Restore mission",
                    true,
                )),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.view-progress",
                screen_id,
                "workflow-monitor",
                "direct",
                "progress disclosure returns to the same compact monitor",
            ),
            transition(
                "action.mission.open-intervene",
                screen_id,
                "workflow-monitor",
                "direct",
                "intervene opens a focused consequence-aware dock",
            ),
            transition(
                "action.mission.monitor-return",
                screen_id,
                "chat-turn",
                "cancel",
                "return restores the originating conversation with preserved draft",
            ),
            transition(
                "action.mission.restart-restore",
                screen_id,
                screen_id,
                "direct",
                "session restart restored running without duplicating mission work or auto-advancing",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    if stopped || stop_confirm {
        write_mission_summary(canvas, 10, "Stopped", 1, 2, None)?;
        canvas.write(14, 4, "Stopped", TextStyle::bright())?;
        canvas.write(
            15,
            4,
            STOPPED_REMAINING,
            TextStyle::text(),
        )?;
        canvas.write(
            16,
            4,
            &format!("Preserved · completed work and evidence · {TASK_1}"),
            TextStyle::muted(),
        )?;
        canvas.write(
            17,
            4,
            "No Resume. Only Replan may create a new proposed continuation.",
            TextStyle::muted(),
        )?;
        write_dock_border(canvas)?;
        dock_line(canvas, DOCK_TOP + 1, "Mission stopped", TextStyle::bright())?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Inspect preserved work or return. Restart never resumes stopped work.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Return to conversation",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [d] Open detail",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "  [r] Replan",
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
                "progress-stage",
                "stage.stopped",
                reference(11, 4, 36),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "stopped-state",
                "mission.stopped",
                reference(15, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "completed-work",
                COMPLETED_TASK_ID,
                reference(16, 4, 52),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "invocation",
                INVOCATION_ID,
                reference(16, 4, 52),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "stopped-return",
                reference(DOCK_TOP + 3, 4, 40),
                "summary",
                Some(action(
                    "action.mission.stopped-return",
                    "Return to conversation",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "open-detail",
                reference(DOCK_TOP + 4, 4, 22),
                "summary",
                Some(action(
                    "action.mission.open-stopped-detail",
                    "Open detail",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "replan",
                reference(DOCK_TOP + 5, 4, 18),
                "summary",
                Some(action("action.mission.replan", "Replan", false)),
            ),
            semantic(
                screen_id,
                fixture_id,
                "restart-restore",
                "mission.restart",
                reference(15, 4, 56),
                "summary",
                Some(action(
                    "action.mission.restart-restore",
                    "Restore mission",
                    true,
                )),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.stopped-return",
                screen_id,
                "chat-turn",
                "direct",
                "returning from stopped restores the originating conversation composer",
            ),
            transition(
                "action.mission.open-stopped-detail",
                screen_id,
                "workflow-monitor",
                "direct",
                "stopped detail stays inspectable without execution",
            ),
            transition(
                "action.mission.replan",
                screen_id,
                "workflow-launch",
                "direct",
                "replan from stopped opens a new proposal and never auto-executes",
            ),
            transition(
                "action.mission.restart-restore",
                screen_id,
                screen_id,
                "direct",
                "session restart restored stopped without auto-resume",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    if stop_open {
        write_mission_summary(canvas, 10, "Running", 1, 2, None)?;
        canvas.write(
            14,
            4,
            "Stopping abandons remaining tasks after the last completed result.",
            TextStyle::muted(),
        )?;
        write_dock_border(canvas)?;
        dock_line(canvas, DOCK_TOP + 1, "Stop this mission?", TextStyle::bright())?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Preserved: completed work, evidence, and conversation. Remaining work is abandoned.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Stop mission",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [esc] Cancel · keep running",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Restart never resumes stopped work automatically.",
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
                "primary-action",
                "stop-mission",
                reference(DOCK_TOP + 3, 4, 28),
                "summary",
                Some(action("action.mission.stop-confirm", "Stop mission", false)),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "cancel-stop",
                reference(DOCK_TOP + 4, 4, 36),
                "summary",
                Some(action("action.mission.stop-cancel", "Cancel", false)),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.stop-confirm",
                screen_id,
                "workflow-monitor",
                "direct",
                "confirming stop produces a persistent inspectable Stopped state",
            ),
            transition(
                "action.mission.stop-cancel",
                screen_id,
                "workflow-monitor",
                "cancel",
                "cancel restores the prior running monitor without abandoning work",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    if pause_open || pause_confirm {
        write_mission_summary(canvas, 10, "Running", 1, 2, None)?;
        canvas.write(
            14,
            4,
            "Current task may finish. New work waits at the next safe boundary.",
            TextStyle::muted(),
        )?;
        canvas.write(
            15,
            4,
            &format!("Safe boundary · {SAFE_BOUNDARY_ID}"),
            TextStyle::muted(),
        )?;
        write_dock_border(canvas)?;
        dock_line(canvas, DOCK_TOP + 1, "Pause this mission?", TextStyle::bright())?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Pause at a safe boundary after the current task finishes. Completed work stays preserved.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Pause mission",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [esc] Cancel · keep running",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Resume later continues once from this boundary.",
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
                "safe-boundary",
                SAFE_BOUNDARY_ID,
                reference(15, 4, 48),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "pause-mission",
                reference(DOCK_TOP + 3, 4, 30),
                "summary",
                Some(action(
                    "action.mission.pause-confirm",
                    "Pause mission",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "cancel-pause",
                reference(DOCK_TOP + 4, 4, 36),
                "summary",
                Some(action("action.mission.pause-cancel", "Cancel", false)),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.pause-confirm",
                screen_id,
                "workflow-monitor",
                "direct",
                "confirming pause persists a safe-boundary paused stage",
            ),
            transition(
                "action.mission.pause-cancel",
                screen_id,
                "workflow-monitor",
                "cancel",
                "cancel restores the running monitor without pausing",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    // Default: intervene chooser or paused shell.
    let stage = if paused || lower.contains("restart-paused") {
        "Paused"
    } else {
        "Running"
    };
    write_mission_summary(canvas, 10, stage, 1, 2, None)?;
    canvas.write(
        14,
        4,
        if stage == "Paused" {
            "Paused at a safe boundary. Resume continues from this point once."
        } else {
            "Choose one intervention. Consequences stay focused and reversible when cancelled."
        },
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        &format!("Safe boundary · {SAFE_BOUNDARY_ID}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        &format!("Completed kept · {TASK_1}"),
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    if stage == "Paused" {
        dock_line(canvas, DOCK_TOP + 1, "Mission paused", TextStyle::bright())?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Resume continues once from the saved safe boundary. Finished work is not rerun.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Resume",
            TextStyle::accent(),
        )?;
        dock_line(canvas, DOCK_TOP + 4, "  [d] Replan", TextStyle::muted())?;
        dock_line(canvas, DOCK_TOP + 5, "  [s] Stop", TextStyle::muted())?;
        let pairs = lifecycle_paused_pairs(screen_id, fixture_id);
        let transitions = vec![
            transition(
                "action.mission.resume",
                screen_id,
                "workflow-monitor",
                "direct",
                "resume transitions paused to running exactly once from the safe boundary",
            ),
            transition(
                "action.mission.replan",
                screen_id,
                "workflow-launch",
                "direct",
                "replan leaves paused monitoring for a revised proposal then preflight",
            ),
            transition(
                "action.mission.stop-open",
                screen_id,
                "workflow-monitor",
                "direct",
                "stop opens a consequence confirmation before abandoning remaining work",
            ),
            transition(
                "action.mission.restart-restore",
                screen_id,
                screen_id,
                "direct",
                "session restart restored paused without duplicating mission work or auto-advancing",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    // Intervene dock: Pause focused by default (one primary).
    dock_line(canvas, DOCK_TOP + 1, "Intervene in this mission", TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Pause at the next safe boundary. Replan and Stop stay secondary.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Pause",
        TextStyle::accent(),
    )?;
    dock_line(canvas, DOCK_TOP + 4, "  [d] Replan", TextStyle::muted())?;
    dock_line(canvas, DOCK_TOP + 5, "  [s] Stop · [esc] Cancel", TextStyle::muted())?;

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
            "safe-boundary",
            SAFE_BOUNDARY_ID,
            reference(15, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "pause-open",
            reference(DOCK_TOP + 3, 4, 20),
            "summary",
            Some(action("action.mission.pause-open", "Pause", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "replan",
            reference(DOCK_TOP + 4, 4, 18),
            "summary",
            Some(action("action.mission.replan", "Replan", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "stop-open",
            reference(DOCK_TOP + 5, 4, 16),
            "summary",
            Some(action("action.mission.stop-open", "Stop", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "intervene-cancel",
            reference(DOCK_TOP + 5, 4, 40),
            "summary",
            Some(action(
                "action.mission.intervene-cancel",
                "Cancel",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.pause-open",
            screen_id,
            "workflow-monitor",
            "direct",
            "pause opens a confirmation dock describing the safe boundary",
        ),
        transition(
            "action.mission.replan",
            screen_id,
            "workflow-launch",
            "direct",
            "replan opens a revised conversational proposal with exact count deltas",
        ),
        transition(
            "action.mission.stop-open",
            screen_id,
            "workflow-monitor",
            "direct",
            "stop opens a confirmation dock naming irreversible remaining-work abandonment",
        ),
        transition(
            "action.mission.intervene-cancel",
            screen_id,
            "workflow-monitor",
            "cancel",
            "cancel restores the healthy monitor with View progress primary",
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

fn lifecycle_paused_pairs(
    screen_id: &str,
    fixture_id: &str,
) -> [(SemanticCandidate, SemanticEvent); 9] {
    [
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
            "progress-stage",
            "stage.paused",
            reference(11, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "safe-boundary",
            SAFE_BOUNDARY_ID,
            reference(15, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "invocation",
            INVOCATION_ID,
            reference(14, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "resume",
            reference(DOCK_TOP + 3, 4, 22),
            "summary",
            Some(action("action.mission.resume", "Resume", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "replan",
            reference(DOCK_TOP + 4, 4, 18),
            "summary",
            Some(action("action.mission.replan", "Replan", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "stop-open",
            reference(DOCK_TOP + 5, 4, 16),
            "summary",
            Some(action("action.mission.stop-open", "Stop", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "restart-restore",
            "mission.restart",
            reference(14, 4, 56),
            "summary",
            Some(action(
                "action.mission.restart-restore",
                "Restore mission",
                true,
            )),
        ),
    ]
}

fn lifecycle_passive(
    screen_id: &str,
    fixture_id: &str,
    stage_id: &str,
    _stage_label: &str,
    path: &str,
) -> Result<ScreenEvidence, String> {
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
            "progress-stage",
            stage_id,
            reference(11, 4, 40),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "invocation",
            INVOCATION_ID,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "completed-work",
            COMPLETED_TASK_ID,
            reference(10, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "return-action",
            "monitor-return",
            reference(17, 4, 34),
            "summary",
            Some(action(
                "action.mission.monitor-return",
                "Resume conversation",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "restart-restore",
            "mission.restart",
            reference(10, 4, 56),
            "summary",
            Some(action(
                "action.mission.restart-restore",
                "Restore mission",
                true,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.restart-restore",
            screen_id,
            screen_id,
            "direct",
            path,
        ),
        transition(
            "action.mission.monitor-return",
            screen_id,
            "chat-turn",
            "cancel",
            "return restores the originating conversation with preserved draft",
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

/// First progress disclosure: human task groups only (VAL-V3-F6-013/014/056).
fn render_progress_disclosure(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    attention: bool,
    bounded: bool,
    expand_working: bool,
) -> Result<ScreenEvidence, String> {
    let completed = 1usize;
    let remaining = if attention { 3usize } else { 2usize };
    let card = MissionCard::new(
        MISSION_GOAL,
        "Running",
        completed,
        remaining,
        None,
        "View progress",
    );
    write_component(canvas, 10, 4, &card)?;

    let mut row: u16 = 15;
    if expand_working {
        // Local one-group expansion only (VAL-V3-F6-056).
        canvas.write(row, 4, "Working · expanded", TextStyle::bright())?;
        row += 1;
        for task in PROGRESS_WORKING_FULL {
            canvas.write(row, 4, &format!("  ● {task}"), TextStyle::text())?;
            row += 1;
        }
        canvas.write(row, 4, "Needs attention", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "  … still bounded below", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "Recently completed", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        row += 2;
        canvas.write(row, 4, "[esc] Back · restore bounded groups", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[w] View workers", TextStyle::muted())?;
    } else if bounded {
        // Bound each group and publish exact omitted counts.
        canvas.write(row, 4, "Needs attention", TextStyle::bright())?;
        row += 1;
        for task in PROGRESS_NEEDS_VISIBLE.iter().take(PROGRESS_BOUND) {
            canvas.write(row, 4, &format!("  ✗ {task}"), TextStyle::text())?;
            row += 1;
        }
        let needs_omitted = PROGRESS_NEEDS_FULL.len().saturating_sub(PROGRESS_BOUND);
        canvas.write(
            row,
            4,
            &format!("  … {needs_omitted} more · [e] Expand Needs attention"),
            TextStyle::muted(),
        )?;
        row += 2;

        canvas.write(row, 4, "Working", TextStyle::bright())?;
        row += 1;
        for task in PROGRESS_WORKING_VISIBLE.iter().take(PROGRESS_BOUND) {
            canvas.write(row, 4, &format!("  ● {task}"), TextStyle::text())?;
            row += 1;
        }
        let working_omitted = PROGRESS_WORKING_FULL.len().saturating_sub(PROGRESS_BOUND);
        canvas.write(
            row,
            4,
            &format!("  … {working_omitted} more · [x] Expand Working"),
            TextStyle::muted(),
        )?;
        row += 2;

        canvas.write(row, 4, "Recently completed", TextStyle::bright())?;
        row += 1;
        for task in PROGRESS_COMPLETED_VISIBLE.iter().take(PROGRESS_BOUND) {
            canvas.write(row, 4, &format!("  ✓ {task}"), TextStyle::muted())?;
            row += 1;
        }
        let completed_omitted = PROGRESS_COMPLETED_FULL
            .len()
            .saturating_sub(PROGRESS_BOUND);
        canvas.write(
            row,
            4,
            &format!("  … {completed_omitted} more · [c] Expand Recently completed"),
            TextStyle::muted(),
        )?;
        row += 2;
        canvas.write(row, 4, "[esc] Collapse progress", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[w] View workers", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[t] Open team board", TextStyle::muted())?;
    } else if attention {
        // Exception-first ordered groups with Needs attention non-empty.
        canvas.write(row, 4, "Needs attention", TextStyle::bright())?;
        row += 1;
        canvas.write(row, 4, &format!("  ✗ {TASK_3}"), TextStyle::text())?;
        row += 1;
        canvas.write(row, 4, &format!("    {FAILURE_CAUSE}"), TextStyle::muted())?;
        row += 2;
        canvas.write(row, 4, "Working", TextStyle::bright())?;
        row += 1;
        canvas.write(row, 4, &format!("  ● {TASK_2}"), TextStyle::text())?;
        row += 2;
        canvas.write(row, 4, "Recently completed", TextStyle::bright())?;
        row += 1;
        canvas.write(row, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        row += 2;
        canvas.write(row, 4, "[esc] Collapse progress", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[w] View workers", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[t] Open team board", TextStyle::muted())?;
    } else {
        // Healthy first disclosure: omit empty Needs attention group (VAL-V3-F6-013).
        canvas.write(row, 4, "Working", TextStyle::bright())?;
        row += 1;
        canvas.write(row, 4, &format!("  ● {TASK_2}"), TextStyle::text())?;
        row += 2;
        canvas.write(row, 4, "Recently completed", TextStyle::bright())?;
        row += 1;
        canvas.write(row, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
        row += 2;
        canvas.write(row, 4, "[esc] Collapse progress", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[w] View workers", TextStyle::muted())?;
        row += 1;
        canvas.write(row, 4, "[t] Open team board", TextStyle::muted())?;
    }

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "surface",
            "temporary-detail-view",
            reference(10, 4, 28),
            "summary",
            None,
        ),
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
            if expand_working {
                "stage.running.progress.expand-working"
            } else if bounded {
                "stage.running.progress.bounded"
            } else if attention {
                "stage.running.progress.attention"
            } else {
                "stage.running.progress"
            },
            reference(11, 4, 36),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "progress-group",
            if attention || bounded {
                "group.needs-attention"
            } else {
                "group.working"
            },
            reference(15, 4, 28),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            if expand_working {
                "collapse-group"
            } else {
                "collapse-progress"
            },
            reference(12, 4, 28),
            "summary",
            Some(action(
                if expand_working {
                    "action.mission.progress-collapse-group"
                } else {
                    "action.mission.collapse-progress"
                },
                if expand_working {
                    "Back"
                } else {
                    "Collapse progress"
                },
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "view-workers",
            reference(21, 4, 24),
            "summary",
            Some(action(
                "action.mission.view-workers",
                "View workers",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "open-team-board",
            reference(22, 4, 28),
            "summary",
            Some(action(
                "action.mission.open-team-board",
                "Open team board",
                false,
            )),
        ),
    ];
    if bounded && !expand_working {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "progress-bound",
            "omitted.working.3",
            reference(20, 4, 40),
            "summary",
            Some(action(
                "action.mission.progress-expand-group",
                "Expand Working",
                false,
            )),
        ));
    }

    let mut transitions = vec![
        transition(
            "action.mission.collapse-progress",
            screen_id,
            "workflow-monitor",
            "cancel",
            "progress disclosure returns to the same compact monitor",
        ),
        transition(
            "action.mission.view-workers",
            screen_id,
            "team-board",
            "direct",
            "workers open only from explicit progress or board disclosure",
        ),
        transition(
            "action.mission.open-team-board",
            screen_id,
            "team-board",
            "direct",
            "team board opens only after explicit worker or task detail",
        ),
    ];
    if bounded && !expand_working {
        transitions.push(transition(
            "action.mission.progress-expand-group",
            screen_id,
            "workflow-monitor",
            "direct",
            "one progress group expands locally without opening workers or advanced evidence",
        ));
    }
    if expand_working {
        // Primary already emits progress-collapse-group; keep only that collapse edge.
        transitions.retain(|t| t.action_id != "action.mission.collapse-progress");
        transitions.push(transition(
            "action.mission.progress-collapse-group",
            screen_id,
            "workflow-monitor",
            "cancel",
            "Back restores the same group, selection, and monitor state",
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

fn render_team_board(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let lower = fixture_id.to_ascii_lowercase();
    let advanced = lower.contains("advanced");
    let workers = lower.contains("workers") || lower.contains("worker");

    // Stable mission origin remains visible in the temporary detail header.
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;

    if advanced {
        canvas.write(7, 4, "Team board · advanced detail", TextStyle::bright())?;
        canvas.write(
            8,
            4,
            "Scheduling and environment detail. Work state is unchanged.",
            TextStyle::muted(),
        )?;
        canvas.write(10, 4, &format!("Task · {TASK_3}"), TextStyle::text())?;
        canvas.write(11, 4, "Wave · release-checks-2", TextStyle::muted())?;
        canvas.write(12, 4, "Branch · release/safe-path", TextStyle::muted())?;
        canvas.write(
            13,
            4,
            "Worktree · ~/project/.worktrees/release-safe",
            TextStyle::muted(),
        )?;
        canvas.write(
            14,
            4,
            "Contract · acceptance-contract.release.safe-path",
            TextStyle::muted(),
        )?;
        canvas.write(16, 4, &format!("Task · {TASK_2}"), TextStyle::text())?;
        canvas.write(17, 4, "Wave · notes-align-1", TextStyle::muted())?;
        canvas.write(18, 4, "Branch · release/next-step", TextStyle::muted())?;
        canvas.write(
            19,
            4,
            "Worktree · ~/project/.worktrees/notes-align",
            TextStyle::muted(),
        )?;
        canvas.write(
            20,
            4,
            "Contract · acceptance-contract.release.next-step",
            TextStyle::muted(),
        )?;
        canvas.write(23, 4, "[esc] Return to team board", TextStyle::accent())?;

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
                "board-advanced",
                "team-board.advanced",
                reference(7, 4, 36),
                "evidence",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "board-task",
                "task.publish-path",
                reference(10, 4, 40),
                "evidence",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "team-board-advanced-return",
                reference(23, 4, 36),
                "evidence",
                Some(action(
                    "action.mission.team-board-advanced-return",
                    "Return to team board",
                    false,
                )),
            ),
        ];
        let transitions = vec![transition(
            "action.mission.team-board-advanced-return",
            screen_id,
            "team-board",
            "cancel",
            "advanced internals collapse back to the human ownership board",
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

    if workers {
        canvas.write(7, 4, "Workers", TextStyle::bright())?;
        canvas.write(
            8,
            4,
            "Explicit ownership summary. Mission state is unchanged.",
            TextStyle::muted(),
        )?;
        canvas.write(10, 4, "● Notes specialist · working", TextStyle::text())?;
        canvas.write(11, 4, &format!("  Task · {TASK_2}"), TextStyle::muted())?;
        canvas.write(13, 4, "✗ Release reviewer · needs attention", TextStyle::text())?;
        canvas.write(14, 4, &format!("  Task · {TASK_3}"), TextStyle::muted())?;
        canvas.write(16, 4, "✓ Docs lead · recently completed", TextStyle::muted())?;
        canvas.write(17, 4, &format!("  Task · {TASK_1}"), TextStyle::muted())?;
        canvas.write(20, 4, "[esc] Return to mission monitor", TextStyle::accent())?;
        canvas.write(21, 4, "[t] Open team board", TextStyle::muted())?;

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
                "worker-detail",
                "workers.summary",
                reference(7, 4, 20),
                "focused",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "board-task",
                "task.next-step",
                reference(10, 4, 36),
                "focused",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "workers-return",
                reference(20, 4, 36),
                "focused",
                Some(action(
                    "action.mission.workers-return",
                    "Return to mission monitor",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "open-team-board",
                reference(21, 4, 28),
                "focused",
                Some(action(
                    "action.mission.open-team-board",
                    "Open team board",
                    false,
                )),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "team-board-return",
                reference(20, 4, 36),
                "focused",
                Some(action(
                    "action.mission.team-board-return",
                    "Return to mission monitor",
                    false,
                )),
            ),
        ];
        let transitions = vec![
            transition(
                "action.mission.workers-return",
                screen_id,
                "workflow-monitor",
                "cancel",
                "worker detail returns to the same monitor without mission changes",
            ),
            transition(
                "action.mission.open-team-board",
                screen_id,
                "team-board",
                "direct",
                "workers can open the general ownership board",
            ),
            transition(
                "action.mission.team-board-return",
                screen_id,
                "workflow-monitor",
                "cancel",
                "leaving the board restores the same monitor state without changing work",
            ),
        ];
        return Ok((
            pairs
                .iter()
                .map(|(candidate, _)| candidate.clone())
                .collect(),
            pairs.iter().map(|(_, event)| event.clone()).collect(),
            transitions,
        ));
    }

    // First board level: human task, dependency, owner, readable state (VAL-V3-F6-021).
    canvas.write(7, 4, "Team board", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Shows ownership and assignments for the current mission.",
        TextStyle::muted(),
    )?;
    canvas.write(10, 4, "Needs attention", TextStyle::bright())?;
    canvas.write(11, 4, &format!("  ✗ {TASK_3}"), TextStyle::text())?;
    canvas.write(12, 4, "    Depends on · next-step note and review", TextStyle::muted())?;
    canvas.write(13, 4, "    Owner · Release reviewer · blocked", TextStyle::text())?;
    canvas.write(15, 4, "Working", TextStyle::bright())?;
    canvas.write(16, 4, &format!("  ● {TASK_2}"), TextStyle::text())?;
    canvas.write(17, 4, "    Depends on · release notes review", TextStyle::muted())?;
    canvas.write(18, 4, "    Owner · Notes specialist · working", TextStyle::text())?;
    canvas.write(20, 4, "Recently completed", TextStyle::bright())?;
    canvas.write(21, 4, &format!("  ✓ {TASK_1}"), TextStyle::muted())?;
    canvas.write(22, 4, "    Depends on · none", TextStyle::muted())?;
    canvas.write(23, 4, "    Owner · Docs lead · complete", TextStyle::muted())?;
    canvas.write(25, 4, "[d] View advanced detail", TextStyle::muted())?;
    canvas.write(26, 4, "[w] View workers", TextStyle::muted())?;
    canvas.write(28, 4, "[esc] Return to mission monitor", TextStyle::accent())?;

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
            "team-board",
            "team-board.ownership",
            reference(7, 4, 24),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "board-task",
            "task.publish-path",
            reference(11, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "board-task",
            "task.next-step",
            reference(16, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "board-task",
            "task.release-notes",
            reference(21, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "team-board-return",
            reference(28, 4, 40),
            "focused",
            Some(action(
                "action.mission.team-board-return",
                "Return to mission monitor",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "team-board-advanced",
            reference(25, 4, 32),
            "focused",
            Some(action(
                "action.mission.team-board-advanced",
                "View advanced detail",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "view-workers",
            reference(26, 4, 24),
            "focused",
            Some(action(
                "action.mission.view-workers",
                "View workers",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "open-team-board",
            reference(7, 4, 20),
            "focused",
            Some(action(
                "action.mission.open-team-board",
                "Open team board",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.team-board-return",
            screen_id,
            "workflow-monitor",
            "cancel",
            "leaving the board restores the same monitor state without changing work",
        ),
        transition(
            "action.mission.team-board-advanced",
            screen_id,
            "team-board",
            "direct",
            "waves, branches, worktrees, and contracts open only as a second detail level",
        ),
        transition(
            "action.mission.view-workers",
            screen_id,
            "team-board",
            "direct",
            "worker identity remains a focused detail under the board",
        ),
        transition(
            "action.mission.open-team-board",
            screen_id,
            screen_id,
            "direct",
            "team board remains reachable as an explicit ownership detail route",
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

fn render_workflow_monitor_failure(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    write_conversation_header(canvas)?;
    let lower = fixture_id.to_ascii_lowercase();
    let missing_input = lower.contains("missing-input") || lower.contains("missing_input");
    let retry_eligible = lower.contains("retry-eligible") || lower.contains("retry_eligible");

    let cause = if missing_input {
        MISSING_INPUT
    } else {
        FAILURE_CAUSE
    };

    // Compact card fields only; primary action lives in the dock.
    write_mission_summary(
        canvas,
        10,
        "Needs attention",
        1,
        2,
        Some(cause),
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
        &format!("    {cause}"),
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
        if retry_eligible {
            "Retry is eligible for this task. Validation will rerun for the affected scope."
        } else if missing_input {
            "Missing input blocks retry. Provide the note before repair can continue."
        } else {
            "Completed work stays preserved. Create a fix task to repair."
        },
        TextStyle::muted(),
    )?;

    write_dock_border(canvas)?;
    if retry_eligible {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Retry eligible failed work",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            &format!("Retry · {TASK_3}. Fresh validation will rerun for this scope."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Retry task",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "  [d] Create a fix task",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "  [esc] Resume conversation",
            TextStyle::muted(),
        )?;
    } else if missing_input {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Missing input blocks repair",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Add the missing checklist note. Retry is not offered until input arrives.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› [enter] Add missing input",
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
    } else {
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
    }

    let pairs = if retry_eligible {
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
                "retry-scope",
                "task.check-publish",
                reference(DOCK_TOP + 2, 4, 56),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "retry-task",
                reference(DOCK_TOP + 3, 4, 28),
                "summary",
                Some(action("action.mission.retry-task", "Retry task", false)),
            ),
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "create-fix-task",
                reference(DOCK_TOP + 4, 4, 30),
                "summary",
                Some(action(
                    "action.mission.create-fix-task",
                    "Create a fix task",
                    false,
                )),
            ),
        ]
    } else if missing_input {
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
                "blocker",
                "blocker.missing-input",
                reference(12, 4, 64),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "failure",
                "failure.missing-input",
                reference(16, 4, 64),
                "summary",
                None,
            ),
            semantic(
                screen_id,
                fixture_id,
                "primary-action",
                "add-missing-input",
                reference(DOCK_TOP + 3, 4, 34),
                "summary",
                Some(action(
                    "action.mission.add-missing-input",
                    "Add missing input",
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
                "view-validation-stages",
                reference(DOCK_TOP + 4, 4, 24),
                "summary",
                Some(action(
                    "action.mission.view-validation-stages",
                    "Open evidence",
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
        ]
    };
    let mut pairs = pairs;
    let mut transitions = if retry_eligible {
        vec![
            transition(
                "action.mission.retry-task",
                screen_id,
                "workflow-monitor",
                "direct",
                "retry triggers fresh scoped execution and validation for the eligible task only",
            ),
            transition(
                "action.mission.create-fix-task",
                screen_id,
                "workflow-monitor",
                "direct",
                "creating a fix task records the failure as planned repair work",
            ),
        ]
    } else if missing_input {
        vec![
            transition(
                "action.mission.add-missing-input",
                screen_id,
                "workflow-launch-blocked",
                "direct",
                "missing input resolution stays blocked until the checklist note is provided",
            ),
            transition(
                "action.mission.open-evidence",
                screen_id,
                "verification-ladder",
                "direct",
                "evidence disclosure opens the validation stages for the same failure",
            ),
        ]
    } else {
        vec![
            transition(
                "action.mission.create-fix-task",
                screen_id,
                "workflow-monitor",
                "direct",
                "creating a fix task records the failure as planned repair work",
            ),
            transition(
                "action.mission.view-validation-stages",
                screen_id,
                "verification-ladder",
                "direct",
                "validation stages open only from an explicit evidence action",
            ),
            transition(
                "action.mission.open-evidence",
                screen_id,
                "verification-ladder",
                "direct",
                "evidence disclosure opens the validation stages for the same failure",
            ),
        ]
    };
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
    // Keep the stable mission task band and You-line continuous with monitor and
    // approved frames. "Mission review" is a labeled surface section under that
    // identity, never an unlabeled second journey title that drops task/You.
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Mission review", TextStyle::muted())?;
    canvas.write(
        8,
        4,
        "Review outcome ready. Mission card stays compact.",
        TextStyle::muted(),
    )?;

    write_mission_summary(canvas, 10, "Review", 2, 1, None)?;

    let review = ReviewBlock::new(
        "Changes requested",
        REVIEW_REASON_CHANGES,
        "Create a fix task",
    )
    .with_affected_result(AFFECTED_RESULT);
    write_component(canvas, 14, 4, &review)?;
    canvas.write(
        19,
        4,
        "Checklist and deeper evidence stay collapsed until you open them.",
        TextStyle::muted(),
    )?;
    // Validation stages and score detail are separate open affordances (1:1).
    // Gate frames keep the single summary control + stages only. Non-gate and
    // score-open fixtures paint a distinct quality-report open for failing scores.
    let score_open = wants_score_open(fixture_id);
    canvas.write(
        21,
        4,
        "[d] View validation summary",
        TextStyle::muted(),
    )?;
    if score_open {
        canvas.write(22, 4, "[q] View quality report", TextStyle::muted())?;
        canvas.write(
            24,
            4,
            "[esc] Return to mission monitor",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            23,
            4,
            "[esc] Return to mission monitor",
            TextStyle::muted(),
        )?;
    }
    let return_row = if score_open { 24 } else { 23 };

    let mut pairs = vec![
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
            "mission-task",
            "mission-conversation",
            reference(3, 4, 56),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "user-turn",
            "mission-conversation",
            reference(5, 4, 72),
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
            "review.changes-requested",
            reference(14, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "create-fix-from-review",
            reference(16, 4, 32),
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
            "view-validation-stages",
            reference(21, 4, 36),
            "summary",
            Some(action(
                "action.mission.view-validation-stages",
                "View validation summary",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "return-monitor",
            reference(return_row, 4, 36),
            "summary",
            Some(action(
                "action.mission.review-return",
                "Return to mission monitor",
                false,
            )),
        ),
    ];
    let mut transitions = vec![
        transition(
            "action.mission.review-create-fix",
            screen_id,
            "workflow-monitor",
            "direct",
            "requested changes become an explicit fix task followed by fresh validation",
        ),
        transition(
            "action.mission.view-validation-stages",
            screen_id,
            "verification-ladder",
            "direct",
            "validation stages open only from an explicit evidence action",
        ),
        transition(
            "action.mission.review-return",
            screen_id,
            "workflow-monitor",
            "cancel",
            "leaving review returns to the originating monitor without approving",
        ),
    ];
    if score_open {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "view-score-detail",
            reference(22, 4, 36),
            "summary",
            Some(action(
                "action.mission.view-score-detail",
                "View quality report",
                false,
            )),
        ));
        transitions.push(transition(
            "action.mission.view-score-detail",
            screen_id,
            "intent-scorer-fail",
            "direct",
            "failing score detail opens only from an explicit evidence action",
        ));
    }
    if fixture_id.contains("gate.mission.reviewer-output") {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "bridge-note",
            "action.storyboard.summarize-review-to-approval",
            reference(21, 4, 48),
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
    // Passing score detail needs its own painted open, never dual-bound onto Stay.
    // Gate frames keep the three dock actions only; non-gate/score-open paint quality.
    let score_open = wants_score_open(fixture_id);

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
    if score_open && !request_changes_dock {
        // Disclosure-safe, non-dock secondary so dock geometry stay-lines remain fixed.
        canvas.write(20, 4, "[q] View quality report", TextStyle::muted())?;
    }

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
        let mut pairs = vec![
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
            semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "stay-on-review",
                reference(DOCK_TOP + 5, 4, 36),
                "summary",
                Some(action(
                    "action.mission.stay-on-review",
                    "Stay on this review",
                    false,
                )),
            ),
        ];
        if score_open {
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "secondary-action",
                "view-score-detail",
                reference(20, 4, 36),
                "summary",
                Some(action(
                    "action.mission.view-score-detail",
                    "View quality report",
                    false,
                )),
            ));
        }
        pairs
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
        let mut transitions = vec![
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
        ];
        if score_open {
            transitions.push(transition(
                "action.mission.view-score-detail",
                screen_id,
                "intent-scorer-pass",
                "direct",
                "passing score detail opens only from an explicit evidence action",
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

/// Explicit validation stages (VAL-V3-F6-027 / 028 / 036 / 049).
/// Compact failure opens this focused decision view; stage payloads and tool
/// envelopes remain one level deeper and unwind one level at a time.
fn render_verification_ladder(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let lower = fixture_id.to_ascii_lowercase();
    let stage_evidence = lower.contains("stage-evidence")
        || lower.contains("stage_evidence")
        || lower.contains("detailed-evidence")
        || lower.contains("detailed_evidence");

    if stage_evidence {
        return render_verification_stage_evidence(canvas, screen_id, fixture_id);
    }

    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Validation stages", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Decision view for the current validation run. Mission counts stay unchanged.",
        TextStyle::muted(),
    )?;
    canvas.write(10, 4, "Origin · failure on the safe publish path", TextStyle::muted())?;

    // Readable stage purpose + outcome. Failing stage is distinct.
    canvas.write(12, 4, "✓ Deterministic checks · pass", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "  Format, lint, type check, and build are clean.",
        TextStyle::muted(),
    )?;
    canvas.write(15, 4, "✓ Agent-visible tests · pass", TextStyle::muted())?;
    canvas.write(
        16,
        4,
        "  The automated cases that stay on this board still pass.",
        TextStyle::muted(),
    )?;
    canvas.write(18, 4, "✗ Held-out checks · needs repair", TextStyle::text())?;
    canvas.write(
        19,
        4,
        "  One hidden assertion failed on the safe publish path.",
        TextStyle::text(),
    )?;
    canvas.write(
        20,
        4,
        "  Current stage · repair this before review can continue.",
        TextStyle::bright(),
    )?;
    canvas.write(22, 4, "○ Intent audit · pending", TextStyle::muted())?;
    canvas.write(
        23,
        4,
        "  Opens after the held-out failure is repaired.",
        TextStyle::muted(),
    )?;
    canvas.write(25, 4, "○ Process check · pending", TextStyle::muted())?;
    canvas.write(
        26,
        4,
        "  Final process evidence waits on a clean held-out pass.",
        TextStyle::muted(),
    )?;

    canvas.write(
        28,
        4,
        "[e] Open detailed evidence for the held-out failure",
        TextStyle::muted(),
    )?;
    canvas.write(
        29,
        4,
        "[t] View tool evidence",
        TextStyle::muted(),
    )?;
    canvas.write(
        31,
        4,
        "[esc] Return to mission failure",
        TextStyle::accent(),
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
            "validation-stage",
            "stage.deterministic",
            reference(12, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "validation-stage",
            "stage.held-out",
            reference(18, 4, 40),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "validation-stage",
            "stage.intent-audit",
            reference(22, 4, 36),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "open-stage-evidence",
            reference(28, 4, 52),
            "focused",
            Some(action(
                "action.mission.view-stage-evidence",
                "Open detailed evidence",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "view-tool-evidence",
            reference(29, 4, 32),
            "focused",
            Some(action(
                "action.mission.view-tool-evidence",
                "View tool evidence",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "verification-ladder-return",
            reference(31, 4, 40),
            "focused",
            Some(action(
                "action.mission.verification-ladder-return",
                "Return to mission failure",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.verification-ladder-return",
            screen_id,
            "workflow-monitor-failure",
            "cancel",
            "validation stages return to the failure that opened them",
        ),
        transition(
            "action.mission.view-tool-evidence",
            screen_id,
            "tool-envelope",
            "direct",
            "tool payloads stay one disclosure deeper under validation stages",
        ),
        transition(
            "action.mission.view-stage-evidence",
            screen_id,
            "verification-ladder",
            "direct",
            "nested stage evidence remains inside the validation detail",
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

/// One-level-deeper held-out stage payload under the validation decisions view.
/// Unwinds only to Validation stages (never skips to the failure monitor).
fn render_verification_stage_evidence(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Stage evidence · Held-out checks", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Focused detail for one failing stage. Mission counts stay unchanged.",
        TextStyle::muted(),
    )?;
    canvas.write(
        10,
        4,
        "Mission · Needs attention · 1 complete · 2 remaining",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        &format!("Blocker · {FAILURE_CAUSE}"),
        TextStyle::text(),
    )?;
    canvas.write(13, 4, "Assertion detail · publish path marker", TextStyle::text())?;
    canvas.write(
        14,
        4,
        "  Expected · safe release marker present on the publish path.",
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        "  Observed · marker missing after the last path rewrite.",
        TextStyle::text(),
    )?;
    canvas.write(
        16,
        4,
        "  Hidden assertion body · truncated after 240 bytes …",
        TextStyle::muted(),
    )?;
    canvas.write(
        18,
        4,
        "evidence body · path rewrite left the safe-path guard unset",
        TextStyle::muted(),
    )?;
    canvas.write(
        19,
        4,
        "Source · held-out validation run for the current repair cycle",
        TextStyle::muted(),
    )?;
    canvas.write(
        21,
        4,
        "This nested view does not open waves, workers, scores, or tool payloads.",
        TextStyle::muted(),
    )?;
    canvas.write(
        23,
        4,
        "Tool payloads stay a sibling advanced route from Validation stages.",
        TextStyle::muted(),
    )?;
    canvas.write(
        25,
        4,
        "[esc] Return to validation stages",
        TextStyle::accent(),
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
            "validation-stage",
            "stage.held-out.evidence",
            reference(7, 4, 44),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "stage-evidence",
            "held-out.assertion.publish-path",
            reference(13, 4, 48),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "stage-evidence-return",
            reference(25, 4, 44),
            "evidence",
            Some(action(
                "action.mission.stage-evidence-return",
                "Return to validation stages",
                false,
            )),
        ),
    ];
    let transitions = vec![transition(
        "action.mission.stage-evidence-return",
        screen_id,
        "verification-ladder",
        "cancel",
        "stage evidence unwinds one level to the validation decision view",
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

/// Advisory pass score detail (VAL-V3-F6-029 / 031). Never controls lifecycle.
fn render_intent_scorer_pass(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Quality report · pass", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Advisory dimensions only. This report never completes the mission.",
        TextStyle::muted(),
    )?;
    canvas.write(
        10,
        4,
        "Result · Pass · supporting dimensions are healthy",
        TextStyle::text(),
    )?;
    canvas.write(12, 4, "✓ Intent fidelity · full", TextStyle::text())?;
    canvas.write(
        13,
        4,
        "  The requested release outcome is present in the result.",
        TextStyle::muted(),
    )?;
    canvas.write(15, 4, "✓ Execution evidence · full", TextStyle::text())?;
    canvas.write(
        16,
        4,
        "  Checks and surface evidence were captured for review.",
        TextStyle::muted(),
    )?;
    canvas.write(18, 4, "● Scope discipline · partial", TextStyle::text())?;
    canvas.write(
        19,
        4,
        "  One cleanup note remains, still within the accepted plan.",
        TextStyle::muted(),
    )?;
    canvas.write(21, 4, "✓ Report honesty · full", TextStyle::text())?;
    canvas.write(
        22,
        4,
        "  Summary matches the submitted change set.",
        TextStyle::muted(),
    )?;
    canvas.write(
        24,
        4,
        "A passing report still needs validation stages and human completion.",
        TextStyle::muted(),
    )?;
    canvas.write(
        26,
        4,
        "This detail is advisory and cannot mark the mission complete.",
        TextStyle::muted(),
    )?;
    canvas.write(
        28,
        4,
        "[esc] Return to human completion",
        TextStyle::accent(),
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
            "score-result",
            "score.pass",
            reference(10, 4, 48),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "score-dimension",
            "dimension.intent-fidelity",
            reference(12, 4, 36),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "score-dimension",
            "dimension.scope-discipline",
            reference(18, 4, 40),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "intent-scorer-return",
            reference(28, 4, 40),
            "evidence",
            Some(action(
                "action.mission.intent-scorer-return",
                "Return to human completion",
                false,
            )),
        ),
    ];
    let transitions = vec![transition(
        "action.mission.intent-scorer-return",
        screen_id,
        "reviewer-approved",
        "cancel",
        "closing score detail restores the human completion gate unchanged",
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

/// Failing score detail with repair route (VAL-V3-F6-030 / 031).
fn render_intent_scorer_fail(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Quality report · needs repair", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Advisory only. A failing report routes to repair, not completion.",
        TextStyle::muted(),
    )?;
    canvas.write(
        10,
        4,
        "Result · Fail · one dimension blocks a safe completion claim",
        TextStyle::text(),
    )?;
    canvas.write(12, 4, "✓ Intent fidelity · full", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "  Requested outcome is still present.",
        TextStyle::muted(),
    )?;
    canvas.write(15, 4, "✗ Scope discipline · failed", TextStyle::text())?;
    canvas.write(
        16,
        4,
        "  Publish-path scope still omits the open risk and rollback order.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "  Failed dimension · Scope discipline",
        TextStyle::bright(),
    )?;
    canvas.write(19, 4, "● Report honesty · partial", TextStyle::muted())?;
    canvas.write(
        20,
        4,
        "  Summary notes the issue but does not resolve it.",
        TextStyle::muted(),
    )?;
    canvas.write(
        22,
        4,
        &format!("Suggested fix task · {FIX_TASK}"),
        TextStyle::text(),
    )?;
    canvas.write(
        24,
        4,
        "This report never completes the mission. Create a fix task to continue.",
        TextStyle::muted(),
    )?;
    canvas.write(
        26,
        4,
        "› [enter] Create a fix task",
        TextStyle::accent(),
    )?;
    canvas.write(
        28,
        4,
        "[esc] Return to review outcome",
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
            "score-result",
            "score.fail",
            reference(10, 4, 56),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "score-dimension",
            "dimension.scope-discipline",
            reference(15, 4, 40),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "scorer-create-fix",
            reference(26, 4, 34),
            "evidence",
            Some(action(
                "action.mission.scorer-create-fix",
                "Create a fix task",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "intent-scorer-return",
            reference(28, 4, 40),
            "evidence",
            Some(action(
                "action.mission.intent-scorer-return",
                "Return to review outcome",
                false,
            )),
        ),
    ];
    let transitions = vec![
        transition(
            "action.mission.scorer-create-fix",
            screen_id,
            "workflow-monitor",
            "direct",
            "failing score opens planned fix work instead of completing the mission",
        ),
        transition(
            "action.mission.intent-scorer-return",
            screen_id,
            "reviewer-output",
            "cancel",
            "closing the failing score restores the same review outcome",
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

/// Exception-first runtime detail (VAL-V3-F6-032 / 033).
fn render_concurrency_metrics(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Runtime detail", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Advanced concurrency view. Opening this does not change scheduling.",
        TextStyle::muted(),
    )?;

    // Exceptions first: conflicts, held work, quarantine.
    canvas.write(10, 4, "Needs attention", TextStyle::bright())?;
    canvas.write(11, 4, "✗ Conflict on the safe publish path", TextStyle::text())?;
    canvas.write(
        12,
        4,
        "  Consequence · parallel edits wait until you pause or replan.",
        TextStyle::text(),
    )?;
    canvas.write(14, 4, "● Held work · publish checklist handoff", TextStyle::text())?;
    canvas.write(
        15,
        4,
        "  Consequence · one worker is blocked until the hold is cleared.",
        TextStyle::text(),
    )?;
    canvas.write(17, 4, "○ Quarantine · risky draft branch kept aside", TextStyle::text())?;
    canvas.write(
        18,
        4,
        "  Consequence · that branch will not merge until review accepts it.",
        TextStyle::text(),
    )?;

    // Healthy throughput after exceptions.
    canvas.write(20, 4, "Running cleanly", TextStyle::muted())?;
    canvas.write(
        21,
        4,
        "  Throughput · notes work and completed checks stay healthy.",
        TextStyle::muted(),
    )?;
    canvas.write(
        22,
        4,
        "  Workers ready · two quiet capacity lines collapsed.",
        TextStyle::muted(),
    )?;
    canvas.write(
        24,
        4,
        "Use pause, replan, or retry from the monitor if the hold is blocking progress.",
        TextStyle::muted(),
    )?;
    canvas.write(
        26,
        4,
        "[esc] Return to mission monitor",
        TextStyle::accent(),
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
            "runtime-exception",
            "conflict.publish-path",
            reference(11, 4, 42),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "runtime-exception",
            "held-work.checklist",
            reference(14, 4, 48),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "runtime-exception",
            "quarantine.draft",
            reference(17, 4, 48),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "runtime-healthy",
            "throughput.notes",
            reference(21, 4, 56),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "concurrency-return",
            reference(26, 4, 40),
            "evidence",
            Some(action(
                "action.mission.concurrency-return",
                "Return to mission monitor",
                false,
            )),
        ),
    ];
    let transitions = vec![transition(
        "action.mission.concurrency-return",
        screen_id,
        "workflow-monitor",
        "cancel",
        "closing runtime detail restores the same monitor without scheduling changes",
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

/// Tool evidence with success / error / refusal (VAL-V3-F6-034 / 035).
fn render_tool_envelope(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, MISSION_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, MISSION_TASK, TextStyle::bright())?;
    canvas.write(5, 4, USER_PROMPT, TextStyle::bright())?;
    canvas.write(7, 4, "Tool evidence", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Focused diagnostics for the selected tool call. Secrets stay redacted.",
        TextStyle::muted(),
    )?;
    canvas.write(
        10,
        4,
        "Origin · validation on the safe publish path",
        TextStyle::muted(),
    )?;

    canvas.write(12, 4, "✓ Success · list publish checklist", TextStyle::text())?;
    canvas.write(
        13,
        4,
        "  Payload · 12 checklist rows · truncated after 400 bytes …",
        TextStyle::muted(),
    )?;
    canvas.write(
        14,
        4,
        "  Credential fields · [redacted]",
        TextStyle::muted(),
    )?;

    canvas.write(
        16,
        4,
        "✗ Execution error · verify safe publish path",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "  Outcome · command failed with a missing path marker.",
        TextStyle::text(),
    )?;
    canvas.write(
        18,
        4,
        "  Payload · error body truncated · secrets redacted.",
        TextStyle::muted(),
    )?;

    canvas.write(
        20,
        4,
        "○ Refusal · ship without human approval",
        TextStyle::text(),
    )?;
    canvas.write(
        21,
        4,
        "  Outcome · refused · the tool did not run.",
        TextStyle::text(),
    )?;
    canvas.write(
        22,
        4,
        "  Reason · risky action needs explicit permission first.",
        TextStyle::muted(),
    )?;
    canvas.write(
        23,
        4,
        "  Payload · none · refusal carries no success body.",
        TextStyle::muted(),
    )?;

    canvas.write(
        25,
        4,
        "Raw JSON, transport fields, and secret values stay out of this view.",
        TextStyle::muted(),
    )?;
    canvas.write(
        27,
        4,
        "[esc] Return to validation stages",
        TextStyle::accent(),
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
            "tool-outcome",
            "tool.success.checklist",
            reference(12, 4, 44),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "tool-outcome",
            "tool.error.publish-path",
            reference(16, 4, 48),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "tool-outcome",
            "tool.refusal.ship",
            reference(20, 4, 44),
            "evidence",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "tool-envelope-return",
            reference(27, 4, 40),
            "evidence",
            Some(action(
                "action.mission.tool-envelope-return",
                "Return to validation stages",
                false,
            )),
        ),
    ];
    let transitions = vec![transition(
        "action.mission.tool-envelope-return",
        screen_id,
        "verification-ladder",
        "cancel",
        "closing tool evidence restores the validation stages that opened it",
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
    if lower.contains("proposal") || lower.contains("replan") {
        // Replan lands on a conversational proposal before preflight.
        if lower.contains("preflight-running")
            || lower.contains("preflight_running")
            || lower.contains("preflight-run")
        {
            LaunchVariant::PreflightRunning
        } else {
            LaunchVariant::Proposal
        }
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

/// Gate frames preserve approved digests without score/runtime open paint.
/// Non-gate origin fixtures (`score-open` / `runtime-open` tokens, plus the
/// stable orchestration origin walls used by evidence tests) paint distinct
/// 1:1 open labels so open paths never dual-bind Stay or validation summary.
fn wants_score_open(fixture_id: &str) -> bool {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("gate.mission.") {
        return false;
    }
    lower.contains("score-open")
        || lower.contains("score_open")
        || lower == "canonical.reviewer-output"
        || lower == "canonical.reviewer-approved"
        || lower == "orchestration.reviewer-output"
        || lower == "orchestration.reviewer-approved"
        || lower == "orchestration.reviewer-output.score-open"
        || lower == "orchestration.reviewer-approved.score-open"
}

fn wants_runtime_open(fixture_id: &str) -> bool {
    let lower = fixture_id.to_ascii_lowercase();
    if lower.contains("gate.mission.") {
        return false;
    }
    lower.contains("runtime-open")
        || lower.contains("runtime_open")
        || lower == "canonical.workflow-monitor"
        || lower == "orchestration.workflow-monitor"
        || lower == "orchestration.workflow-monitor.runtime-open"
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
