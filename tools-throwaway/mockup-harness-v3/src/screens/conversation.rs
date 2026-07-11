//! Daily conversation pilot fixtures.
//!
//! The first three canonical states deliberately share the same resumed
//! transcript. Home emphasizes entry, the completed turn emphasizes reading,
//! and streaming adds exactly one compact live owner without replacing either
//! the transcript or the bottom composer.

use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{ActivityRow, ActivityState, Component},
        layout::{StatusLine, CANVAS_ROWS},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "home" => render_home(&mut canvas, screen_id, fixture_id)?,
        "chat-turn" if fixture_id.starts_with("case.streaming-interrupt") => {
            render_interrupted_chat_turn(&mut canvas, screen_id, fixture_id)?
        }
        "chat-turn" => render_chat_turn(&mut canvas, screen_id, fixture_id)?,
        "streaming" => render_streaming(&mut canvas, screen_id, fixture_id)?,
        _ => return Err(format!("unknown conversation pilot screen: {screen_id}")),
    };
    canvas.write(
        CANVAS_ROWS - 1,
        2,
        statusline.text(),
        statusline.style(),
    )?;

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
    })
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
    composer(
        canvas,
        "Ask Code to continue",
        None,
    )?;

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
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
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

fn render_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Why are stale sessions rejected after a password change?",
        TextStyle::bright(),
    )?;
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

    let request = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "user-request",
        reference(4, 4, 69),
        "summary",
        None,
    );
    let resume = semantic(
        screen_id,
        fixture_id,
        "resume",
        "current-conversation",
        reference(4, 4, 69),
        "summary",
        None,
    );
    let answer = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "completed-answer",
        reference(9, 4, 67),
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
        Some(action(
            "action.chat-turn.send-reply",
            "Send reply",
            false,
        )),
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
    let pairs = [request, resume, answer, composer, focus, status];
    Ok((
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
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
    canvas.write(
        4,
        4,
        "You · Summarize the auth refactor and why it is safer.",
        TextStyle::bright(),
    )?;
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
    canvas.write(
        13,
        4,
        "The answer is still being written",
        TextStyle::muted(),
    )?;
    let live = ActivityRow::new(
        "Writing reply",
        ActivityState::Running,
        "24s elapsed · partial answer stays above",
    )
    .with_interrupt("Stop reply");
    write_component(canvas, 16, 4, &live)?;
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
        reference(16, 4, 41),
        "summary",
        Some(action(
            "action.streaming.interrupt",
            "Stop reply",
            false,
        )),
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
    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    let pairs = [partial_answer, live_state, queued_composer, status];
    Ok((
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                "action.streaming.interrupt",
                "streaming",
                "chat-turn",
                "direct",
                "stopping locally preserves the partial answer and queued follow-up",
            ),
        ],
    ))
}

fn render_interrupted_chat_turn(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Summarize the auth refactor and why it is safer.",
        TextStyle::bright(),
    )?;
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
        "● Reply stopped safely. Partial answer and draft are saved.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "Queued follow-up: Also list the tests that should cover the change.",
        TextStyle::text(),
    )?;
    composer(canvas, "Reply to Code", Some("Send reply"))?;

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
        reference(14, 4, 73),
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
        Some(action(
            "action.chat-turn.send-reply",
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
    let pairs = [partial_answer, queued_follow_up, composer, status];
    Ok((
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
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
