//! Flow 4 contextual MCP sidebar (`sidebar-mcp`).
//!
//! Keeps the conversation shell readable while summarizing MCP attention in a
//! narrow inspector. Full catalogue, raw tools, and diagnostics stay under the
//! temporary MCP detail view (VAL-V3-F4-041..043).

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{
            subject_title_for_origin, Component, Inspector, InspectorTab, PromptJewel,
        },
        layout::{outer_content_x, CANVAS_ROWS, ScreenComposition, StatusLine, SurfaceLayout},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const WORK_MODE: &str = "Careful edits";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;
const DAILY_WORK_ORIGIN: &str = "Daily work · current conversation";
const DAILY_WORK_TASK: &str =
    "Find where the server applies the rate limit and explain the safe path.";
const DRAFT: &str = "Also list the tests that should cover the search.";
const TURN_ID: &str = "turn.daily-work.mcp-sidebar.18";
const INSPECTOR_LEFT: u16 = 81;
const INSPECTOR_WIDTH: u16 = 37;
const TRANSCRIPT_WIDTH: usize = 74;
const TOTAL_CAPABILITIES: u32 = 7;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidebarFixture {
    Attention,
    AllReady,
}

impl SidebarFixture {
    fn from_id(fixture_id: &str) -> Self {
        let lower = fixture_id.to_ascii_lowercase();
        if lower.contains("all-ready") || lower.contains("all_ready") || lower.contains("healthy")
        {
            Self::AllReady
        } else {
            Self::Attention
        }
    }
}

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    if screen_id != "sidebar-mcp" {
        return Err(format!("unknown MCP sidebar screen: {screen_id}"));
    }
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ContextualInspector, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::ContextualInspector).validate()?;

    let mut canvas = Canvas::new();
    paint_shared_conversation(&mut canvas)?;
    let (candidates, events, mut transitions) =
        render_sidebar_body(&mut canvas, screen_id, fixture_id)?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    transitions.extend([
        transition(
            "action.sidebar-mcp.open-mcp",
            screen_id,
            "mcp",
            "direct",
            "Open MCP opens the temporary detail view without discarding the draft",
        ),
        transition(
            "action.inspector.close",
            screen_id,
            "chat-turn",
            "direct",
            "close restores the full-width origin conversation, draft, and scroll anchor",
        ),
        transition(
            "action.inspector.back",
            screen_id,
            "chat-turn",
            "direct",
            "back restores the full-width origin conversation without entering home",
        ),
        transition(
            "action.inspector.cancel",
            screen_id,
            "chat-turn",
            "direct",
            "cancel restores the full-width origin conversation and kept draft",
        ),
        // Return-from-MCP is authored on the MCP destination surfaces; this
        // identity remains actionable here for scenario bijection when the
        // after state is the restored sidebar (case audits by after screen).
        transition(
            "action.mcp.return-to-sidebar",
            screen_id,
            screen_id,
            "direct",
            "return destination is the same sidebar with selection, draft, and scroll restored",
        ),
    ]);

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
            conversation_id: "conversation.daily-work".to_owned(),
            transcript_id: "transcript.daily-work".to_owned(),
            session_id: "session.daily-work.1".to_owned(),
            turn_id: TURN_ID.to_owned(),
            user_task: DAILY_WORK_TASK.to_owned(),
            visible_origin: DAILY_WORK_ORIGIN.to_owned(),
            submitted_prompt: None,
            paused_activity_id: Some("activity.rate-limit-path".to_owned()),
            later_draft: Some(DRAFT.to_owned()),
        }),
    })
}

fn paint_shared_conversation(canvas: &mut Canvas) -> Result<(), String> {
    let write_transcript = |canvas: &mut Canvas, row: u16, text: &str, style: TextStyle| {
        let clipped = truncate_to(text, TRANSCRIPT_WIDTH);
        canvas.write(row, 4, &clipped, style)
    };

    write_transcript(canvas, 2, subject_title_for_origin(DAILY_WORK_ORIGIN), TextStyle::muted())?;
    write_transcript(canvas, 3, DAILY_WORK_TASK, TextStyle::bright())?;
    write_transcript(
        canvas,
        5,
        "You · Find where the server applies the rate limit and explain the safe path.",
        TextStyle::bright(),
    )?;
    write_transcript(canvas, 8, "Code", TextStyle::bright())?;
    write_transcript(
        canvas,
        10,
        "The shared request guard applies the limit before route work begins.",
        TextStyle::text(),
    )?;
    write_transcript(
        canvas,
        11,
        "→ src/auth/middleware.rs owns the rate-limit decision.",
        TextStyle::text(),
    )?;
    write_transcript(
        canvas,
        12,
        "→ The login and API routes reuse the same safety rule.",
        TextStyle::text(),
    )?;
    write_transcript(
        canvas,
        14,
        "● Reading src/auth/middleware.rs · checked the login path",
        TextStyle::muted(),
    )?;

    composer_with_draft(canvas, DRAFT)?;
    Ok(())
}

fn composer_with_draft(canvas: &mut Canvas, draft: &str) -> Result<(), String> {
    // Cortex PromptJewel with muted model · work mode info line.
    let jewel = PromptJewel::new(draft, MODEL, WORK_MODE)?.focused();
    let x = outer_content_x();
    for (index, line) in jewel.lines().into_iter().enumerate() {
        canvas.write(COMPOSER_TOP + index as u16, x, &line.text, line.style)?;
    }
    let _ = COMPOSER_BOTTOM;
    Ok(())
}

fn render_sidebar_body(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let fixture = SidebarFixture::from_id(fixture_id);
    let tabs = vec![InspectorTab::new("MCP")];
    let inspector = Inspector::new("MCP status", DAILY_WORK_ORIGIN, tabs, 0)?;
    write_component(canvas, 1, INSPECTOR_LEFT, &inspector)?;

    for row in 1..31 {
        canvas.write(row, INSPECTOR_LEFT - 2, "│", TextStyle::muted())?;
    }

    let content_top = 9u16;
    let mut candidates = Vec::new();
    let mut events = Vec::new();

    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-tab",
        "tab-mcp",
        reference(2, INSPECTOR_LEFT, INSPECTOR_LEFT + 12),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "return-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        Some(action(
            "action.inspector.close",
            "Close inspector",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "back-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        Some(action("action.inspector.back", "Back", false)),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "cancel-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        Some(action("action.inspector.cancel", "Cancel", false)),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "return-from-mcp-detail",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        Some(action(
            "action.mcp.return-to-sidebar",
            "Return to sidebar",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "transcript",
        "active-turn",
        reference(5, 4, 4 + TRANSCRIPT_WIDTH as u16),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "queued-input",
        "composer-draft",
        reference(COMPOSER_TOP + 1, 4, 4 + DRAFT.len() as u16 + 4),
        "summary",
        None,
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

    match fixture {
        SidebarFixture::Attention => {
            paint_attention(
                canvas,
                content_top,
                screen_id,
                fixture_id,
                &mut candidates,
                &mut events,
            )?;
        }
        SidebarFixture::AllReady => {
            paint_all_ready(
                canvas,
                content_top,
                screen_id,
                fixture_id,
                &mut candidates,
                &mut events,
            )?;
        }
    }

    Ok((candidates, events, Vec::new()))
}

fn paint_attention(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    let total = format!("{TOTAL_CAPABILITIES} capabilities · 1 needs attention");
    canvas.write(top, INSPECTOR_LEFT, "MCP summary", TextStyle::bright())?;
    canvas.write(top + 1, INSPECTOR_LEFT, &total, TextStyle::muted())?;
    canvas.write(top + 3, INSPECTOR_LEFT, "Needs attention", TextStyle::bright())?;
    canvas.write(top + 4, INSPECTOR_LEFT, "✗ Deploy hooks", TextStyle::text())?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "Blocked by policy",
        TextStyle::bright(),
    )?;
    canvas.write(
        top + 6,
        INSPECTOR_LEFT,
        "Workspace policy blocks remote",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "deploy actions.",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "Healthy details stay collapsed.",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 11,
        INSPECTOR_LEFT,
        "› Open MCP",
        TextStyle::accent(),
    )?;
    canvas.write(
        top + 12,
        INSPECTOR_LEFT,
        "  Manage remaining statuses",
        TextStyle::muted(),
    )?;

    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "mcp-summary",
        "compact-total",
        reference(top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
        "summary",
        None,
    );
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "mcp-attention",
        "deploy-hooks",
        reference(top + 4, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
        "focused",
        None,
    );
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.sidebar-mcp.open-mcp",
        reference(top + 11, INSPECTOR_LEFT, INSPECTOR_LEFT + 22),
        "focused",
        Some(action(
            "action.sidebar-mcp.open-mcp",
            "Open MCP",
            false,
        )),
    );
    Ok(())
}

fn paint_all_ready(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    let total = format!("{TOTAL_CAPABILITIES} capabilities · All ready");
    canvas.write(top, INSPECTOR_LEFT, "MCP summary", TextStyle::bright())?;
    canvas.write(top + 1, INSPECTOR_LEFT, &total, TextStyle::muted())?;
    canvas.write(top + 3, INSPECTOR_LEFT, "All ready", TextStyle::bright())?;
    canvas.write(
        top + 4,
        INSPECTOR_LEFT,
        "Every capability is ready for",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "this conversation.",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "Healthy details stay collapsed.",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "› Open MCP",
        TextStyle::accent(),
    )?;
    canvas.write(
        top + 10,
        INSPECTOR_LEFT,
        "  Inspect connections",
        TextStyle::muted(),
    )?;

    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "mcp-summary",
        "compact-total",
        reference(top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
        "summary",
        None,
    );
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "mcp-attention",
        "all-ready",
        reference(top + 3, INSPECTOR_LEFT, INSPECTOR_LEFT + 20),
        "summary",
        None,
    );
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.sidebar-mcp.open-mcp",
        reference(top + 9, INSPECTOR_LEFT, INSPECTOR_LEFT + 22),
        "focused",
        Some(action(
            "action.sidebar-mcp.open-mcp",
            "Open MCP",
            false,
        )),
    );
    Ok(())
}

fn write_component(
    canvas: &mut Canvas,
    first_row: u16,
    column: u16,
    component: &impl Component,
) -> Result<(), String> {
    for (index, line) in component.lines().into_iter().enumerate() {
        let text = truncate_to(line.text.as_str(), INSPECTOR_WIDTH as usize);
        canvas.write(first_row + index as u16, column, &text, line.style)?;
    }
    Ok(())
}

fn truncate_to(text: &str, max: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max {
        text.to_owned()
    } else {
        chars.into_iter().take(max).collect()
    }
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
        candidate_id: format!("candidate.{family}.{subject_id}"),
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
