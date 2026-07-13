//! Flow 1 contextual inspectors.
//!
//! Canonical surface: transcript remains visible on the left, a narrow
//! task-specific inspector sits on the right, and the composer stays available
//! at the bottom. Tabs stay contextual; close/back/cancel restore the full-
//! width originating conversation.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{Component, Inspector, InspectorTab},
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine, SurfaceLayout},
        tokens::TextStyle,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;
const DAILY_WORK_ORIGIN: &str = "Daily work · current conversation";
const DAILY_WORK_TASK: &str =
    "Find where the server applies the rate limit and explain the safe path.";
const DRAFT: &str = "Also list the tests that should cover the search.";
const TURN_ID: &str = "turn.daily-work.inspector.18";
const INSPECTOR_LEFT: u16 = 81;
const INSPECTOR_WIDTH: u16 = 37;
const TRANSCRIPT_WIDTH: usize = 74;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

const TAB_ORDER: &[&str] = &["Context", "Files", "LSP", "Todo", "Subs"];
const TAB_SCREENS: &[&str] = &[
    "sidebar-context",
    "sidebar-files",
    "sidebar-lsp",
    "sidebar-todo",
    "sidebar-subagents",
];

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    if !TAB_SCREENS.contains(&screen_id) {
        return Err(format!("unknown contextual inspector: {screen_id}"));
    }
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ContextualInspector, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::ContextualInspector).validate()?;

    let mut canvas = Canvas::new();
    paint_shared_conversation(&mut canvas)?;
    let (candidates, events, mut transitions) =
        render_inspector_body(&mut canvas, screen_id, fixture_id)?;
    canvas.write(CANVAS_ROWS - 1, 2, statusline.text(), statusline.style())?;

    // Shared close/back/cancel semantics for every inspector panel.
    transitions.extend([
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
    ]);

    // Tab edges avoid inventing destinations beyond the five Flow 1 inspectors.
    for (index, tab_screen) in TAB_SCREENS.iter().enumerate() {
        if *tab_screen == screen_id {
            continue;
        }
        transitions.push(transition(
            &format!("action.inspector.tab.{}", TAB_ORDER[index].to_lowercase()),
            screen_id,
            tab_screen,
            "direct",
            "tab updates only the narrow inspector while the shared turn, draft, and scroll stay put",
        ));
    }

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
    // Transcript column is 76 cells starting at x=2; stop before the inspector divider.
    let write_transcript = |canvas: &mut Canvas, row: u16, text: &str, style: TextStyle| {
        let clipped = truncate_to(text, TRANSCRIPT_WIDTH);
        canvas.write(row, 4, &clipped, style)
    };

    write_transcript(canvas, 2, DAILY_WORK_ORIGIN, TextStyle::muted())?;
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

    // Full-width composer so the input remains recognizable under the inspector.
    composer_with_draft(canvas, DRAFT)?;
    Ok(())
}

fn composer_with_draft(canvas: &mut Canvas, draft: &str) -> Result<(), String> {
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
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    Ok(())
}

fn selected_tab(screen_id: &str) -> usize {
    TAB_SCREENS
        .iter()
        .position(|id| *id == screen_id)
        .expect("screen is one of the Flow 1 inspectors")
}

fn render_inspector_body(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let selected = selected_tab(screen_id);
    let tabs = TAB_ORDER
        .iter()
        .map(|label| InspectorTab::new(label))
        .collect::<Vec<_>>();
    let inspector = Inspector::new("Inspector", DAILY_WORK_ORIGIN, tabs, selected)?;
    write_component(canvas, 1, INSPECTOR_LEFT, &inspector)?;

    // Divider between transcript and inspector.
    for row in 1..31 {
        canvas.write(row, INSPECTOR_LEFT - 2, "│", TextStyle::muted())?;
    }

    // Content under the shared header (title + 5 tabs + return ≈ 7 lines).
    let content_top = 9u16;
    let mut candidates = Vec::new();
    let mut events = Vec::new();

    // Shared chrome semantics: active tab, one close owner, and alternate close keys.
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-tab",
        &format!("tab-{}", TAB_ORDER[selected].to_lowercase()),
        reference(2 + selected as u16, INSPECTOR_LEFT, INSPECTOR_LEFT + 12),
        "summary",
        None,
    );
    for (index, label) in TAB_ORDER.iter().enumerate() {
        if index == selected {
            continue;
        }
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "inspector-tab-action",
            &format!("open-{}", label.to_lowercase()),
            reference(2 + index as u16, INSPECTOR_LEFT, INSPECTOR_LEFT + 12),
            "summary",
            Some(action(
                &format!("action.inspector.tab.{}", label.to_lowercase()),
                &format!("Open {label}"),
                false,
            )),
        );
    }
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "return-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        Some(action("action.inspector.close", "Close inspector", false)),
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

    match screen_id {
        "sidebar-context" => paint_context(canvas, content_top, screen_id, fixture_id, &mut candidates, &mut events)?,
        "sidebar-files" => paint_files(canvas, content_top, screen_id, fixture_id, &mut candidates, &mut events)?,
        "sidebar-lsp" => paint_lsp(canvas, content_top, screen_id, fixture_id, &mut candidates, &mut events)?,
        "sidebar-todo" => paint_todo(canvas, content_top, screen_id, fixture_id, &mut candidates, &mut events)?,
        "sidebar-subagents" => {
            paint_subagents(canvas, content_top, screen_id, fixture_id, &mut candidates, &mut events)?
        }
        _ => return Err(format!("unknown contextual inspector body: {screen_id}")),
    }

    Ok((candidates, events, Vec::new()))
}

fn paint_context(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    canvas.write(top, INSPECTOR_LEFT, "Relevant context", TextStyle::bright())?;
    canvas.write(
        top + 1,
        INSPECTOR_LEFT,
        "4 files help this turn",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 3,
        INSPECTOR_LEFT,
        "middleware.rs · rate-limit rule",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 4,
        INSPECTOR_LEFT,
        "login.rs · route entry",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "retry_policy.rs · shared guard",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 6,
        INSPECTOR_LEFT,
        "tests/login_ratelimit.rs",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 8,
        INSPECTOR_LEFT,
        "Capacity · about one third used",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "space left for more files",
        TextStyle::muted(),
    )?;

    for (offset, subject) in [
        "middleware-rate-limit",
        "login-route-entry",
        "retry-policy-guard",
        "login-ratelimit-tests",
    ]
    .into_iter()
    .enumerate()
    {
        push_pair(
            candidates,
            events,
            screen_id,
            fixture_id,
            "inspector-context",
            subject,
            reference(top + 3 + offset as u16, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
            "summary",
            None,
        );
    }
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "inspector-context-summary",
        "capacity-summary",
        reference(top + 8, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
        "summary",
        None,
    );
    Ok(())
}

fn paint_files(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    canvas.write(top, INSPECTOR_LEFT, "Workspace files", TextStyle::bright())?;
    canvas.write(
        top + 1,
        INSPECTOR_LEFT,
        "3 modified · 2 unchanged",
        TextStyle::muted(),
    )?;
    canvas.write(top + 3, INSPECTOR_LEFT, "▾ src", TextStyle::text())?;
    canvas.write(top + 4, INSPECTOR_LEFT, "  ▾ auth", TextStyle::text())?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "    ● middleware.rs  +12 -4 modified",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 6,
        INSPECTOR_LEFT,
        "    ● login.rs       +7 -2 modified",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "    mod.rs           unchanged",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 8,
        INSPECTOR_LEFT,
        "  ▸ net",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 10,
        INSPECTOR_LEFT,
        "● Cargo.toml         +1 -1 modified",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 11,
        INSPECTOR_LEFT,
        "README.md            unchanged",
        TextStyle::muted(),
    )?;

    for (subject, row) in [
        ("middleware-modified", 5u16),
        ("login-modified", 6),
        ("mod-unchanged", 7),
        ("cargo-modified", 10),
        ("readme-unchanged", 11),
    ] {
        push_pair(
            candidates,
            events,
            screen_id,
            fixture_id,
            "inspector-file",
            subject,
            reference(top + row, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
            "summary",
            None,
        );
    }
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "inspector-file-summary",
        "change-counts",
        reference(top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 26),
        "summary",
        None,
    );
    Ok(())
}

fn paint_lsp(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    canvas.write(top, INSPECTOR_LEFT, "Diagnostics", TextStyle::bright())?;
    canvas.write(
        top + 1,
        INSPECTOR_LEFT,
        "1 error · 2 warnings",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 3,
        INSPECTOR_LEFT,
        "● buffer.rs · 1 error",
        TextStyle::bright(),
    )?;
    canvas.write(
        top + 4,
        INSPECTOR_LEFT,
        "  Error · line 42",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "  capacity must be greater than zero",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "● mod.rs · 2 warnings",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 8,
        INSPECTOR_LEFT,
        "  Warning · line 18",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "  unused import: Read",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 10,
        INSPECTOR_LEFT,
        "  Warning · line 90",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 11,
        INSPECTOR_LEFT,
        "  redundant clone()",
        TextStyle::muted(),
    )?;

    for (subject, row) in [
        ("error-buffer-capacity", 4u16),
        ("warning-mod-unused-import", 8),
        ("warning-mod-redundant-clone", 10),
    ] {
        push_pair(
            candidates,
            events,
            screen_id,
            fixture_id,
            "inspector-diagnostic",
            subject,
            reference(top + row, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
            "summary",
            None,
        );
    }
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "inspector-diagnostic-summary",
        "severity-totals",
        reference(top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 24),
        "summary",
        None,
    );
    Ok(())
}

fn paint_todo(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    // Truth shared with session/todo-list baseline: 3 done · 1 current · 1 pending.
    canvas.write(top, INSPECTOR_LEFT, "Plan · 3 of 5 done", TextStyle::bright())?;
    canvas.write(
        top + 1,
        INSPECTOR_LEFT,
        "1 in progress · 1 pending",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 3,
        INSPECTOR_LEFT,
        "✓ Read auth middleware · done",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 4,
        INSPECTOR_LEFT,
        "✓ Add token-bucket limiter · done",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 5,
        INSPECTOR_LEFT,
        "✓ Wire POST /login route · done",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 6,
        INSPECTOR_LEFT,
        "● Add tests/login_ratelimit.rs",
        TextStyle::bright(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "  cover the fifth attempt · in progress",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 8,
        INSPECTOR_LEFT,
        "○ Run cargo test and cargo fmt",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "  verify the full change · pending",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 11,
        INSPECTOR_LEFT,
        "3 of 5 done · 1 in progress · 1 pending",
        TextStyle::text(),
    )?;

    for (subject, row) in [
        ("todo-done-auth-middleware", 3u16),
        ("todo-done-token-bucket", 4),
        ("todo-done-login-route", 5),
        ("todo-current-login-tests", 6),
        ("todo-pending-verify-suite", 8),
    ] {
        push_pair(
            candidates,
            events,
            screen_id,
            fixture_id,
            "todo-item",
            subject,
            reference(top + row, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
            "summary",
            None,
        );
    }
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "todo-summary",
        "todo-progress-summary",
        reference(top + 11, INSPECTOR_LEFT, INSPECTOR_LEFT + 36),
        "summary",
        None,
    );
    Ok(())
}

fn paint_subagents(
    canvas: &mut Canvas,
    top: u16,
    screen_id: &str,
    fixture_id: &str,
    candidates: &mut Vec<SemanticCandidate>,
    events: &mut Vec<SemanticEvent>,
) -> Result<(), String> {
    canvas.write(top, INSPECTOR_LEFT, "Delegated work", TextStyle::bright())?;
    canvas.write(
        top + 1,
        INSPECTOR_LEFT,
        "1 running · 1 done · 1 queued",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 3,
        INSPECTOR_LEFT,
        "● Explore auth paths · running",
        TextStyle::bright(),
    )?;
    canvas.write(
        top + 4,
        INSPECTOR_LEFT,
        "  find existing middleware · 12s",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 6,
        INSPECTOR_LEFT,
        "✓ Summarize redaction · done",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 7,
        INSPECTOR_LEFT,
        "  finished the audit notes",
        TextStyle::muted(),
    )?;
    canvas.write(
        top + 9,
        INSPECTOR_LEFT,
        "○ Design token refresh · queued",
        TextStyle::text(),
    )?;
    canvas.write(
        top + 10,
        INSPECTOR_LEFT,
        "  waits until explore finishes",
        TextStyle::muted(),
    )?;

    for (subject, row) in [
        ("subagent-running-explore-auth", 3u16),
        ("subagent-done-redaction-notes", 6),
        ("subagent-queued-token-refresh", 9),
    ] {
        push_pair(
            candidates,
            events,
            screen_id,
            fixture_id,
            "inspector-subagent",
            subject,
            reference(top + row, INSPECTOR_LEFT, INSPECTOR_LEFT + 34),
            "summary",
            None,
        );
    }
    push_pair(
        candidates,
        events,
        screen_id,
        fixture_id,
        "inspector-subagent-summary",
        "delegation-totals",
        reference(top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 28),
        "summary",
        None,
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
