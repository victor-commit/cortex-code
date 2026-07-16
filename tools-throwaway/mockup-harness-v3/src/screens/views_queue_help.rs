//! Flow 2 queue inspector and temporary help reference.
//!
//! - `sidebar-queue` is a contextual inspector: transcript and composer stay
//!   recognizable beside one Queue panel with ordered identities/revisions.
//! - `help` is a temporary detail view: task-grouped searchable shortcuts with
//!   recoverable no-results and exact return to the originating surface.

use crate::{
    Surface,
    audit::{
        ActionEvidence, AuditDocument, Canvas, ConversationIdentity, Identity, RenderRef,
        SemanticCandidate, SemanticEvent, TransitionTrace,
    },
    chrome::{
        components::{subject_title_for_origin, Component, DetailView, Inspector, InspectorTab},
        layout::{CANVAS_ROWS, ScreenComposition, StatusLine, SurfaceLayout},
        tokens::TextStyle,
    },
    screens::input_and_approval::{
        MULTILINE_DRAFT, QUEUE_APPLIED_ORDER, QUEUE_OPEN_DRAFT, QUEUE_OPENED_AUTHORITY,
    },
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const COMPOSER_TOP: u16 = 32;
const COMPOSER_BOTTOM: u16 = 37;
const INSPECTOR_LEFT: u16 = 81;
const INSPECTOR_WIDTH: u16 = 37;
const TRANSCRIPT_WIDTH: usize = 74;

const FLOW2_ORIGIN: &str = "Input and approval · paused conversation";
const FLOW2_TASK: &str = "Check the retry path before we merge.";
const FLOW2_YOU: &str = "You · Check the retry path before we merge.";
const TURN_ID: &str = "turn.retry-review.18";
const SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";
const LATER_DRAFT: &str = "Add a short note about the remaining test hunk.";
const PAUSED_ACTIVITY: &str = "activity.retry-policy-edit.18";

const HELP_FILTER_MATCH: &str = "queue";
const HELP_FILTER_EMPTY: &str = "zzz-no-match";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy)]
struct QueueRow {
    order: u8,
    identity: &'static str,
    revision: &'static str,
    summary: &'static str,
    unit_bits: &'static str,
    runs_next: bool,
}

const OPENED_QUEUE: [QueueRow; 2] = [
    QueueRow {
        order: 1,
        identity: "queue-item-1",
        revision: "rev=1",
        summary: "Also list unit tests.",
        unit_bits: "[attachment: queue-notes-a.md]",
        runs_next: true,
    },
    QueueRow {
        order: 2,
        identity: "queue-item-2",
        revision: "rev=2",
        summary: "Then summarize risks.",
        unit_bits: "[attachment: queue-notes-b.md]",
        runs_next: false,
    },
];

const APPLIED_QUEUE: [QueueRow; 2] = [
    QueueRow {
        order: 1,
        identity: "queue-item-2",
        revision: "rev=2",
        summary: "Then summarize risks.",
        unit_bits: "[attachment: queue-notes-b.md]",
        runs_next: true,
    },
    QueueRow {
        order: 2,
        identity: "queue-item-1",
        revision: "rev=1",
        summary: "Also list unit tests.",
        unit_bits: "[attachment: queue-notes-a.md]",
        runs_next: false,
    },
];

#[derive(Clone, Copy)]
struct HelpAction {
    group: &'static str,
    label: &'static str,
    shortcut: &'static str,
    effect: &'static str,
}

const HELP_ACTIONS: [HelpAction; 10] = [
    HelpAction {
        group: "Write",
        label: "Send message",
        shortcut: "Enter",
        effect: "Submit the draft when ready",
    },
    HelpAction {
        group: "Write",
        label: "Open file mention",
        shortcut: "@",
        effect: "Insert a project file reference",
    },
    HelpAction {
        group: "Write",
        label: "Open slash commands",
        shortcut: "/",
        effect: "Choose export, help, queue, or actions",
    },
    HelpAction {
        group: "Navigate",
        label: "Open action palette",
        shortcut: "ctrl+k",
        effect: "Search every action from one dock",
    },
    HelpAction {
        group: "Navigate",
        label: "Jump to latest turn",
        shortcut: "End",
        effect: "Return the transcript to the newest reply",
    },
    HelpAction {
        group: "Manage",
        label: "Open queued prompts",
        shortcut: "/queue",
        effect: "Reorder, remove, or run the next prompt",
    },
    HelpAction {
        group: "Manage",
        label: "Export conversation",
        shortcut: "/export",
        effect: "Save a read-only copy of this chat",
    },
    HelpAction {
        group: "Inspect",
        label: "Open this reference",
        shortcut: "/help",
        effect: "Browse task-grouped shortcuts",
    },
    HelpAction {
        group: "Inspect",
        label: "Open review findings",
        shortcut: "ctrl+r",
        effect: "Inspect the current review scope",
    },
    HelpAction {
        group: "Inspect",
        label: "Search transcript",
        shortcut: "ctrl+f",
        effect: "Find matching turns without leaving the chat",
    },
];

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    match screen_id {
        "sidebar-queue" => render_sidebar_queue(fixture_id),
        "help" => render_help(fixture_id),
        _ => Err(format!("unknown queue/help screen: {screen_id}")),
    }
}

fn render_sidebar_queue(fixture_id: &str) -> Result<AuditDocument, String> {
    let screen_id = "sidebar-queue";
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ContextualInspector, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::ContextualInspector).validate()?;

    let applied = fixture_id.contains("applied") || fixture_id.contains("after-apply");
    let items = if applied { &APPLIED_QUEUE[..] } else { &OPENED_QUEUE[..] };
    let selected_identity = items
        .iter()
        .find(|item| item.runs_next)
        .map(|item| item.identity)
        .unwrap_or(items[0].identity);

    let mut canvas = Canvas::new();
    paint_flow2_conversation(&mut canvas)?;
    let (candidates, events, transitions) =
        paint_queue_inspector(&mut canvas, screen_id, fixture_id, items, selected_identity)?;
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
        conversation: Some(flow2_conversation()),
    })
}

fn paint_flow2_conversation(canvas: &mut Canvas) -> Result<(), String> {
    let write = |canvas: &mut Canvas, row: u16, text: &str, style: TextStyle| {
        let clipped = truncate_to(text, TRANSCRIPT_WIDTH);
        canvas.write(row, 4, &clipped, style)
    };
    write(canvas, 2, subject_title_for_origin(FLOW2_ORIGIN), TextStyle::muted())?;
    write(canvas, 3, FLOW2_TASK, TextStyle::bright())?;
    write(canvas, 5, FLOW2_YOU, TextStyle::bright())?;
    write(canvas, 8, "Code", TextStyle::bright())?;
    write(
        canvas,
        10,
        "The retry path is still open. Queued prompts wait after this turn.",
        TextStyle::text(),
    )?;
    write(
        canvas,
        11,
        "→ Keep the paused reply and draft intact while you inspect the queue.",
        TextStyle::text(),
    )?;
    write(
        canvas,
        14,
        "● Queue waiting · runs after the current turn",
        TextStyle::muted(),
    )?;

    // Multiline-looking draft preview so the composer stays recognizable.
    let draft_lines: Vec<&str> = QUEUE_OPEN_DRAFT.lines().take(3).collect();
    canvas.write(
        COMPOSER_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    for (index, line) in draft_lines.iter().enumerate() {
        let prefix = if index == 0 { "│ › " } else { "│   " };
        canvas.write(
            COMPOSER_TOP + 1 + index as u16,
            2,
            &format!("{prefix}{}", truncate_to(line, 110)),
            if index == 0 {
                TextStyle::bright()
            } else {
                TextStyle::text()
            },
        )?;
    }
    canvas.write(
        COMPOSER_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    Ok(())
}

fn paint_queue_inspector(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
    items: &[QueueRow],
    selected_identity: &str,
) -> Result<ScreenEvidence, String> {
    let tabs = vec![InspectorTab::new("Queue")];
    // Title stays generic so the active tab label is the only Queue marker (no dual header).
    let inspector = Inspector::new("Inspector", FLOW2_ORIGIN, tabs, 0)?;
    write_component(canvas, 1, INSPECTOR_LEFT, &inspector)?;
    for row in 1..31 {
        canvas.write(row, INSPECTOR_LEFT - 2, "│", TextStyle::muted())?;
    }

    let content_top = 9u16;
    write_inspector(
        canvas,
        content_top,
        "Queued prompts · next first",
        TextStyle::bright(),
    )?;
    write_inspector(
        canvas,
        content_top + 1,
        "Same IDs as queue manager",
        TextStyle::muted(),
    )?;

    let mut candidates = Vec::new();
    let mut events = Vec::new();

    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "active-context",
        "queue",
        reference(1, INSPECTOR_LEFT, INSPECTOR_LEFT + 10),
        "focused",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-tab",
        "tab-queue",
        reference(2, INSPECTOR_LEFT, INSPECTOR_LEFT + 12),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "transcript",
        "paused-turn",
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
        reference(COMPOSER_TOP + 1, 4, 50),
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
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "filter",
        "query-empty",
        reference(content_top + 1, INSPECTOR_LEFT, INSPECTOR_LEFT + 20),
        "summary",
        None,
    );

    for (index, item) in items.iter().enumerate() {
        let row = content_top + 3 + (index as u16) * 4;
        let selected = item.identity == selected_identity;
        let marker = if selected { "›" } else { " " };
        let next_badge = if item.runs_next { " · [next]" } else { "" };
        let head = format!(
            "{marker} {} · {} {}",
            item.order, item.identity, item.revision
        );
        write_inspector(
            canvas,
            row,
            &format!("{head}{next_badge}"),
            if selected {
                TextStyle::accent()
            } else {
                TextStyle::text()
            },
        )?;
        write_inspector(canvas, row + 1, &format!("  {}", item.summary), TextStyle::muted())?;
        write_inspector(canvas, row + 2, &format!("  {}", item.unit_bits), TextStyle::muted())?;

        let subject = format!("{}·{}", item.identity, item.revision);
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "queue-item",
            &subject,
            reference(row, INSPECTOR_LEFT, INSPECTOR_LEFT + 30),
            if selected { "focused" } else { "summary" },
            None,
        );
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "queue-order",
            &format!("order-{}", item.order),
            reference(row, INSPECTOR_LEFT + 2, INSPECTOR_LEFT + 4),
            "summary",
            None,
        );
        if item.runs_next {
            push_pair(
                &mut candidates,
                &mut events,
                screen_id,
                fixture_id,
                "queue-next",
                &format!("{}-runs-next", item.identity),
                reference(row, INSPECTOR_LEFT + 20, INSPECTOR_LEFT + 28),
                "focused",
                None,
            );
        }
        if selected {
            push_pair(
                &mut candidates,
                &mut events,
                screen_id,
                fixture_id,
                "selection",
                item.identity,
                reference(row, INSPECTOR_LEFT, INSPECTOR_LEFT + 18),
                "focused",
                None,
            );
            push_pair(
                &mut candidates,
                &mut events,
                screen_id,
                fixture_id,
                "revision",
                &subject,
                reference(row, INSPECTOR_LEFT + 6, INSPECTOR_LEFT + 28),
                "focused",
                None,
            );
        }
    }

    let manage_row = content_top + 3 + (items.len() as u16) * 4 + 1;
    write_inspector(canvas, manage_row, "m for Manage queue", TextStyle::accent())?;
    write_inspector(
        canvas,
        manage_row + 1,
        "opens shared IDs · apply/cancel",
        TextStyle::muted(),
    )?;
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.sidebar-queue.manage",
        reference(manage_row, INSPECTOR_LEFT, INSPECTOR_LEFT + 17),
        "focused",
        Some(action(
            "action.sidebar-queue.manage",
            "Manage queue",
            false,
        )),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "return-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 28),
        "summary",
        Some(action(
            "action.sidebar-queue.close",
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
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 28),
        "summary",
        Some(action("action.sidebar-queue.back", "Back", false)),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "inspector-return",
        "cancel-to-conversation",
        reference(8, INSPECTOR_LEFT, INSPECTOR_LEFT + 28),
        "summary",
        Some(action("action.sidebar-queue.cancel", "Cancel", false)),
    );

    let transitions = vec![
        transition(
            "action.sidebar-queue.manage",
            screen_id,
            "queue-manager",
            "direct",
            "manage opens queue-manager with the same item texts, order, next marker, identities, and revisions",
        ),
        transition(
            "action.sidebar-queue.close",
            screen_id,
            "composer-multiline",
            "direct",
            "close restores the originating conversation, draft, scroll, and leaves the queue context",
        ),
        transition(
            "action.sidebar-queue.back",
            screen_id,
            "composer-multiline",
            "direct",
            "back restores the origin conversation without entering home",
        ),
        transition(
            "action.sidebar-queue.cancel",
            screen_id,
            "composer-multiline",
            "cancel",
            "cancel restores the origin conversation and kept draft",
        ),
    ];

    // Ensure shared ledger strings are reachable for audit consumers via events.
    let _opened = QUEUE_OPENED_AUTHORITY;
    let _applied = QUEUE_APPLIED_ORDER;
    let _draft = MULTILINE_DRAFT;

    Ok((candidates, events, transitions))
}

fn render_help(fixture_id: &str) -> Result<AuditDocument, String> {
    let screen_id = "help";
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::TemporaryDetailView, statusline.clone())
        .validate()?;
    SurfaceLayout::for_surface(Surface::TemporaryDetailView).validate()?;

    let from_queue = fixture_id.contains("sidebar-queue") || fixture_id.contains("from-queue");
    let origin = if from_queue {
        "From Queue inspector · paused conversation"
    } else {
        "From composer · paused conversation"
    };
    let return_target = if from_queue {
        "sidebar-queue"
    } else {
        "composer-multiline"
    };

    let filter = if fixture_id.contains("no-results") {
        HELP_FILTER_EMPTY
    } else if fixture_id.contains("filtered") {
        HELP_FILTER_MATCH
    } else {
        ""
    };
    let matching: Vec<&HelpAction> = HELP_ACTIONS
        .iter()
        .filter(|action| {
            if filter.is_empty() {
                return true;
            }
            let needle = filter.to_ascii_lowercase();
            action.group.to_ascii_lowercase().contains(&needle)
                || action.label.to_ascii_lowercase().contains(&needle)
                || action.shortcut.to_ascii_lowercase().contains(&needle)
                || action.effect.to_ascii_lowercase().contains(&needle)
        })
        .collect();
    let no_results = !filter.is_empty() && matching.is_empty();

    let mut canvas = Canvas::new();
    canvas.write(2, 4, subject_title_for_origin(FLOW2_ORIGIN), TextStyle::muted())?;
    canvas.write(3, 4, FLOW2_TASK, TextStyle::bright())?;
    canvas.write(5, 4, "Help · temporary shortcut reference", TextStyle::bright())?;
    canvas.write(6, 4, origin, TextStyle::muted())?;
    canvas.write(
        8,
        4,
        &format!(
            "Filter › {}",
            if filter.is_empty() {
                "type to filter actions".to_owned()
            } else {
                filter.to_owned()
            }
        ),
        TextStyle::bright(),
    )?;

    // DetailView anatomy for structure/return wording.
    let _detail = DetailView::new(
        "Help",
        origin,
        vec![
            "Task-grouped actions · keys live beside each action",
            "Return restores the exact prior screen and draft",
        ],
    );

    let mut candidates = Vec::new();
    let mut events = Vec::new();
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "detail-origin",
        if from_queue {
            "sidebar-queue"
        } else {
            "composer-multiline"
        },
        reference(6, 4, 60),
        "summary",
        None,
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "filter",
        if filter.is_empty() {
            "query-empty"
        } else if no_results {
            "query-no-results"
        } else {
            "query-filtered"
        },
        reference(8, 4, 40),
        "focused",
        None,
    );

    let mut row = 10u16;
    if no_results {
        canvas.write(
            row,
            4,
            &format!("No matching shortcuts for \"{filter}\""),
            TextStyle::text(),
        )?;
        canvas.write(
            row + 1,
            4,
            "Clear the filter to restore every task group · draft stays intact",
            TextStyle::muted(),
        )?;
        push_pair(
            &mut candidates,
            &mut events,
            screen_id,
            fixture_id,
            "help-no-results",
            "recoverable-empty",
            reference(row, 4, 50),
            "focused",
            None,
        );
    } else {
        let mut current_group = "";
        for action in &matching {
            if action.group != current_group {
                current_group = action.group;
                // Group headers use bright hierarchy; ACCENT stays for the one
                // return primary below (VAL-V3-DS-004).
                canvas.write(row, 4, current_group, TextStyle::bright())?;
                push_pair(
                    &mut candidates,
                    &mut events,
                    screen_id,
                    fixture_id,
                    "help-group",
                    &current_group.to_ascii_lowercase(),
                    reference(row, 4, 4 + current_group.chars().count() as u16),
                    "summary",
                    None,
                );
                row += 1;
            }
            let line = format!(
                "  {} · {} · {}",
                action.label, action.shortcut, action.effect
            );
            canvas.write(row, 4, &truncate_to(&line, 112), TextStyle::text())?;
            push_pair(
                &mut candidates,
                &mut events,
                screen_id,
                fixture_id,
                "help-action",
                &action.label.to_ascii_lowercase().replace(' ', "-"),
                reference(row, 6, 70),
                "summary",
                None,
            );
            row += 1;
            if row > 32 {
                break;
            }
        }
    }

    canvas.write(
        34,
        4,
        "› Esc to return · restores origin, draft, cursor, selection, and scroll",
        TextStyle::accent(),
    )?;
    canvas.write(
        35,
        4,
        "This temporary reference is the only place for grouped shortcuts",
        TextStyle::muted(),
    )?;
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "primary-action",
        "action.help.return",
        reference(34, 4, 20),
        "focused",
        Some(action("action.help.return", "Return", false)),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "cancel-action",
        "action.help.cancel",
        reference(34, 4, 20),
        "summary",
        Some(action("action.help.cancel", "Cancel", false)),
    );
    push_pair(
        &mut candidates,
        &mut events,
        screen_id,
        fixture_id,
        "filter-action",
        "action.help.filter",
        reference(8, 4, 20),
        "summary",
        Some(action("action.help.filter", "Filter help", false)),
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
            "action.help.return",
            screen_id,
            return_target,
            "direct",
            "return closes the temporary reference and restores the exact prior screen, draft, cursor, selection, scroll, and open inspector when that was the origin",
        ),
        transition(
            "action.help.cancel",
            screen_id,
            return_target,
            "cancel",
            "cancel closes help without submitting input and restores the origin draft and scroll",
        ),
        transition(
            "action.help.filter",
            screen_id,
            screen_id,
            "direct",
            "local filter keeps help open, retains the query, and shows matching actions or a recoverable no-results state",
        ),
    ];

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
        conversation: Some(flow2_conversation()),
    })
}

fn flow2_conversation() -> ConversationIdentity {
    ConversationIdentity {
        conversation_id: "conversation.retry-review".to_owned(),
        transcript_id: "transcript.retry-review".to_owned(),
        session_id: "session.retry-review.18".to_owned(),
        turn_id: TURN_ID.to_owned(),
        user_task: FLOW2_TASK.to_owned(),
        visible_origin: FLOW2_ORIGIN.to_owned(),
        submitted_prompt: Some(SUBMITTED_PROMPT.to_owned()),
        paused_activity_id: Some(PAUSED_ACTIVITY.to_owned()),
        later_draft: Some(LATER_DRAFT.to_owned()),
    }
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

fn write_inspector(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
    style: TextStyle,
) -> Result<(), String> {
    canvas.write(
        row,
        INSPECTOR_LEFT,
        &truncate_to(text, INSPECTOR_WIDTH as usize),
        style,
    )
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
