//! Theme galleries, session status detail, and empty conversation origin.
//!
//! - `theme-gallery-dark` / `theme-gallery-light`: one shared temporary-detail
//!   gallery layout; only palette rendering differs.
//! - `status`: focused current configuration and actionable issues.
//! - `empty`: conversation shell with one dominant start and subordinate
//!   configuration/resume detours that return without creating a transcript.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, ModelChoiceEvidence,
        RenderRef, SemanticCandidate, SemanticEvent, TransitionTrace, WorkStyleEvidence,
    },
    chrome::{
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine, SurfaceLayout},
        tokens::{ColorToken, TextStyle, Theme},
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const WORK_STYLE: &str = "Careful edits";
const ACCESS_SCOPE: &str = "asks before editing project files";

const FLOW3_ORIGIN: &str = "Work configuration · theme choice";
const FLOW3_TASK: &str = "Preview the theme swatches before applying.";
const EMPTY_ORIGIN: &str = "Start · empty conversation";
const EMPTY_TASK: &str = "Start a new conversation";
const STATUS_ORIGIN: &str = "Daily work · current conversation";
const STATUS_TASK: &str =
    "Find where the server applies the rate limit and explain the safe path.";
const STATUS_DRAFT: &str = "Also list the tests that should cover the search.";

const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;

/// Approved theme preview rows shared by dark and light galleries (content parity).
#[derive(Clone, Copy)]
struct ThemePreview {
    id: &'static str,
    name: &'static str,
    descriptor: &'static str,
    tag: &'static str,
    /// Five exact swatch RGB triples (canvas, text, accent, ok, bad-family).
    swatches: [(u8, u8, u8); 5],
}

const THEME_PREVIEWS: [ThemePreview; 5] = [
    ThemePreview {
        id: "dark",
        name: "Dark",
        descriptor: "cortex default",
        tag: "warm",
        swatches: [
            (0x10, 0x10, 0x11),
            (0xDD, 0xDA, 0xD4),
            (0xD9, 0x82, 0x5F),
            (0x79, 0xA8, 0x89),
            (0xC6, 0x6F, 0x6F),
        ],
    },
    ThemePreview {
        id: "light",
        name: "Light",
        descriptor: "daytime",
        tag: "cool",
        swatches: [
            (0xFA, 0xF7, 0xF2),
            (0x33, 0x38, 0x3F),
            (0xC0, 0x6B, 0x44),
            (0x4F, 0x79, 0x42),
            (0xB5, 0x4A, 0x55),
        ],
    },
    ThemePreview {
        id: "vscode-dark",
        name: "VS Code dark",
        descriptor: "familiar",
        tag: "neutral",
        swatches: [
            (0x1E, 0x1E, 0x1E),
            (0xD4, 0xD4, 0xD4),
            (0x4F, 0xC1, 0xFF),
            (0x89, 0xBE, 0x74),
            (0xEF, 0x53, 0x50),
        ],
    },
    ThemePreview {
        id: "monokai",
        name: "Monokai",
        descriptor: "vivid",
        tag: "high-contrast",
        swatches: [
            (0x27, 0x28, 0x22),
            (0xF8, 0xF8, 0xF2),
            (0xFD, 0x97, 0x1F),
            (0xA6, 0xE2, 0x2E),
            (0xF9, 0x26, 0x72),
        ],
    },
    ThemePreview {
        id: "ocean",
        name: "Ocean",
        descriptor: "cool blues",
        tag: "calm",
        swatches: [
            (0x0F, 0x22, 0x34),
            (0xD6, 0xE1, 0xEC),
            (0x4A, 0xA4, 0xD1),
            (0x6C, 0xBE, 0x8F),
            (0xD8, 0x6A, 0x78),
        ],
    },
];

/// Semantic token stand-ins for TXT (color-independent labels); PNG uses exact RGB.
const SWATCH_TOKEN_FALLBACK: [ColorToken; 5] = [
    ColorToken::Border,
    ColorToken::Text,
    ColorToken::Accent,
    ColorToken::Ok,
    ColorToken::Bad,
];

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    match screen_id {
        "theme-gallery-dark" => render_theme_gallery(Theme::Dark, fixture_id),
        "theme-gallery-light" => render_theme_gallery(Theme::Light, fixture_id),
        "status" => render_status(fixture_id),
        "empty" => render_empty(fixture_id),
        _ => Err(format!("unknown theme/status/empty screen: {screen_id}")),
    }
}



fn render_theme_gallery(scheme: Theme, fixture_id: &str) -> Result<AuditDocument, String> {
    let screen_id = match scheme {
        Theme::Dark => "theme-gallery-dark",
        Theme::Light => "theme-gallery-light",
    };
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::TemporaryDetailView, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::TemporaryDetailView).validate()?;

    let selected = selected_theme_index(fixture_id);
    let mut canvas = Canvas::new_with_theme(scheme);
    let (candidates, events, transitions) =
        paint_theme_gallery(&mut canvas, screen_id, fixture_id, selected)?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: Some(WorkStyleEvidence {
            applied: WORK_STYLE.to_owned(),
            pending: None,
            access_scope: ACCESS_SCOPE.to_owned(),
        }),
        model_choice: None,
        conversation: Some(ConversationIdentity {
            conversation_id: "conversation.configuration".to_owned(),
            transcript_id: "transcript.configuration".to_owned(),
            session_id: "session.configuration.1".to_owned(),
            turn_id: "turn.configuration.theme.1".to_owned(),
            user_task: FLOW3_TASK.to_owned(),
            visible_origin: FLOW3_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: Some("activity.theme-picker".to_owned()),
            later_draft: Some("Continue the paused conversation.".to_owned()),
        }),
    })
}

fn selected_theme_index(fixture_id: &str) -> usize {
    // Prefer explicit subject markers. Do not key off screen names like
    // `theme-gallery-light`, which must share the same selected row as dark.
    if fixture_id.contains("select-light") || fixture_id.ends_with(".light-selected") {
        1
    } else if fixture_id.contains("vscode") {
        2
    } else if fixture_id.contains("monokai") {
        3
    } else if fixture_id.contains("ocean") {
        4
    } else {
        0
    }
}

fn paint_theme_gallery(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    selected: usize,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, FLOW3_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, FLOW3_TASK, TextStyle::bright())?;
    canvas.write(
        5,
        4,
        "Theme gallery · temporary detail",
        TextStyle::bright(),
    )?;
    canvas.write(
        6,
        4,
        "From theme picker · pending selection and origin stay intact on return",
        TextStyle::muted(),
    )?;
    canvas.write(
        8,
        4,
        "Five brand themes · swatches only use multi-hue samples",
        TextStyle::muted(),
    )?;

    let mut candidates = Vec::new();
    let mut events = Vec::new();
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "detail-origin",
        "theme-picker",
        reference(6, 4, 74),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "gallery",
        "theme-comparison",
        reference(5, 4, 40),
        "summary",
        None,
    );

    for (index, theme) in THEME_PREVIEWS.iter().enumerate() {
        let row = 10 + (index as u16) * 4;
        let active = index == selected;
        let marker = if active { "›" } else { " " };
        let style = if active {
            TextStyle::accent()
        } else {
            TextStyle::text()
        };
        let active_label = if active { "  active" } else { "" };
        let head = format!(
            "{marker} {:<14} {:<16} {:<14}",
            theme.name, theme.descriptor, theme.tag
        );
        canvas.write(row, 4, &format!("{head}["), style)?;
        // Swatch many cells at a fixed column; label selection in text, not color alone.
        let swatch_col = 4 + head.chars().count() as u16 + 1;
        for (s_index, rgb) in theme.swatches.iter().enumerate() {
            let col = swatch_col + (s_index as u16) * 4;
            let token = SWATCH_TOKEN_FALLBACK[s_index];
            canvas.write_absolute(
                row,
                col,
                "███",
                TextStyle {
                    foreground: token,
                    background: Some(token),
                    bold: false,
                },
                Some(*rgb),
                Some(*rgb),
            )?;
            if s_index + 1 < theme.swatches.len() {
                canvas.write(row, col + 3, " ", TextStyle::muted())?;
            }
        }
        let after_swatches = swatch_col + 5 * 4 - 1;
        canvas.write(row, after_swatches, &format!("]{active_label}"), style)?;
        if active {
            canvas.write(
                row + 1,
                8,
                "Selected · labels stay readable; colors are sample only",
                TextStyle::muted(),
            )?;
        }
        if index + 1 != THEME_PREVIEWS.len() {
            canvas.write(
                row + 2,
                4,
                &"─".repeat(112),
                TextStyle::muted(),
            )?;
        }
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "theme-row",
            theme.id,
            reference(row, 4, 50),
            if active { "focused" } else { "summary" },
            None,
        );
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "theme-swatch",
            theme.id,
            reference(row, swatch_col, swatch_col + 19),
            "summary",
            None,
        );
    }

    let selected_name = THEME_PREVIEWS[selected].name;
    canvas.write(
        32,
        4,
        &format!("Selected · {selected_name} · preview samples stay temporary until Apply"),
        TextStyle::bright(),
    )?;
    canvas.write(
        34,
        4,
        "› [esc] Return to theme picker · keeps pending selection and origin",
        TextStyle::accent(),
    )?;
    canvas.write(
        35,
        4,
        "[↑↓] Choose theme · apply stays on the picker, not this gallery",
        TextStyle::muted(),
    )?;
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "selected-theme",
        THEME_PREVIEWS[selected].id,
        reference(32, 4, 70),
        "focused",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.theme-gallery.return",
        reference(34, 4, 50),
        "focused",
        Some(action(
            "action.theme-gallery.return",
            "Return to theme picker",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );

    let transitions = vec![transition(
        "action.theme-gallery.return",
        screen_id,
        "theme-picker",
        "cancel",
        "gallery return restores the picker with pending theme, focus, and conversation origin",
    )];
    Ok((candidates, events, transitions))
}

fn render_status(fixture_id: &str) -> Result<AuditDocument, String> {
    let screen_id = "status";
    let advanced = fixture_id.contains("advanced");
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::TemporaryDetailView, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::TemporaryDetailView).validate()?;

    let mut canvas = Canvas::new();
    canvas.write(2, 4, STATUS_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, STATUS_TASK, TextStyle::bright())?;
    canvas.write(
        5,
        4,
        "Session status · temporary detail",
        TextStyle::bright(),
    )?;
    canvas.write(
        6,
        4,
        "From the rate-limit conversation · answers what is active and what needs attention",
        TextStyle::muted(),
    )?;

    canvas.write(8, 4, "Active now", TextStyle::accent())?;
    canvas.write(
        10,
        4,
        &format!("Model · {MODEL}"),
        TextStyle::bright(),
    )?;
    canvas.write(
        11,
        4,
        "Everyday coding · balanced speed for daily work",
        TextStyle::muted(),
    )?;
    canvas.write(
        13,
        4,
        &format!("How Code should work · {WORK_STYLE}"),
        TextStyle::bright(),
    )?;
    canvas.write(14, 4, ACCESS_SCOPE, TextStyle::muted())?;
    canvas.write(
        16,
        4,
        "Project · ~/project · main · branch is clean",
        TextStyle::text(),
    )?;
    canvas.write(
        17,
        4,
        "Session · Retry policy review · 17 turns today",
        TextStyle::muted(),
    )?;

    canvas.write(19, 4, "Needs attention", TextStyle::accent())?;
    canvas.write(
        21,
        4,
        "● Docs research · Needs sign-in before the next lookup",
        TextStyle::text(),
    )?;
    canvas.write(
        22,
        4,
        "  Open connection to continue that capability",
        TextStyle::muted(),
    )?;
    canvas.write(
        24,
        4,
        "○ Local tools · Ready · no blockers",
        TextStyle::muted(),
    )?;

    if advanced {
        canvas.write(26, 4, "Advanced detail", TextStyle::bright())?;
        canvas.write(
            27,
            4,
            "Provider · Anthropic · connected for signed-in work",
            TextStyle::muted(),
        )?;
        canvas.write(
            28,
            4,
            "Context budget and cost stay out of the statusline; open details only here",
            TextStyle::muted(),
        )?;
    } else {
        canvas.write(
            26,
            4,
            "Advanced raw metadata stays hidden until you open Details",
            TextStyle::muted(),
        )?;
    }

    canvas.write(
        32,
        4,
        if advanced {
            "› [esc] Return to conversation · Draft and turn stay exactly where you left them"
        } else {
            "› [d] Details · [esc] Return · keeps draft, selection, and origin"
        },
        TextStyle::accent(),
    )?;
    canvas.write(
        33,
        4,
        "This view answers configuration questions without duplicating the statusline",
        TextStyle::muted(),
    )?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    let mut candidates = Vec::new();
    let mut events = Vec::new();
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "detail-origin",
        "conversation.daily-work",
        reference(6, 4, 80),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "model-summary",
        "claude-opus-4-8",
        reference(10, 4, 40),
        "focused",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "work-style-summary",
        "careful-edits",
        reference(13, 4, 50),
        "focused",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "project-summary",
        "project-main",
        reference(16, 4, 50),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "issue-row",
        "docs-research-auth",
        reference(21, 4, 70),
        "focused",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        if advanced {
            "advanced-detail"
        } else {
            "advanced-hidden"
        },
        if advanced {
            "provider-metadata"
        } else {
            "disclosure-closed"
        },
        reference(26, 4, 70),
        if advanced { "focused" } else { "summary" },
        None,
    );
    if advanced {
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "primary-action",
            "action.status.return",
            reference(32, 4, 70),
            "focused",
            Some(action("action.status.return", "Return", false)),
        );
    } else {
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "primary-action",
            "action.status.details",
            reference(32, 4, 24),
            "focused",
            Some(action("action.status.details", "Details", false)),
        );
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "return-action",
            "action.status.return",
            reference(32, 30, 70),
            "summary",
            Some(action("action.status.return", "Return", false)),
        );
    }
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );

    let transitions = if advanced {
        vec![transition(
            "action.status.return",
            "status",
            "chat-turn",
            "cancel",
            "return restores the paused conversation draft and origin",
        )]
    } else {
        vec![
            transition(
                "action.status.details",
                "status",
                "status",
                "direct",
                "details discloses advanced metadata without leaving the temporary view",
            ),
            transition(
                "action.status.return",
                "status",
                "chat-turn",
                "cancel",
                "return restores the paused conversation draft and origin",
            ),
        ]
    };

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: Some(WorkStyleEvidence {
            applied: WORK_STYLE.to_owned(),
            pending: None,
            access_scope: ACCESS_SCOPE.to_owned(),
        }),
        model_choice: Some(ModelChoiceEvidence {
            applied: MODEL.to_owned(),
            pending: None,
            origin: STATUS_ORIGIN.to_owned(),
            pending_action: None,
            return_policy: "return-to-chat-turn".to_owned(),
        }),
        conversation: Some(ConversationIdentity {
            conversation_id: "conversation.daily-work".to_owned(),
            transcript_id: "transcript.daily-work".to_owned(),
            session_id: "session.daily-work.1".to_owned(),
            turn_id: "turn.daily-work.18".to_owned(),
            user_task: STATUS_TASK.to_owned(),
            visible_origin: STATUS_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: Some("activity.rate-limit-path".to_owned()),
            later_draft: Some(STATUS_DRAFT.to_owned()),
        }),
    })
}

fn render_empty(fixture_id: &str) -> Result<AuditDocument, String> {
    // Onboarding destinations remain owned by audit.rs specialized path via
    // fixture culture; production empty quaternion here is the States default.
    if fixture_id.contains("onboarding-complete") || fixture_id.contains("onboarding-skip") {
        return Err(
            "onboarding empty fixtures must route through audit empty_after_onboarding_audit"
                .to_owned(),
        );
    }

    let screen_id = "empty";
    let after_apply = fixture_id.contains("after-apply");
    let model = if after_apply && fixture_id.contains("sonnet") {
        "Claude Sonnet 4.5"
    } else {
        MODEL
    };
    let work_style = if after_apply && fixture_id.contains("auto") {
        "Independent work"
    } else {
        WORK_STYLE
    };
    let draft = if fixture_id.contains("draft") {
        "Draft a retry plan"
    } else {
        ""
    };

    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, model)?;
    ScreenComposition::new(screen_id, Surface::ConversationShell, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::ConversationShell).validate()?;

    let mut canvas = Canvas::new();
    // Restrained entry brand (optional ACCENT ASCII is skipped on empty to
    // keep the composer dominant; origin chip only).
    canvas.write(2, 4, EMPTY_ORIGIN, TextStyle::muted())?;
    canvas.write(3, 4, EMPTY_TASK, TextStyle::bright())?;
    canvas.write(
        6,
        4,
        "No messages yet. Type below to begin, or resume earlier work.",
        TextStyle::text(),
    )?;
    canvas.write(
        8,
        4,
        &format!("Ready · {model} · How Code should work · {work_style}"),
        TextStyle::muted(),
    )?;
    canvas.write(
        10,
        4,
        "Secondary · /resume previous sessions · /model · /theme · change work style",
        TextStyle::muted(),
    )?;
    canvas.write(
        11,
        4,
        "Those configuration detours return here without inventing a transcript",
        TextStyle::muted(),
    )?;

    canvas.write(
        COMPOSER_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    let composer_line = if draft.is_empty() {
        "│ › Ask Code to start a task".to_owned()
    } else {
        format!("│ › {draft}")
    };
    canvas.write(
        COMPOSER_TOP + 1,
        2,
        &composer_line,
        TextStyle::bright(),
    )?;
    canvas.write(
        COMPOSER_TOP + 2,
        2,
        "│",
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM - 1,
        4,
        "This is the primary start · subordinate resume and settings stay secondary",
        TextStyle::muted(),
    )?;
    canvas.write(
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    let mut candidates = Vec::new();
    let mut events = Vec::new();
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "surface",
        "conversation-shell",
        reference(3, 4, 30),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "empty-guidance",
        "no-messages",
        reference(6, 4, 70),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "secondary-resume",
        "resume-sessions",
        reference(10, 4, 30),
        "summary",
        Some(action(
            "action.empty.open-session-picker",
            "Resume sessions",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "secondary-config",
        "configuration-entry",
        reference(10, 34, 70),
        "summary",
        Some(action(
            "action.empty.open-model-picker",
            "Change model",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.empty.start-request",
        reference(COMPOSER_TOP + 1, 4, 40),
        "focused",
        Some(action(
            "action.empty.start-request",
            "Start from composer",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );

    let transitions = vec![
        transition(
            "action.empty.start-request",
            "empty",
            "chat-turn",
            "direct",
            "composer start begins the first request from the empty shell",
        ),
        transition(
            "action.empty.open-model-picker",
            "empty",
            "model-picker",
            "direct",
            "configuration from empty records empty as origin",
        ),
        transition(
            "action.empty.open-session-picker",
            "empty",
            "session-picker",
            "direct",
            "resume from empty opens the session list with empty origin",
        ),
    ];

    Ok(AuditDocument {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        styled_cells: canvas.into_cells(),
        candidates,
        events,
        transitions,
        work_style: Some(WorkStyleEvidence {
            applied: work_style.to_owned(),
            pending: None,
            access_scope: ACCESS_SCOPE.to_owned(),
        }),
        model_choice: Some(ModelChoiceEvidence {
            applied: model.to_owned(),
            pending: None,
            origin: EMPTY_ORIGIN.to_owned(),
            pending_action: None,
            return_policy: "stay-on-empty".to_owned(),
        }),
        conversation: Some(ConversationIdentity {
            conversation_id: "conversation.first-run".to_owned(),
            transcript_id: "transcript.first-run".to_owned(),
            session_id: "session.first-run.1".to_owned(),
            turn_id: "turn.first-run.0".to_owned(),
            user_task: EMPTY_TASK.to_owned(),
            visible_origin: EMPTY_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: None,
            later_draft: if draft.is_empty() {
                Some(String::new())
            } else {
                Some(draft.to_owned())
            },
        }),
    })
}

fn action(action_id: &str, label: &str, automatic: bool) -> ActionEvidence {
    ActionEvidence {
        action_id: action_id.to_owned(),
        label: label.to_owned(),
        enabled: true,
        automatic,
    }
}

fn transition(
    action_id: &str,
    from: &str,
    to: &str,
    kind: &str,
    path: &str,
) -> TransitionTrace {
    TransitionTrace {
        action_id: action_id.to_owned(),
        from_screen_id: from.to_owned(),
        to_screen_id: to.to_owned(),
        kind: kind.to_owned(),
        path: path.to_owned(),
    }
}

fn reference(row: u16, start: u16, end: u16) -> RenderRef {
    RenderRef { row, start, end }
}

#[allow(clippy::too_many_arguments)]
fn push_pair(
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
    screen_id: &str,
    fixture_id: &str,
    family: &str,
    subject_id: &str,
    render_ref: RenderRef,
    detail_level: &str,
    action: Option<ActionEvidence>,
) {
    let identity = Identity {
        screen_id: screen_id.to_owned(),
        fixture_id: fixture_id.to_owned(),
        candidate_id: format!("{family}.{subject_id}"),
        event_id: format!("event.{family}.{subject_id}"),
        family: family.to_owned(),
        subject_id: subject_id.to_owned(),
        render_refs: vec![render_ref],
        detail_level: detail_level.to_owned(),
    };
    candidates.push(SemanticCandidate {
        identity: identity.clone(),
    });
    events.push(SemanticEvent { identity, action });
}
