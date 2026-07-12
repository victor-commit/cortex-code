//! Flow 3 session choice and chronological history pilot states.
//!
//! The session picker remains attached to the conversation as a bottom action
//! dock. Its selected row owns one read-only destination preview. Timeline is a
//! temporary detail view that always returns to that exact preview.

use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace, WorkStyleEvidence,
    },
    chrome::{
        layout::{ScreenComposition, StatusLine, CANVAS_ROWS},
        tokens::TextStyle,
    },
    Surface,
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Sonnet 4.5";
const DOCK_TOP: u16 = 30;

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    let surface = match screen_id {
        "session-picker" => Surface::ActionDock,
        "timeline" => Surface::TemporaryDetailView,
        _ => return Err(format!("unknown Flow 3 session screen: {screen_id}")),
    };
    ScreenComposition::new(screen_id, surface, statusline.clone()).validate()?;
    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "session-picker" => render_session_picker(&mut canvas, screen_id, fixture_id)?,
        "timeline" => render_timeline(&mut canvas, screen_id, fixture_id)?,
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
        work_style: Some(WorkStyleEvidence {
            applied: "Careful edits".to_owned(),
            pending: None,
            access_scope: "asks before editing project files".to_owned(),
        }),
        model_choice: None,
        conversation: None,
    })
}

fn render_session_picker(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    if fixture_id.contains("no-sessions") {
        return render_no_sessions(canvas, screen_id, fixture_id);
    }
    if fixture_id.contains("no-matches") {
        return render_no_matches(canvas, screen_id, fixture_id);
    }

    let gate_origin = fixture_id.starts_with("gate.work-configuration");
    let docs_selected = fixture_id.contains("selected-docs");
    let selected_index = usize::from(docs_selected);
    canvas.write(
        3,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(6, 4, "Resume a conversation", TextStyle::bright())?;
    canvas.write(
        8,
        4,
        "Filter · all saved conversations · 3 matches",
        TextStyle::muted(),
    )?;
    let rows = [
        (
            "Retry policy review",
            "~/project · Today, 10:42 · Current",
            "Tests pass; review is ready",
        ),
        (
            "Document release steps",
            "~/project · Yesterday, 16:08",
            "Drafted the release checklist",
        ),
        (
            "Fix search pagination",
            "~/storefront · Monday, 14:21",
            "Found the boundary condition",
        ),
    ];
    for (index, (title, context, latest)) in rows.iter().enumerate() {
        let row = 11 + index as u16 * 5;
        let selected = index == selected_index;
        canvas.write(
            row,
            4,
            &format!("{} {title}", if selected { "›" } else { " " }),
            if selected {
                TextStyle::accent()
            } else {
                TextStyle::text()
            },
        )?;
        canvas.write(row + 1, 6, context, TextStyle::muted())?;
        canvas.write(row + 2, 6, latest, TextStyle::text())?;
    }

    let (selected_title, selected_context, latest, draft) = if docs_selected {
        (
            "Document release steps",
            "~/project · latest stable work from yesterday",
            "Drafted the release checklist",
            "No saved draft",
        )
    } else {
        (
            "Retry policy review",
            "~/project · latest stable work from today",
            "Tests pass; review is ready",
            "Your saved draft: Add the final review note.",
        )
    };
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!("Selected · {selected_title}"),
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        &format!("Latest stable work · {latest}"),
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        &format!("{selected_context} · {draft}"),
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "› [enter] Resume latest · [t] View timeline",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "[f] Fork instead makes a copy · [esc] Back · [↑↓] Choose",
        TextStyle::muted(),
    )?;

    let row_subjects = [
        "retry-policy-review",
        "document-release-steps",
        "fix-search-pagination",
    ];
    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "conversation-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "picker",
            "saved-conversations",
            reference(6, 4, 25),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            "all-saved-conversations",
            reference(8, 4, 50),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(3, 4, 48),
            "summary",
            None,
        ),
    ];
    for (index, subject) in row_subjects.iter().enumerate() {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "session-row",
            subject,
            reference(11 + index as u16 * 5, 4, 34),
            if index == selected_index {
                "focused"
            } else {
                "summary"
            },
            None,
        ));
    }
    pairs.extend([
        semantic(
            screen_id,
            fixture_id,
            "selected-detail",
            row_subjects[selected_index],
            reference(DOCK_TOP + 1, 4, 45),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "resume-destination",
            "latest-stable-work",
            reference(DOCK_TOP + 2, 4, 73),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.session-picker.resume",
            reference(DOCK_TOP + 4, 4, 30),
            "focused",
            Some(action(
                "action.session-picker.resume",
                "Resume latest",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.session-picker.timeline",
            reference(DOCK_TOP + 4, 31, 49),
            "summary",
            Some(action(
                "action.session-picker.timeline",
                "View timeline",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.session-picker.fork",
            reference(DOCK_TOP + 5, 4, 37),
            "summary",
            Some(action("action.session-picker.fork", "Fork instead", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.session-picker.back",
            reference(DOCK_TOP + 5, 38, 50),
            "summary",
            Some(action("action.session-picker.back", "Back", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "local-control",
            "action.session-picker.choose",
            reference(DOCK_TOP + 5, 51, 65),
            "summary",
            Some(action(
                "action.session-picker.choose",
                "Choose session",
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
    ]);

    Ok(evidence(
        pairs,
        vec![
            transition(
                "action.session-picker.resume",
                "session-picker",
                "chat-turn",
                "direct",
                "resume opens the selected conversation at its latest stable state with its project, work style, transcript, and draft",
            ),
            transition(
                "action.session-picker.timeline",
                "session-picker",
                "timeline",
                "direct",
                "timeline opens a temporary chronological detail tied to the selected preview",
            ),
            transition(
                "action.session-picker.fork",
                "session-picker",
                "fork-picker",
                "direct",
                "fork opens a separate copy decision and leaves the source conversation unchanged",
            ),
            transition(
                "action.session-picker.back",
                "session-picker",
                if gate_origin { "onboarding" } else { "chat-turn" },
                "cancel",
                if gate_origin {
                    "back restores the onboarding configuration hub without applying a session"
                } else {
                    "back restores the exact conversation origin, draft, focus, and reading position"
                },
            ),
            transition(
                "action.session-picker.choose",
                "session-picker",
                "session-picker",
                "direct",
                "choosing another row changes only the read-only preview",
            ),
        ],
    ))
}

fn render_no_sessions(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        3,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Resume a conversation", TextStyle::bright())?;
    canvas.write(11, 4, "No saved conversations yet", TextStyle::text())?;
    canvas.write(
        13,
        4,
        "Your current draft stays here. Start when you are ready.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Nothing to resume",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Start a new conversation without changing the paused origin.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Start a conversation",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[esc] Back · your draft and reading position stay unchanged",
        TextStyle::muted(),
    )?;
    dock_line(canvas, DOCK_TOP + 5, "", TextStyle::muted())?;

    let pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "conversation-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "empty-state",
            "no-saved-conversations",
            reference(11, 4, 30),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.session-picker.start",
            reference(DOCK_TOP + 3, 4, 36),
            "focused",
            Some(action(
                "action.session-picker.start",
                "Start a conversation",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.session-picker.back",
            reference(DOCK_TOP + 4, 4, 15),
            "summary",
            Some(action("action.session-picker.back", "Back", false)),
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
    Ok(evidence(
        pairs,
        vec![
            transition(
                "action.session-picker.start",
                "session-picker",
                "empty",
                "direct",
                "start opens one usable empty conversation while preserving the recorded origin draft",
            ),
            transition(
                "action.session-picker.back",
                "session-picker",
                "chat-turn",
                "cancel",
                "back restores the exact conversation origin",
            ),
        ],
    ))
}

fn render_no_matches(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(
        3,
        4,
        "You · Continue the paused conversation.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Resume a conversation", TextStyle::bright())?;
    canvas.write(9, 4, "Filter · billing · 0 matches", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        "No conversations match \"billing\"",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "Clear the filter to restore the same list, focus, and reading position.",
        TextStyle::muted(),
    )?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "No matching conversations",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        "Your current conversation and draft are unchanged.",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "› [enter] Clear filter",
        TextStyle::accent(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "[esc] Back · keep the filter and return to the conversation",
        TextStyle::muted(),
    )?;
    dock_line(canvas, DOCK_TOP + 5, "", TextStyle::muted())?;

    let pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "conversation-origin",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            "billing",
            reference(9, 4, 33),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "empty-state",
            "no-filter-matches",
            reference(13, 4, 38),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(3, 4, 48),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.session-picker.clear-filter",
            reference(DOCK_TOP + 3, 4, 29),
            "focused",
            Some(action(
                "action.session-picker.clear-filter",
                "Clear filter",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.session-picker.back",
            reference(DOCK_TOP + 4, 4, 15),
            "summary",
            Some(action("action.session-picker.back", "Back", false)),
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
    Ok(evidence(
        pairs,
        vec![
            transition(
                "action.session-picker.clear-filter",
                "session-picker",
                "session-picker",
                "direct",
                "clear filter restores the saved-session rows at the preserved focus and scroll position",
            ),
            transition(
                "action.session-picker.back",
                "session-picker",
                "chat-turn",
                "cancel",
                "back restores the exact conversation origin and leaves the filter local to the closed picker",
            ),
        ],
    ))
}

fn render_timeline(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    canvas.write(2, 4, "Retry policy review", TextStyle::bright())?;
    canvas.write(
        4,
        4,
        "~/project · Session history · Oldest first",
        TextStyle::muted(),
    )?;
    canvas.write(
        6,
        4,
        "A temporary detail from the selected session preview.",
        TextStyle::text(),
    )?;
    let events = [
        (
            9,
            "Today, 09:14",
            "Opened the retry review",
            "Started from the failing network test.",
        ),
        (
            14,
            "Today, 09:37",
            "Updated the retry policy",
            "Checkpoint · Fork available from this point.",
        ),
        (
            19,
            "Today, 10:06",
            "Tests passed",
            "All focused retry checks completed.",
        ),
        (
            24,
            "Today, 10:42",
            "Current · Review ready",
            "Latest stable state · saved draft is recoverable.",
        ),
    ];
    for (row, time, title, detail) in events {
        canvas.write(row, 4, time, TextStyle::muted())?;
        canvas.write(
            row + 1,
            8,
            title,
            if title.starts_with("Current") {
                TextStyle::accent()
            } else {
                TextStyle::bright()
            },
        )?;
        canvas.write(row + 2, 8, detail, TextStyle::text())?;
    }
    canvas.write(
        30,
        4,
        "Selected point · Today, 09:37 · Updated the retry policy",
        TextStyle::text(),
    )?;
    canvas.write(
        32,
        4,
        "Historical points fork into a copy. Only the latest state can resume directly.",
        TextStyle::muted(),
    )?;
    canvas.write(35, 4, "› [enter] Resume latest", TextStyle::accent())?;
    canvas.write(
        36,
        4,
        "[f] Fork from selected point · [esc] Back to session preview · [↑↓] Choose point",
        TextStyle::muted(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "detail-view",
            "session-history",
            reference(2, 4, 25),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "origin",
            "selected-session-preview",
            reference(6, 4, 60),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "work-style",
            "work_style",
            reference(4, 4, 47),
            "summary",
            None,
        ),
    ];
    for (index, row) in [9_u16, 14, 19, 24].into_iter().enumerate() {
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "timeline-event",
            &format!("chronological-point-{}", index + 1),
            reference(row, 4, 21),
            if index == 3 { "focused" } else { "summary" },
            None,
        ));
    }
    pairs.extend([
        semantic(
            screen_id,
            fixture_id,
            "selected-point",
            "retry-policy-checkpoint",
            reference(30, 4, 64),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.timeline.resume",
            reference(35, 4, 29),
            "focused",
            Some(action("action.timeline.resume", "Resume latest", false)),
        ),
        semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.timeline.fork",
            reference(36, 4, 36),
            "summary",
            Some(action(
                "action.timeline.fork",
                "Fork from selected point",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "cancel-action",
            "action.timeline.back",
            reference(36, 37, 68),
            "summary",
            Some(action(
                "action.timeline.back",
                "Back to session preview",
                false,
            )),
        ),
        semantic(
            screen_id,
            fixture_id,
            "local-control",
            "action.timeline.choose",
            reference(36, 69, 87),
            "summary",
            Some(action(
                "action.timeline.choose",
                "Choose timeline point",
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
    ]);
    Ok(evidence(
        pairs,
        vec![
            transition(
                "action.timeline.resume",
                "timeline",
                "chat-turn",
                "direct",
                "resume is valid only for the latest stable state and restores that selected conversation",
            ),
            transition(
                "action.timeline.fork",
                "timeline",
                "fork-picker",
                "direct",
                "a historical point opens a non-destructive fork confirmation",
            ),
            transition(
                "action.timeline.back",
                "timeline",
                "session-picker",
                "cancel",
                "back restores the exact selected preview, filter, focus, scroll position, and conversation origin",
            ),
            transition(
                "action.timeline.choose",
                "timeline",
                "timeline",
                "direct",
                "choosing a point changes only the temporary read-only selection",
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

fn evidence(
    pairs: Vec<(SemanticCandidate, SemanticEvent)>,
    transitions: Vec<TransitionTrace>,
) -> ScreenEvidence {
    (
        pairs
            .iter()
            .map(|(candidate, _)| candidate.clone())
            .collect(),
        pairs.into_iter().map(|(_, event)| event).collect(),
        transitions,
    )
}
