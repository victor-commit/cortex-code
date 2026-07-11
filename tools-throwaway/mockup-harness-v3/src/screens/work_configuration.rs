//! Flow 3 pilot states for the first configuration decision and model choice.
//!
//! Both states stay in the shared bottom action dock. The first-run screen
//! explains one decision and offers a safe default. The model picker compares
//! human-readable choices, then adds detail only for the focused pending row.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace,
    },
    chrome::{
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const DOCK_TOP: u16 = 31;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ActionDock, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "onboarding" => render_onboarding(&mut canvas, screen_id, fixture_id)?,
        "model-picker" => render_model_picker(&mut canvas, screen_id, fixture_id)?,
        _ => return Err(format!("unknown Flow 3 onboarding screen: {screen_id}")),
    };
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;
    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
    })
}

fn render_onboarding(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(4, 4, "Code", TextStyle::bright())?;
    canvas.write(7, 4, "Let's set up one thing", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Choose the model Code should use for your next conversation.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "You can change this later. No project or account setup is needed to start.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Recommended · Claude Sonnet 4.5",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Balanced speed and depth for everyday coding conversations.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Use Claude Sonnet 4.5",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [s] Skip with this safe default",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "[m] See other models · [esc] Back · first setup returns here",
        TextStyle::muted(),
    )?;

    let pairs = [
        semantic(
            screen_id,
            fixture_id,
            "surface",
            "onboarding-action-dock",
            reference(7, 4, 26),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "decision",
            "first-model-choice",
            reference(9, 4, 67),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "default",
            "recommended-claude-sonnet-4-5",
            reference(DOCK_TOP + 1, 4, 41),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "balanced-speed-and-depth",
            reference(DOCK_TOP + 2, 4, 69),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.onboarding.apply-default",
            reference(DOCK_TOP + 3, 4, 36),
            "focused",
            Some(action(
                "action.onboarding.apply-default",
                "Use Claude Sonnet 4.5",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.onboarding.skip",
            reference(DOCK_TOP + 4, 4, 37),
            "summary",
            Some(action(
                "action.onboarding.skip",
                "Skip with this safe default",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.onboarding.other-models",
            reference(DOCK_TOP + 5, 4, 55),
            "summary",
            Some(action(
                "action.onboarding.other-models",
                "See other models",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.onboarding.back",
            reference(DOCK_TOP + 5, 57, 75),
            "summary",
            Some(action("action.onboarding.back", "Back", false)),
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
                "action.onboarding.apply-default",
                "onboarding",
                "empty",
                "direct",
                "the recommended model is applied and first setup enters a usable conversation shell",
            ),
            transition(
                "action.onboarding.skip",
                "onboarding",
                "empty",
                "direct",
                "skip applies the same safe default before opening the empty conversation",
            ),
            transition(
                "action.onboarding.other-models",
                "onboarding",
                "model-picker",
                "direct",
                "other models opens the focused picker while retaining onboarding as the origin",
            ),
            transition(
                "action.onboarding.back",
                "onboarding",
                "onboarding",
                "cancel",
                "first-run back stays at the preceding onboarding step rather than inventing an origin",
            ),
        ],
    ))
}

fn render_model_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let blocked = fixture_id.contains("blocked");
    canvas.write(
        4,
        4,
        "You · Continue the retry conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Choose a model", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Each row says what changes before you apply it.",
        TextStyle::muted(),
    )?;
    let rows = [
        ("Claude Sonnet 4.5", "Current · Balanced speed and depth"),
        ("Claude Opus 4.8", "More depth for complex changes · slower"),
        (
            "GPT-5.2 Codex",
            "Fast implementation help · best for focused edits",
        ),
    ];
    let selected_index = if blocked { 2 } else { 1 };
    for (index, (label, consequence)) in rows.iter().enumerate() {
        let row = 12 + index as u16 * 3;
        let marker = if index == selected_index { "›" } else { " " };
        let style = if index == selected_index {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        canvas.write(row, 4, &format!("{marker} {label}"), style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        if blocked {
            "Selected · GPT-5.2 Codex · connection needed"
        } else {
            "Selected · Claude Opus 4.8"
        },
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        if blocked {
            "Connect this provider to use the selected model; your draft stays paused."
        } else {
            "Switching applies this choice: deeper reasoning for the next reply only."
        },
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        if blocked {
            "› [enter] Connect provider · [esc] Cancel"
        } else {
            "› [enter] Apply model · [esc] Cancel"
        },
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[↑↓] Choose · [f] Filter models · current choice stays unchanged until Apply",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Only the selected model's pending change is explained here.",
        TextStyle::muted(),
    )?;

    let selected_label = if blocked {
        "GPT-5.2 Codex"
    } else {
        "Claude Opus 4.8"
    };
    let primary_id = if blocked {
        "action.model-picker.connect"
    } else {
        "action.model-picker.apply"
    };
    let primary_label = if blocked {
        "Connect provider"
    } else {
        "Apply model"
    };
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "conversation-origin",
            reference(4, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "picker",
            "model-selection",
            reference(7, 4, 18),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "model-row",
            "Claude Sonnet 4.5",
            reference(12, 4, 22),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "model-row",
            "Claude Opus 4.8",
            reference(15, 4, 19),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "model-row",
            "GPT-5.2 Codex",
            reference(18, 4, 19),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            selected_label,
            reference(DOCK_TOP + 1, 4, 51),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            if blocked {
                "connection-needed"
            } else {
                "deeper-reasoning-next-reply"
            },
            reference(DOCK_TOP + 2, 4, 84),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            primary_id,
            reference(DOCK_TOP + 3, 4, 58),
            "focused",
            Some(action(primary_id, primary_label, false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.model-picker.cancel",
            reference(DOCK_TOP + 3, 59, 82),
            "summary",
            Some(action("action.model-picker.cancel", "Cancel", false)),
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
    let target = if blocked {
        "connect-provider"
    } else {
        "chat-turn"
    };
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "local-control",
        "filter-models",
        reference(DOCK_TOP + 4, 4, 17),
        "summary",
        Some(action("action.model-picker.filter", "Filter models", false)),
    ));
    Ok((
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![
            transition(
                primary_id,
                "model-picker",
                target,
                "direct",
                if blocked {
                    "the unavailable model opens provider connection without losing the conversation or draft"
                } else {
                    "the selected model applies once and returns to the exact conversation origin"
                },
            ),
            transition(
                "action.model-picker.cancel",
                "model-picker",
                "chat-turn",
                "cancel",
                "cancel discards the pending model and restores the exact conversation origin",
            ),
            transition(
                "action.model-picker.filter",
                "model-picker",
                "model-picker",
                "direct",
                "filtering changes only the visible model rows and leaves the pending choice intact",
            ),
        ],
    ))
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
