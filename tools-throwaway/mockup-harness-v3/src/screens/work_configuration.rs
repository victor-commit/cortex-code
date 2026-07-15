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
        components::{subject_title_for_origin, ENTRY_CODE_BANNER},
        layout::{ACTION_DOCK_TOP, CANVAS_ROWS, ScreenComposition, StatusLine},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const FLOW3_SETUP_TASK: &str = "Choose a model and how Code should work.";
const FLOW3_ONBOARDING_ORIGIN: &str = "Work configuration · onboarding";
const FLOW3_CURRENT_ORIGIN: &str = "Work configuration · current conversation";
const DOCK_TOP: u16 = ACTION_DOCK_TOP;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let model_name = if matches!(screen_id, "variant-picker") {
        "Claude Opus 4.8"
    } else {
        MODEL
    };
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, model_name)?;
    ScreenComposition::new(screen_id, Surface::ActionDock, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "onboarding" => render_onboarding(&mut canvas, screen_id, fixture_id)?,
        "model-picker" => render_model_picker(&mut canvas, screen_id, fixture_id)?,
        "model-picker-filtered" => render_model_picker_filtered(&mut canvas, screen_id, fixture_id)?,
        "agent-picker" => render_agent_picker(&mut canvas, screen_id, fixture_id)?,
        "skills-picker" => render_skills_picker(&mut canvas, screen_id, fixture_id)?,
        "subagent-picker" => render_subagent_picker(&mut canvas, screen_id, fixture_id)?,
        "variant-picker" => render_variant_picker(&mut canvas, screen_id, fixture_id)?,
        "theme-picker" => render_theme_picker(&mut canvas, screen_id, fixture_id)?,
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
            "onboarding" | "model-picker" | "agent-picker" | "skills-picker"
            | "subagent-picker" | "variant-picker" | "theme-picker" => Some(WorkStyleEvidence {
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
                pending: Some(if fixture_id.contains("skills") {
                    "Research the web".to_owned()
                } else {
                    "Autonomous changes".to_owned()
                }),
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
            "model-picker" | "model-picker-filtered" => {
                let no_match = screen_id == "model-picker-filtered"
                    && (fixture_id.contains("no-match") || fixture_id.contains("zzq"));
                Some(ModelChoiceEvidence {
                    applied: "Claude Sonnet 4.5".to_owned(),
                    pending: if no_match {
                        None
                    } else {
                        Some(if fixture_id.contains("blocked") {
                            "GPT-5.2 Codex".to_owned()
                        } else {
                            "Claude Opus 4.8".to_owned()
                        })
                    },
                    origin: if is_onboarding_origin(fixture_id) {
                        "Onboarding".to_owned()
                    } else {
                        "Current conversation".to_owned()
                    },
                    pending_action: if no_match {
                        Some("Clear filter".to_owned())
                    } else {
                        Some(if fixture_id.contains("blocked") {
                            "Connect provider".to_owned()
                        } else {
                            "Apply model".to_owned()
                        })
                    },
                    return_policy: if is_onboarding_origin(fixture_id) {
                        "Return to onboarding".to_owned()
                    } else {
                        "Return to conversation".to_owned()
                    },
                })
            }
            _ => None,
        },
        conversation: (screen_id == "mode-switch" && is_gate_fixture(fixture_id)).then(|| {
            ConversationIdentity {
                conversation_id: "conversation.configuration".to_owned(),
                transcript_id: "transcript.configuration".to_owned(),
                session_id: "session.configuration.1".to_owned(),
                turn_id: "turn.configuration.1".to_owned(),
                user_task: FLOW3_SETUP_TASK.to_owned(),
                visible_origin: FLOW3_CURRENT_ORIGIN.to_owned(),
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
    for (offset, line) in ENTRY_CODE_BANNER.iter().enumerate() {
        canvas.write(2 + offset as u16, 4, line, TextStyle::accent())?;
    }
    // Keep body content tight above the standard bottom dock (rows 20-30).
    canvas.write(20, 4, "Set up your first conversation", TextStyle::bright())?;
    canvas.write(
        22,
        4,
        "Choose the model Code will use when you begin.",
        TextStyle::text(),
    )?;
    canvas.write(
        23,
        4,
        "Recommended: Claude Sonnet 4.5 · balanced everyday coding.",
        TextStyle::text(),
    )?;
    canvas.write(
        25,
        4,
        "You can change it later. You do not need to connect an account yet.",
        TextStyle::muted(),
    )?;
    canvas.write(
        27,
        4,
        "How Code should work: Careful edits, which asks before changing files.",
        TextStyle::text(),
    )?;
    canvas.write(
        29,
        4,
        "Continue uses the safe recommendation in the dock.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Claude Sonnet 4.5 · recommended starting model",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "› Continue",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Skip with this safe default",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  Review other models",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "Enter to continue · s to skip · m for models · Esc to go back",
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
            "action-dock",
            reference(20, 4, 34),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "decision",
            "first-model-choice",
            reference(22, 4, 53),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "default",
            "recommended-claude-sonnet-4-5",
            reference(DOCK_TOP + 1, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "consequence",
            "balanced-speed-and-depth",
            reference(23, 4, 64),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(27, 4, 73),
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
            reference(DOCK_TOP + 2, 4, 24),
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
            reference(DOCK_TOP + 3, 4, 37),
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
            reference(DOCK_TOP + 4, 4, 29),
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
            reference(DOCK_TOP + 5, 4, 16),
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
    let onboarding_chapter = is_onboarding_origin(fixture_id);
    if onboarding_chapter {
        canvas.write(3, 4, FLOW3_ONBOARDING_ORIGIN, TextStyle::muted())?;
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
            "› Connect provider · Esc to Cancel"
        } else {
            "› Apply model · Esc to Cancel"
        },
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "↑↓ to Choose · f for Filter models · current choice stays unchanged until Apply",
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
    } else if onboarding_chapter {
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
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "filter",
        "filterable-model-catalog",
        reference(DOCK_TOP + 4, 18, 36),
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
                primary_id,
                "model-picker",
                target,
                "direct",
                if blocked {
                    "the unavailable model opens provider connection without losing the conversation or draft"
                } else if onboarding_chapter {
                    "the selected model applies once and returns to the onboarding configuration hub"
                } else {
                    "the selected model applies once and returns to the exact conversation origin"
                },
            ),
            transition(
                "action.model-picker.cancel",
                "model-picker",
                if onboarding_chapter {
                    "onboarding"
                } else {
                    "chat-turn"
                },
                "cancel",
                if onboarding_chapter {
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

fn render_model_picker_filtered(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let no_match = fixture_id.contains("no-match") || fixture_id.contains("zzq");
    let query = if no_match { "zzq" } else { "claude" };
    let catalog = [
        ("Claude Sonnet 4.5", "Current · Balanced speed and depth"),
        ("Claude Opus 4.8", "More depth for complex changes · slower"),
        (
            "GPT-5.2 Codex",
            "Fast implementation help · best for focused edits",
        ),
    ];
    let matches: Vec<(usize, &str, &str)> = catalog
        .iter()
        .enumerate()
        .filter(|(_, (label, consequence))| {
            let haystack = format!("{label} {consequence}").to_ascii_lowercase();
            haystack.contains(query)
        })
        .map(|(index, (label, consequence))| (index, *label, *consequence))
        .collect();
    let total = catalog.len();
    let match_count = matches.len();

    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Choose a model", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        &format!("Filter · {query} · {match_count} of {total} matching"),
        TextStyle::text(),
    )?;
    canvas.write(
        10,
        4,
        "Only models that match this query stay visible. Original order is kept.",
        TextStyle::muted(),
    )?;

    let selected_label = if no_match {
        None
    } else {
        Some("Claude Opus 4.8")
    };
    for (visible_index, (_original, label, consequence)) in matches.iter().enumerate() {
        let row = 12 + visible_index as u16 * 3;
        let selected = selected_label == Some(*label);
        let marker = if selected { "›" } else { " " };
        let style = if selected {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        canvas.write(row, 4, &format!("{marker} {label}"), style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }
    if no_match {
        canvas.write(14, 4, "No models match zzq.", TextStyle::bright())?;
        canvas.write(
            16,
            4,
            "Edit the query or clear the filter to restore every model choice.",
            TextStyle::muted(),
        )?;
    }

    write_dock_border(canvas)?;
    if no_match {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Filter active · no matching models",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Clearing restores the full model list without leaving this picker.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› c for Clear filter · Esc to Cancel",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "Type to edit the query · applied model stays unchanged",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Apply stays unavailable until a matching model remains.",
            TextStyle::muted(),
        )?;
    } else {
        dock_line(
            canvas,
            DOCK_TOP + 1,
            "Selected · Claude Opus 4.8 · 2 of 3 matching",
            TextStyle::bright(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 2,
            "Switching applies this choice: deeper reasoning for the next reply only.",
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "› Apply model · c for Clear filter · Esc to Cancel",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "↑↓ to Choose · query stays visible · current choice stays until Apply",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Nonmatching models stay hidden; relative order of matches is unchanged.",
            TextStyle::muted(),
        )?;
    }

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
            "filter",
            if no_match { "query-zzq" } else { "query-claude" },
            reference(9, 4, 48),
            "focused",
            None,
        ),
    ];
    for (visible_index, (_original, label, _)) in matches.iter().enumerate() {
        let selected = selected_label == Some(*label);
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "model-row",
            label,
            reference(12 + visible_index as u16 * 3, 4, 22),
            if selected { "focused" } else { "summary" },
            None,
        ));
    }
    if no_match {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            "no-models-match-zzq",
            reference(14, 4, 28),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.model-picker.clear-filter",
            reference(DOCK_TOP + 3, 4, 24),
            "focused",
            Some(action(
                "action.model-picker.clear-filter",
                "Clear filter",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.model-picker.cancel",
            reference(DOCK_TOP + 3, 25, 42),
            "summary",
            Some(action("action.model-picker.cancel", "Cancel", false)),
        ));
    } else {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            "Claude Opus 4.8",
            reference(DOCK_TOP + 1, 4, 51),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            "deeper-reasoning-next-reply",
            reference(DOCK_TOP + 2, 4, 84),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.model-picker.apply",
            reference(DOCK_TOP + 3, 4, 28),
            "focused",
            Some(action("action.model-picker.apply", "Apply model", false)),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.model-picker.clear-filter",
            reference(DOCK_TOP + 3, 29, 50),
            "summary",
            Some(action(
                "action.model-picker.clear-filter",
                "Clear filter",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.model-picker.cancel",
            reference(DOCK_TOP + 3, 51, 70),
            "summary",
            Some(action("action.model-picker.cancel", "Cancel", false)),
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
            "action.model-picker.clear-filter",
            "model-picker-filtered",
            "model-picker",
            "direct",
            "clearing restores the complete model list in place without leaving the picker",
        ),
        transition(
            "action.model-picker.cancel",
            "model-picker-filtered",
            "chat-turn",
            "cancel",
            "cancel discards the filter and pending model and restores the exact conversation origin",
        ),
    ];
    if !no_match {
        transitions.insert(
            0,
            transition(
                "action.model-picker.apply",
                "model-picker-filtered",
                "chat-turn",
                "direct",
                "the selected matching model applies once and restores the exact conversation origin",
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
        canvas.write(3, 4, FLOW3_CURRENT_ORIGIN, TextStyle::muted())?;
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
            "› Keep Careful edits · Esc to Cancel"
        } else if unchanged {
            "› Use Quick implementation · Esc to Cancel"
        } else {
            "› Review access change · Esc to Cancel"
        },
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "↑↓ to Choose · each row explains how Code will work",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No filter · three work-style choices fit on one screen",
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
    pairs.push(semantic(
        screen_id,
        fixture_id,
        "filter-not-needed",
        "no-filter-three-work-styles",
        reference(DOCK_TOP + 5, 4, 62),
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

fn render_skills_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let escalating = fixture_id.contains("access") || fixture_id.contains("escalat");
    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Choose session skills", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Each skill names the result it helps achieve. Changes stay pending until Apply.",
        TextStyle::muted(),
    )?;
    // Applied set: Commit cleanly, Design reviews. Pending: also Review images; escalating adds Research the web.
    let rows = [
        (
            "Commit cleanly",
            "On · Keep commits under the linked identity",
            true,
            false,
        ),
        (
            "Design reviews",
            "On · Keep screens simple and readable",
            true,
            false,
        ),
        (
            "Review images",
            "Pending on · Check rendered interfaces for clarity",
            false,
            true,
        ),
        (
            "Research the web",
            if escalating {
                "Pending on · Search and summarize public web content"
            } else {
                "Off · Search and summarize public web content"
            },
            false,
            escalating,
        ),
    ];
    let selected_index = if escalating { 3 } else { 2 };
    for (index, (label, consequence, _, _)) in rows.iter().enumerate() {
        let row = 11 + index as u16 * 2;
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
    let selected = rows[selected_index];
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "Selected · {} · {}",
            selected.0,
            if escalating {
                "expands access"
            } else {
                "session-local"
            }
        ),
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        if escalating {
            "Turning this on expands capability; review the access scope before applying."
        } else {
            "Pending set · Commit cleanly, Design reviews, Review images · apply once."
        },
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        if escalating {
            "› Review access change · Esc to Cancel"
        } else {
            "› Apply skills · Esc to Cancel"
        },
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[space] Toggle · ↑↓ to Move · enabled state is labeled on each row",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No filter · four session skills fit on one screen",
        TextStyle::muted(),
    )?;

    let primary_id = if escalating {
        "action.skills-picker.review-access"
    } else {
        "action.skills-picker.apply"
    };
    let primary_label = if escalating {
        "Review access change"
    } else {
        "Apply skills"
    };
    let target = if escalating {
        "mode-switch"
    } else {
        "chat-turn"
    };
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
            "picker",
            "skills-selection",
            reference(7, 4, 24),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "skill-row",
            "commit-cleanly",
            reference(11, 4, 20),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "skill-row",
            "design-reviews",
            reference(13, 4, 20),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "skill-row",
            "review-images",
            reference(15, 4, 20),
            if escalating { "summary" } else { "focused" },
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "skill-row",
            "research-the-web",
            reference(17, 4, 22),
            if escalating { "focused" } else { "summary" },
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            selected.0,
            reference(DOCK_TOP + 1, 4, 50),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            if escalating {
                "access-review-needed"
            } else {
                "atomic-pending-set"
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
            "action.skills-picker.cancel",
            reference(DOCK_TOP + 3, 59, 82),
            "summary",
            Some(action("action.skills-picker.cancel", "Cancel", false)),
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
            "filter-not-needed",
            "no-filter-four-session-skills",
            reference(DOCK_TOP + 5, 4, 58),
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
                primary_id,
                "skills-picker",
                target,
                "direct",
                if escalating {
                    "a capability-expanding skill toggles into the same access-guard confirmation used by work style"
                } else {
                    "Apply commits the exact pending skill set atomically and returns to the paused conversation"
                },
            ),
            transition(
                "action.skills-picker.cancel",
                "skills-picker",
                "chat-turn",
                "cancel",
                "cancel restores the prior skill set, focus, draft, and transcript exactly",
            ),
        ],
    ))
}

fn render_subagent_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let _ = fixture_id;
    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Delegate this task", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Pick a helper for the current task only. Global work style stays unchanged.",
        TextStyle::muted(),
    )?;
    let rows = [
        (
            "Explore auth boundaries",
            "Find existing auth middleware and the shared guard path.",
        ),
        (
            "Summarize findings",
            "Turn the open notes into a short review for this turn.",
        ),
        (
            "Plan the next edit",
            "Outline a narrow change without applying it.",
        ),
    ];
    let selected_index = 0;
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
        "Selected · Explore auth boundaries · this task only",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Delegation stays task-local; your session work style is unchanged.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› Use this helper · Esc to Cancel",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "↑↓ to Choose · outcome describes fit for the current task",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No filter · three task helpers fit on one screen",
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
            "picker",
            "subagent-selection",
            reference(7, 4, 22),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "subagent-row",
            "explore-auth-boundaries",
            reference(12, 4, 32),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "subagent-row",
            "summarize-findings",
            reference(15, 4, 26),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "subagent-row",
            "plan-the-next-edit",
            reference(18, 4, 24),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            "explore-auth-boundaries",
            reference(DOCK_TOP + 1, 4, 58),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            "task-local-delegation",
            reference(DOCK_TOP + 2, 4, 72),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.subagent-picker.apply",
            reference(DOCK_TOP + 3, 4, 50),
            "focused",
            Some(action(
                "action.subagent-picker.apply",
                "Use this helper",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.subagent-picker.cancel",
            reference(DOCK_TOP + 3, 51, 75),
            "summary",
            Some(action("action.subagent-picker.cancel", "Cancel", false)),
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
            "filter-not-needed",
            "no-filter-three-task-helpers",
            reference(DOCK_TOP + 5, 4, 56),
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
                "action.subagent-picker.apply",
                "subagent-picker",
                "chat-turn",
                "direct",
                "using the helper keeps the choice task-local and returns to the paused conversation",
            ),
            transition(
                "action.subagent-picker.cancel",
                "subagent-picker",
                "chat-turn",
                "cancel",
                "cancel restores the exact conversation origin with no helper started",
            ),
        ],
    ))
}

fn render_variant_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let _ = fixture_id;
    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Choose a response variant", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Base model · Claude Opus 4.8 · the variant only changes deliberation for the next replies.",
        TextStyle::muted(),
    )?;
    let rows = [
        (
            "Fast",
            "Least deliberation · quickest replies for narrow edits.",
        ),
        (
            "Balanced",
            "Current · Moderate wait with careful everyday reasoning.",
        ),
        (
            "Thorough",
            "More careful, slower replies for risky or multi-step changes.",
        ),
    ];
    let selected_index = 2;
    for (index, (label, consequence)) in rows.iter().enumerate() {
        let row = 12 + index as u16 * 3;
        let marker = if index == selected_index { "›" } else { " " };
        let style = if index == selected_index {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        let visible = if index == 1 {
            format!("{marker} Current · {label}")
        } else if index == selected_index {
            format!("{marker} Pending · {label}")
        } else {
            format!("{marker} {label}")
        };
        canvas.write(row, 4, &visible, style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Selected · Thorough · keeps Claude Opus 4.8",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Apply changes only the response variant; the base model stays the same.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› Apply variant · Esc to Cancel",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "↑↓ to Choose · each row explains the practical tradeoff",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No filter · three response variants fit on one screen",
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
            "picker",
            "variant-selection",
            reference(7, 4, 28),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "variant-row",
            "fast",
            reference(12, 4, 10),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "variant-row",
            "balanced",
            reference(15, 4, 24),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "variant-row",
            "thorough",
            reference(18, 4, 22),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            "thorough",
            reference(DOCK_TOP + 1, 4, 48),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "pending-consequence",
            "variant-only-change",
            reference(DOCK_TOP + 2, 4, 78),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.variant-picker.apply",
            reference(DOCK_TOP + 3, 4, 48),
            "focused",
            Some(action(
                "action.variant-picker.apply",
                "Apply variant",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.variant-picker.cancel",
            reference(DOCK_TOP + 3, 49, 70),
            "summary",
            Some(action("action.variant-picker.cancel", "Cancel", false)),
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
            "filter-not-needed",
            "no-filter-three-response-variants",
            reference(DOCK_TOP + 5, 4, 60),
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
                "action.variant-picker.apply",
                "variant-picker",
                "chat-turn",
                "direct",
                "Apply changes only the response variant and returns with the same base model",
            ),
            transition(
                "action.variant-picker.cancel",
                "variant-picker",
                "chat-turn",
                "cancel",
                "cancel restores the previous variant and conversation origin",
            ),
        ],
    ))
}

fn render_theme_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let _previewing_light = fixture_id.contains("preview") || fixture_id.contains("light");
    canvas.write(
        4,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Choose a theme", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "Moving focus previews colors temporarily. Apply commits; Cancel restores the applied palette.",
        TextStyle::muted(),
    )?;
    let rows = [
        ("Dark", "Applied · Warm default palette for daily work."),
        ("Light", "Temporary preview · cooler daylight contrast."),
        ("VS Code dark", "Familiar editor neutrals with a blue accent."),
        ("Monokai", "Higher-contrast vivid coding palette."),
    ];
    // Default published frame is the temporary Light preview with Dark still applied.
    let selected_index = 1;
    for (index, (label, consequence)) in rows.iter().enumerate() {
        let row = 11 + index as u16 * 2;
        let marker = if index == selected_index { "›" } else { " " };
        let style = if index == selected_index {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        let visible = if index == 0 {
            format!("{marker} Current · {label}")
        } else if index == selected_index {
            format!("{marker} Preview · {label}")
        } else {
            format!("{marker} {label}")
        };
        canvas.write(row, 4, &visible, style)?;
        canvas.write(row + 1, 6, consequence, TextStyle::muted())?;
    }
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Selected · Light · temporary preview of the daylight palette",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Preview is temporary until Apply; Cancel restores Dark exactly.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› Apply theme · Esc to Cancel · restores previous theme",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "↑↓ to Preview · only the focused theme is temporary",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "No filter · four themes fit on one screen",
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
            "picker",
            "theme-selection",
            reference(7, 4, 18),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "theme-row",
            "dark",
            reference(11, 4, 18),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "theme-row",
            "light",
            reference(13, 4, 20),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "theme-row",
            "vscode-dark",
            reference(15, 4, 18),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "theme-row",
            "monokai",
            reference(17, 4, 14),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            "light",
            reference(DOCK_TOP + 1, 4, 64),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "theme-preview",
            "temporary-light",
            reference(DOCK_TOP + 2, 4, 70),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.theme-picker.apply",
            reference(DOCK_TOP + 3, 4, 48),
            "focused",
            Some(action("action.theme-picker.apply", "Apply theme", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.theme-picker.cancel",
            reference(DOCK_TOP + 3, 49, 92),
            "summary",
            Some(action("action.theme-picker.cancel", "Cancel", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "local-control",
            "action.theme-picker.preview",
            reference(DOCK_TOP + 4, 4, 24),
            "summary",
            Some(action(
                "action.theme-picker.preview",
                "Preview theme",
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
        semantic(
            screen_id,
            fixture_id,
            "filter-not-needed",
            "no-filter-four-themes",
            reference(DOCK_TOP + 5, 4, 48),
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
                "action.theme-picker.apply",
                "theme-picker",
                "chat-turn",
                "direct",
                "Apply commits the focused theme and returns to the exact conversation origin",
            ),
            transition(
                "action.theme-picker.cancel",
                "theme-picker",
                "chat-turn",
                "cancel",
                "cancel restores the previously applied palette and content state without committing preview",
            ),
            transition(
                "action.theme-picker.preview",
                "theme-picker",
                "theme-picker",
                "direct",
                "focus preview updates the temporary palette without committing the applied theme",
            ),
        ],
    ))
}

fn render_mode_switch(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let gate_origin = is_gate_fixture(fixture_id);
    let skills_access = fixture_id.contains("skills");
    if gate_origin {
        canvas.write(3, 4, subject_title_for_origin(FLOW3_CURRENT_ORIGIN), TextStyle::muted())?;
        canvas.write(4, 4, FLOW3_SETUP_TASK, TextStyle::bright())?;
    } else {
        canvas.write(
            4,
            4,
            "You · Continue the paused conversation.",
            TextStyle::bright(),
        )?;
    }
    if skills_access {
        canvas.write(7, 4, "Review a capability change", TextStyle::bright())?;
        canvas.write(9, 4, "Requested: Research the web", TextStyle::text())?;
        canvas.write(
            11,
            4,
            "This skill can search public web content during the session.",
            TextStyle::text(),
        )?;
        canvas.write(
            13,
            4,
            "Scope: this project · until you turn the skill off.",
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
            "Pending · current skill set remains active",
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
            "› Apply access change · Esc to Keep current skills",
            TextStyle::accent(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 4,
            "One confirmation expands skill access; Cancel changes nothing.",
            TextStyle::muted(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "Return to session skills to choose another set.",
            TextStyle::muted(),
        )?;
    } else {
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
            "› Apply access change · Esc to Keep Careful edits",
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
    }

    let cancel_label = if skills_access {
        "Keep current skills"
    } else {
        "Keep Careful edits"
    };
    let cancel_target = if skills_access {
        "skills-picker"
    } else {
        "agent-picker"
    };
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
            if skills_access {
                "research-the-web"
            } else {
                "autonomous-changes"
            },
            reference(9, 4, 38),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "capability-scope",
            if skills_access {
                "project-web-access"
            } else {
                "project-file-access"
            },
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
            if skills_access {
                "current-skills-remain-active"
            } else {
                "careful-edits-remains-active"
            },
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
                cancel_label,
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
                if skills_access {
                    "confirmation applies the disclosed skill access and returns to the paused conversation"
                } else {
                    "confirmation applies the one work_style value and returns to the paused conversation"
                },
            ),
            transition(
                "action.mode-switch.cancel",
                "mode-switch",
                cancel_target,
                "cancel",
                if skills_access {
                    "cancel leaves the prior skill set and access unchanged, then returns to the skills picker"
                } else if gate_origin {
                    "cancel leaves Careful edits and its current access active, then returns to the work-style choice while preserving the current conversation origin"
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
        "› Choose Quick implementation · same access",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "  Esc to Return to work",
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
