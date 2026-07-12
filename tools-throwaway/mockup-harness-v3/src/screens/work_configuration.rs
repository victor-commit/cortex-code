//! Flow 3 pilot states for the first configuration decision and model choice.
//!
//! Both states stay in the shared bottom action dock. The first-run screen
//! explains one decision and offers a safe default. The model picker compares
//! human-readable choices, then adds detail only for the focused pending row.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, ModelChoiceEvidence,
        RenderRef, SemanticCandidate, SemanticEvent, TransitionTrace, WorkStyleEvidence,
    },
    chrome::{
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const FLOW3_TASK: &str = "Choose a model and how Code should work.";
const FLOW3_ORIGIN: &str = "Work configuration · onboarding";
const DOCK_TOP: u16 = 31;
const ONBOARDING_DOCK_TOP: u16 = 25;
const CODE_BANNER: [&str; 6] = [
    " ██████              ██",
    "██                   ██",
    "██        ████    █████   ████",
    "██       ██  ██  ██  ██  ██████",
    "██       ██  ██  ██  ██  ██",
    " ██████   ████    █████   █████",
];

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
        "agent-picker" => render_agent_picker(&mut canvas, screen_id, fixture_id)?,
        "mode-switch" => render_mode_switch(&mut canvas, screen_id, fixture_id)?,
        "mode-switch-refused" => render_mode_switch_refused(&mut canvas, screen_id, fixture_id)?,
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
        work_style: match screen_id {
            "onboarding" | "model-picker" | "agent-picker" => Some(WorkStyleEvidence {
                applied: "Careful edits".to_owned(),
                pending: (screen_id == "agent-picker" && !fixture_id.contains("current-only"))
                    .then(|| {
                        if fixture_id.contains("unchanged") {
                            "Quick implementation".to_owned()
                        } else {
                            "Autonomous changes".to_owned()
                        }
                    }),
                access_scope: "asks before editing project files".to_owned(),
            }),
            "mode-switch" => Some(WorkStyleEvidence {
                applied: "Careful edits".to_owned(),
                pending: Some("Autonomous changes".to_owned()),
                access_scope: "asks before editing project files".to_owned(),
            }),
            "mode-switch-refused" => Some(WorkStyleEvidence {
                applied: "Careful edits".to_owned(),
                pending: None,
                access_scope: "asks before editing project files".to_owned(),
            }),
            _ => None,
        },
        model_choice: match screen_id {
            "onboarding" => Some(ModelChoiceEvidence {
                applied: "Claude Sonnet 4.5".to_owned(),
                pending: None,
                origin: "Onboarding".to_owned(),
                pending_action: Some(if is_gate_fixture(fixture_id) {
                    "Review model choices".to_owned()
                } else {
                    "Continue".to_owned()
                }),
                return_policy: "Return to onboarding".to_owned(),
            }),
            "model-picker" => Some(ModelChoiceEvidence {
                applied: "Claude Sonnet 4.5".to_owned(),
                pending: Some(if fixture_id.contains("blocked") {
                    "GPT-5.2 Codex".to_owned()
                } else {
                    "Claude Opus 4.8".to_owned()
                }),
                origin: if is_onboarding_origin(fixture_id) {
                    "Onboarding".to_owned()
                } else {
                    "Current conversation".to_owned()
                },
                pending_action: Some(if fixture_id.contains("blocked") {
                    "Connect provider".to_owned()
                } else {
                    "Apply model".to_owned()
                }),
                return_policy: if is_onboarding_origin(fixture_id) {
                    "Return to onboarding".to_owned()
                } else {
                    "Return to conversation".to_owned()
                },
            }),
            _ => None,
        },
        conversation: (screen_id == "mode-switch" && is_gate_fixture(fixture_id)).then(|| {
            ConversationIdentity {
                conversation_id: "conversation.configuration".to_owned(),
                transcript_id: "transcript.configuration".to_owned(),
                session_id: "session.configuration.1".to_owned(),
                turn_id: "turn.configuration.1".to_owned(),
                user_task: FLOW3_TASK.to_owned(),
                visible_origin: FLOW3_ORIGIN.to_owned(),
                submitted_prompt: None,
                paused_activity_id: None,
                later_draft: None,
            }
        }),
    })
}

fn render_onboarding(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let gate_origin = is_gate_fixture(fixture_id);
    for (offset, line) in CODE_BANNER.iter().enumerate() {
        canvas.write(2 + offset as u16, 4, line, TextStyle::accent())?;
    }
    canvas.write(10, 4, "Set up your first conversation", TextStyle::bright())?;
    canvas.write(
        12,
        4,
        "Choose the model Code will use when you begin.",
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "The recommended model is a safe everyday default.",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "You can change it later. You do not need to connect an account yet.",
        TextStyle::muted(),
    )?;
    canvas.write(
        20,
        4,
        "How Code should work: Careful edits, which asks before changing files.",
        TextStyle::text(),
    )?;
    canvas.write(
        23,
        4,
        "Review the safe recommendation below, then continue or choose another model.",
        TextStyle::muted(),
    )?;
    write_onboarding_dock_border(canvas)?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 1,
        "Recommended starting model",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 2,
        "Claude Sonnet 4.5",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 3,
        "Balanced speed and depth for everyday coding conversations.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 4,
        "Works with Careful edits, which asks before changing files.",
        TextStyle::muted(),
    )?;
    dock_line(canvas, ONBOARDING_DOCK_TOP + 5, "", TextStyle::text())?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 6,
        "› [enter] Continue",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 7,
        "  [s] Skip with this safe default",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 8,
        "  [m] Review other models",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 9,
        "  [esc] Back to the previous setup step",
        TextStyle::muted(),
    )?;
    dock_line(canvas, ONBOARDING_DOCK_TOP + 10, "", TextStyle::text())?;
    dock_line(
        canvas,
        ONBOARDING_DOCK_TOP + 11,
        "Continue accepts the recommendation. Skip keeps the same safe default.",
        TextStyle::muted(),
    )?;

    let pairs = vec![
        semantic_with_refs(
            screen_id,
            fixture_id,
            "entry-brand",
            "code-ascii-banner",
            (2..=7).map(|row| reference(row, 4, 39)).collect(),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "surface",
            "onboarding-action-dock",
            reference(10, 4, 34),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "decision",
            "first-model-choice",
            reference(12, 4, 53),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "default",
            "recommended-claude-sonnet-4-5",
            reference(ONBOARDING_DOCK_TOP + 1, 4, 30),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "balanced-speed-and-depth",
            reference(ONBOARDING_DOCK_TOP + 3, 4, 63),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(20, 4, 73),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            if gate_origin {
                "action.onboarding.review-models"
            } else {
                "action.onboarding.apply-default"
            },
            reference(ONBOARDING_DOCK_TOP + 6, 4, 24),
            "focused",
            Some(action(
                if gate_origin {
                    "action.onboarding.review-models"
                } else {
                    "action.onboarding.apply-default"
                },
                "Continue",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.onboarding.skip",
            reference(ONBOARDING_DOCK_TOP + 7, 4, 37),
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
            reference(ONBOARDING_DOCK_TOP + 8, 4, 29),
            "summary",
            Some(action(
                "action.onboarding.other-models",
                "Review other models",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.onboarding.back",
            reference(ONBOARDING_DOCK_TOP + 9, 4, 43),
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
                if gate_origin {
                    "action.onboarding.review-models"
                } else {
                    "action.onboarding.apply-default"
                },
                "onboarding",
                if gate_origin { "model-picker" } else { "empty" },
                "direct",
                if gate_origin {
                    "the configuration storyboard opens model choices without applying a pending value"
                } else {
                    "the recommended model is applied and first setup enters a usable conversation shell"
                },
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
    let gate_origin = is_onboarding_origin(fixture_id);
    if gate_origin {
        canvas.write(3, 4, "Work configuration · onboarding", TextStyle::muted())?;
        canvas.write(
            4,
            4,
            "Your first conversation is waiting for this choice.",
            TextStyle::bright(),
        )?;
    } else {
        canvas.write(
            4,
            4,
            "You · Continue the paused conversation.",
            TextStyle::bright(),
        )?;
    }
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
            "work-style",
            "work_style",
            reference(6, 4, 76),
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
    } else if gate_origin {
        "onboarding"
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
                } else if gate_origin {
                    "the selected model applies once and returns to the onboarding configuration hub"
                } else {
                    "the selected model applies once and returns to the exact conversation origin"
                },
            ),
            transition(
                "action.model-picker.cancel",
                "model-picker",
                if gate_origin {
                    "onboarding"
                } else {
                    "chat-turn"
                },
                "cancel",
                if gate_origin {
                    "cancel discards the pending model and restores the onboarding configuration hub"
                } else {
                    "cancel discards the pending model and restores the exact conversation origin"
                },
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

fn render_agent_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let unchanged = fixture_id.contains("unchanged");
    let current_only = fixture_id.contains("current-only");
    let gate_origin = is_gate_fixture(fixture_id);
    let selected = if current_only {
        "Careful edits"
    } else if unchanged {
        "Quick implementation"
    } else {
        "Autonomous changes"
    };
    if gate_origin {
        canvas.write(3, 4, FLOW3_ORIGIN, TextStyle::muted())?;
        canvas.write(
            4,
            4,
            "Choose how Code should work for this conversation.",
            TextStyle::bright(),
        )?;
    } else {
        canvas.write(
            4,
            4,
            "You · Continue the paused conversation.",
            TextStyle::bright(),
        )?;
    }
    canvas.write(7, 4, "How Code should work", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Choose the outcome you want. Your current choice stays active until you apply.",
        TextStyle::muted(),
    )?;
    let rows = [
        (
            "Careful edits",
            "You approve each project-file edit before it happens.",
        ),
        (
            "Quick implementation",
            "Focused edits happen now; you review them together at the end.",
        ),
        (
            "Autonomous changes",
            "Project-file edits can happen without asking each time.",
        ),
    ];
    let selected_index = rows
        .iter()
        .position(|(label, _)| *label == selected)
        .expect("work-style fixture must select one known row");
    for (index, (label, consequence)) in rows.iter().enumerate() {
        let row = 12 + index as u16 * 3;
        let marker = if index == selected_index { "›" } else { " " };
        let style = if index == selected_index {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        let visible_label = if index == 0 {
            format!("{marker} Current · {label}")
        } else if index == selected_index {
            format!("{marker} Pending selection · {label}")
        } else {
            format!("{marker} Available · {label}")
        };
        canvas.write(row, 4, &visible_label, style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }
    write_dock_border(canvas)?;
    let selected_summary = if current_only {
        "Current · Careful edits · no pending change".to_owned()
    } else {
        format!("Pending selection · {selected}")
    };
    dock_line(canvas, DOCK_TOP + 1, &selected_summary, TextStyle::bright())?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        if current_only {
            "Careful edits remains applied; choose another row to preview an outcome."
        } else if unchanged {
            "Keeps current access; this choice applies directly."
        } else {
            "Changes access; review the concrete project scope before applying."
        },
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        if current_only {
            "› [enter] Keep Careful edits · [esc] Cancel"
        } else if unchanged {
            "› [enter] Use Quick implementation · [esc] Cancel"
        } else {
            "› [enter] Review access change · [esc] Cancel"
        },
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[↑↓] Choose · each row explains how Code will work",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Each row shows its outcome; access details open after review.",
        TextStyle::muted(),
    )?;

    let primary_id = if current_only {
        "action.agent-picker.keep-current"
    } else if unchanged {
        "action.agent-picker.apply-unchanged"
    } else {
        "action.agent-picker.review-access"
    };
    let primary_label = if current_only {
        "Keep Careful edits"
    } else if unchanged {
        "Use Quick implementation"
    } else {
        "Review access change"
    };
    let target = if current_only || unchanged {
        "chat-turn"
    } else {
        "mode-switch"
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
            "decision",
            "work-style-choice",
            reference(7, 4, 22),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(7, 4, 22),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style-row",
            "careful-edits",
            reference(12, 4, 28),
            if current_only { "focused" } else { "summary" },
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style-row",
            "quick-implementation",
            reference(15, 4, 45),
            if unchanged { "focused" } else { "summary" },
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style-row",
            "autonomous-changes",
            reference(18, 4, 42),
            if unchanged || current_only {
                "summary"
            } else {
                "focused"
            },
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-outcome",
            selected,
            reference(DOCK_TOP + 1, 4, 33),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            if current_only {
                "no-pending-change"
            } else if unchanged {
                "same-access-applies-directly"
            } else {
                "access-review-needed"
            },
            reference(DOCK_TOP + 2, 4, 70),
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
            "action.agent-picker.cancel",
            reference(DOCK_TOP + 3, 59, 82),
            "summary",
            Some(action("action.agent-picker.cancel", "Cancel", false)),
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
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "local-control",
        "choose-work-style",
        reference(DOCK_TOP + 4, 4, 15),
        "summary",
        Some(action(
            "action.agent-picker.choose",
            "Choose work style",
            false,
        )),
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
                "agent-picker",
                target,
                "direct",
                if current_only {
                    "the current work style remains applied and returns to the paused conversation"
                } else if unchanged {
                    "the unchanged-access choice applies directly and returns to the paused conversation"
                } else {
                    "the access-changing choice opens one focused access review before it can apply"
                },
            ),
            transition(
                "action.agent-picker.cancel",
                "agent-picker",
                if gate_origin {
                    "onboarding"
                } else {
                    "chat-turn"
                },
                "cancel",
                if gate_origin {
                    "cancel keeps the current work style and restores the onboarding configuration hub"
                } else {
                    "cancel keeps the current work style and restores the exact conversation origin"
                },
            ),
        ],
    ))
}

fn render_mode_switch(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let onboarding_origin = is_gate_fixture(fixture_id);
    if onboarding_origin {
        canvas.write(3, 4, FLOW3_ORIGIN, TextStyle::muted())?;
        canvas.write(4, 4, FLOW3_TASK, TextStyle::bright())?;
    } else {
        canvas.write(
            4,
            4,
            "You · Continue the paused conversation.",
            TextStyle::bright(),
        )?;
    }
    canvas.write(7, 4, "Review a work-style change", TextStyle::bright())?;
    canvas.write(9, 4, "Requested: Autonomous changes", TextStyle::text())?;
    canvas.write(
        11,
        4,
        "How Code should work: edit project files without asking each time.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "Scope: this project · until you choose another work style.",
        TextStyle::muted(),
    )?;
    canvas.write(
        15,
        4,
        "Your transcript and draft stay unchanged while this is pending.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Pending · Careful edits remains active",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "The requested access applies only after you confirm.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Apply access change · [esc] Keep Careful edits",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "One confirmation changes the work style; Cancel changes nothing.",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Return to How Code should work to choose another outcome.",
        TextStyle::muted(),
    )?;

    let pairs = [
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
            "work-style",
            "work_style",
            reference(7, 4, 30),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "requested-behavior",
            "autonomous-changes",
            reference(9, 4, 38),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "capability-scope",
            "project-file-access",
            reference(13, 4, 80),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "origin-preserved",
            reference(15, 4, 64),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-state",
            "careful-edits-remains-active",
            reference(DOCK_TOP + 1, 4, 43),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.mode-switch.confirm",
            reference(DOCK_TOP + 3, 4, 60),
            "focused",
            Some(action(
                "action.mode-switch.confirm",
                "Apply access change",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.mode-switch.cancel",
            reference(DOCK_TOP + 3, 61, 85),
            "summary",
            Some(action(
                "action.mode-switch.cancel",
                "Keep Careful edits",
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
                "action.mode-switch.confirm",
                "mode-switch",
                "chat-turn",
                "direct",
                "confirmation applies the one work_style value and returns to the paused conversation",
            ),
            transition(
                "action.mode-switch.cancel",
                "mode-switch",
                "agent-picker",
                "cancel",
                if onboarding_origin {
                    "cancel leaves Careful edits and its current access active, then returns to the work-style choice while preserving the onboarding origin"
                } else {
                    "cancel leaves Careful edits and its current access active, then returns to the work-style choice"
                },
            ),
        ],
    ))
}

fn render_mode_switch_refused(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Access change not applied", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Autonomous changes need approval for this project.",
        TextStyle::text(),
    )?;
    canvas.write(
        11,
        4,
        "You are still using Careful edits, which asks before editing files.",
        TextStyle::text(),
    )?;
    canvas.write(
        13,
        4,
        "Your access, transcript and draft are unchanged.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Not applied · Careful edits remains active",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Quick implementation keeps the same access and adds review at the end.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Choose Quick implementation · same access",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  [esc] Return to work",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Nothing was applied; Careful edits remains the only applied choice.",
        TextStyle::muted(),
    )?;

    let pairs = [
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
            "work-style",
            "work_style",
            reference(11, 4, 75),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "refusal",
            "access-change-not-applied",
            reference(7, 4, 32),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "preservation",
            "origin-preserved",
            reference(13, 4, 53),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.mode-switch-refused.choose-unchanged",
            reference(DOCK_TOP + 3, 4, 52),
            "focused",
            Some(action(
                "action.mode-switch-refused.choose-unchanged",
                "Choose Quick implementation · same access",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.mode-switch-refused.return",
            reference(DOCK_TOP + 4, 4, 25),
            "summary",
            Some(action(
                "action.mode-switch-refused.return",
                "Return to work",
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
                "action.mode-switch-refused.choose-unchanged",
                "mode-switch-refused",
                "agent-picker",
                "direct",
                "the safe recovery opens Quick implementation as an unchanged-access choice while Careful edits remains active",
            ),
            transition(
                "action.mode-switch-refused.return",
                "mode-switch-refused",
                "chat-turn",
                "cancel",
                "return restores the paused conversation with its draft and access unchanged",
            ),
        ],
    ))
}

fn is_gate_fixture(fixture_id: &str) -> bool {
    fixture_id.starts_with("gate.work-configuration.")
}

fn is_onboarding_origin(fixture_id: &str) -> bool {
    is_gate_fixture(fixture_id) || fixture_id.starts_with("case.onboarding-unusable-model.")
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

fn write_onboarding_dock_border(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        ONBOARDING_DOCK_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        ONBOARDING_DOCK_TOP + 12,
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
    semantic_with_refs(
        screen_id,
        fixture_id,
        family,
        subject_id,
        vec![render_ref],
        detail_level,
        action,
    )
}

fn semantic_with_refs(
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_refs: Vec<RenderRef>,
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
        render_refs,
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
