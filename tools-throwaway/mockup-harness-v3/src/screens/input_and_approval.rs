//! Flow 2 multiline-composer and project-file mention fixtures.
//!
//! These two states keep the paused conversation visible while the bottom
//! surface either contains the authored draft itself or a focused file list.
//! The fixture deliberately records authored line boundaries separately from a
//! visual continuation, so a wrapped display line is never mistaken for a
//! newline the person entered.

use crate::{
    audit::{
        ActionEvidence, AuditDocument, Canvas, Identity, RenderRef, SemanticCandidate,
        SemanticEvent, TransitionTrace,
    },
    chrome::{
        layout::{ScreenComposition, StatusLine, CANVAS_ROWS},
        tokens::TextStyle,
    },
    Surface,
};

const PROJECT_PATH: &str = "~/project";
const BRANCH: &str = "main";
const MODEL: &str = "Claude Opus 4.8";
const DOCK_TOP: u16 = 31;
const DOCK_BOTTOM: u16 = 37;

pub(crate) const MENTION_FRAGMENT: &str = "@retry";
pub(crate) const INSERTED_FILE: &str = "@src/net/retry_policy.rs";
pub(crate) const MULTILINE_DRAFT: &str = "Check the retry path in @retry\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";

type ScreenEvidence = (
    Vec<SemanticCandidate>,
    Vec<SemanticEvent>,
    Vec<TransitionTrace>,
);

#[derive(Clone, Copy)]
struct FileCandidate {
    path: &'static str,
    detail: &'static str,
}

const PROJECT_FILES: [FileCandidate; 3] = [
    FileCandidate {
        path: "src/net/retry_policy.rs",
        detail: "project file · best match",
    },
    FileCandidate {
        path: "src/net/retry_queue.rs",
        detail: "project file",
    },
    FileCandidate {
        path: "tests/net/retry_policy_test.rs",
        detail: "project file · test",
    },
];

pub fn render(screen_id: &str, fixture_id: &str) -> Result<AuditDocument, String> {
    let statusline = StatusLine::new(PROJECT_PATH, BRANCH, MODEL)?;
    ScreenComposition::new(screen_id, Surface::ActionDock, statusline.clone()).validate()?;

    let mut canvas = Canvas::new();
    let (candidates, events, transitions) = match screen_id {
        "composer-multiline" => render_multiline_composer(&mut canvas, screen_id, fixture_id)?,
        "file-mention" => render_file_mention(&mut canvas, screen_id, fixture_id)?,
        _ => return Err(format!("unknown Flow 2 composer screen: {screen_id}")),
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

fn render_multiline_composer(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    let inserted_file = fixture_id.contains("file-mention-insert");
    let file_reference = if inserted_file {
        INSERTED_FILE
    } else {
        MENTION_FRAGMENT
    };
    let cursor_offset = if inserted_file { 48 } else { 30 };
    transcript(canvas)?;
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        "Draft · 3 authored lines · wraps visually · [@] Mention a file",
        TextStyle::bright(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 2,
        &format!(
            "› Check the retry path in {file_reference}| cursor at character {cursor_offset}"
        ),
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 3,
        "  Then explain the failure path that still needs coverage before the merge;",
        TextStyle::text(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 4,
        "    wrap: include a concise test plan with the visible recovery behavior.",
        TextStyle::muted(),
    )?;
    dock_line(
        canvas,
        DOCK_TOP + 5,
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;

    let transcript = semantic(
        screen_id,
        fixture_id,
        "transcript",
        "paused-conversation",
        reference(4, 4, 83),
        "summary",
        None,
    );
    let draft = semantic(
        screen_id,
        fixture_id,
        "draft",
        "multiline-authored-draft",
        reference(DOCK_TOP + 2, 4, 61),
        "focused",
        None,
    );
    let first_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-1",
        reference(DOCK_TOP + 2, 6, 36),
        "focused",
        None,
    );
    let second_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-2",
        reference(DOCK_TOP + 3, 6, 80),
        "focused",
        None,
    );
    let third_newline = semantic(
        screen_id,
        fixture_id,
        "composer-newline",
        "line-3",
        reference(DOCK_TOP + 5, 6, 46),
        "focused",
        None,
    );
    let wrap = semantic(
        screen_id,
        fixture_id,
        "composer-wrap",
        "line-2-visual-continuation",
        reference(DOCK_TOP + 4, 8, 78),
        "focused",
        None,
    );
    let cursor = semantic(
        screen_id,
        fixture_id,
        "cursor",
        &format!("draft-offset-{cursor_offset}"),
        reference(
            DOCK_TOP + 2,
            28 + file_reference.chars().count() as u16,
            29 + file_reference.chars().count() as u16,
        ),
        "focused",
        None,
    );
    let mention = semantic(
        screen_id,
        fixture_id,
        "primary-action",
        "action.composer-multiline.open-file-mention",
        reference(DOCK_TOP + 1, 52, 69),
        "summary",
        Some(action(
            "action.composer-multiline.open-file-mention",
            "Mention a project file",
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
    let pairs = [
        transcript,
        draft,
        first_newline,
        second_newline,
        third_newline,
        wrap,
        cursor,
        mention,
        status,
    ];
    Ok((
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        vec![transition(
            "action.composer-multiline.open-file-mention",
            "composer-multiline",
            "file-mention",
            "direct",
            "the exact multiline draft opens a project-file mention dock at the composer",
        )],
    ))
}

fn render_file_mention(
    canvas: &mut Canvas,
    screen_id: &str,
    fixture_id: &str,
) -> Result<ScreenEvidence, String> {
    transcript(canvas)?;
    let query = if fixture_id.contains("no-results") {
        "@zzq"
    } else {
        MENTION_FRAGMENT
    };
    draft_preview(canvas, query)?;
    let matches = fuzzy_file_matches(query);
    write_dock_border(canvas)?;
    dock_line(
        canvas,
        DOCK_TOP + 1,
        &format!(
            "@ files · project-relative paths · query: {query} · {} matching project files",
            matches.len()
        ),
        TextStyle::bright(),
    )?;

    let mut pairs = vec![
        semantic(
            screen_id,
            fixture_id,
            "transcript",
            "paused-conversation",
            reference(4, 4, 83),
            "summary",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "draft",
            "multiline-authored-draft",
            reference(12, 4, 74),
            "focused",
            None,
        ),
        semantic(
            screen_id,
            fixture_id,
            "filter",
            &format!("query-{}", query.trim_start_matches('@')),
            reference(DOCK_TOP + 1, 4, 68),
            "focused",
            None,
        ),
    ];

    if matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 2,
            &format!("No project files match {query}."),
            TextStyle::text(),
        )?;
        dock_line(
            canvas,
            DOCK_TOP + 3,
            "Keep typing or dismiss to keep this literal draft.",
            TextStyle::muted(),
        )?;
        dock_line(canvas, DOCK_TOP + 4, "", TextStyle::text())?;
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "[esc] Dismiss",
            TextStyle::bright(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "empty-result",
            &format!("no-match-{}", query.trim_start_matches('@')),
            reference(DOCK_TOP + 2, 4, 37),
            "focused",
            None,
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.file-mention.dismiss",
            reference(DOCK_TOP + 5, 4, 17),
            "summary",
            Some(action(
                "action.file-mention.dismiss",
                "Dismiss",
                false,
            )),
        ));
    } else {
        for (index, file) in matches.iter().enumerate() {
            let row = DOCK_TOP + 2 + index as u16;
            let selected = index == 0;
            let marker = if selected { "›" } else { " " };
            dock_line(
                canvas,
                row,
                &format!("{marker} {} · {}", file.path, file.detail),
                if selected {
                    TextStyle::accent()
                } else {
                    TextStyle::text()
                },
            )?;
            pairs.push(semantic(
                screen_id,
                fixture_id,
                "file-match",
                file.path,
                reference(row, 6, 6 + file.path.chars().count() as u16),
                "focused",
                None,
            ));
            if selected {
                pairs.push(semantic(
                    screen_id,
                    fixture_id,
                    "selection",
                    file.path,
                    reference(row, 4, 6 + file.path.chars().count() as u16),
                    "focused",
                    None,
                ));
            }
        }
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "[tab] Insert file  [esc] Dismiss  [↑↓] Choose",
            TextStyle::bright(),
        )?;
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "primary-action",
            "action.file-mention.insert-selected",
            reference(DOCK_TOP + 5, 4, 21),
            "summary",
            Some(action(
                "action.file-mention.insert-selected",
                "Insert file",
                false,
            )),
        ));
        pairs.push(semantic(
            screen_id,
            fixture_id,
            "secondary-action",
            "action.file-mention.dismiss",
            reference(DOCK_TOP + 5, 23, 38),
            "summary",
            Some(action(
                "action.file-mention.dismiss",
                "Dismiss",
                false,
            )),
        ));
    }

    for row in DOCK_TOP + 2 + matches.len() as u16..=DOCK_TOP + 4 {
        if row <= DOCK_TOP + 4 && !matches.is_empty() {
            dock_line(canvas, row, "", TextStyle::text())?;
        }
    }
    if !matches.is_empty() {
        dock_line(
            canvas,
            DOCK_TOP + 5,
            "[tab] Insert file  [esc] Dismiss  [↑↓] Choose",
            TextStyle::bright(),
        )?;
    }

    let status = semantic(
        screen_id,
        fixture_id,
        "statusline",
        "shared-statusline",
        reference(CANVAS_ROWS - 1, 2, 38),
        "summary",
        None,
    );
    pairs.push(status);

    let transitions = if matches.is_empty() {
        vec![transition(
            "action.file-mention.dismiss",
            "file-mention",
            "composer-multiline",
            "cancel",
            "dismissal returns the literal visible query to its exact multiline draft",
        )]
    } else {
        vec![
            transition(
                "action.file-mention.insert-selected",
                "file-mention",
                "composer-multiline",
                "direct",
                "inserting one selected project file replaces only the active mention span",
            ),
            transition(
                "action.file-mention.dismiss",
                "file-mention",
                "composer-multiline",
                "cancel",
                "dismissal restores the literal visible query and original cursor",
            ),
        ]
    };
    Ok((
        pairs.iter().map(|(candidate, _)| candidate.clone()).collect(),
        pairs.iter().map(|(_, event)| event.clone()).collect(),
        transitions,
    ))
}

fn transcript(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        4,
        4,
        "You · Review the retry path before we merge.",
        TextStyle::bright(),
    )?;
    canvas.write(7, 4, "Code", TextStyle::bright())?;
    canvas.write(
        9,
        4,
        "I can add a project file without changing the rest of your draft.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn draft_preview(canvas: &mut Canvas, mention_fragment: &str) -> Result<(), String> {
    canvas.write(12, 4, "Paused draft · 3 authored lines", TextStyle::muted())?;
    canvas.write(
        13,
        4,
        &format!("› Check the retry path in {mention_fragment}"),
        TextStyle::text(),
    )?;
    canvas.write(
        14,
        4,
        "  Then explain the failure path that still needs coverage before the merge;",
        TextStyle::text(),
    )?;
    canvas.write(
        15,
        4,
        "    wrap: include a concise test plan with the visible recovery behavior.",
        TextStyle::muted(),
    )?;
    canvas.write(
        16,
        4,
        "  Keep the summary short before we merge.",
        TextStyle::text(),
    )?;
    Ok(())
}

fn write_dock_border(canvas: &mut Canvas) -> Result<(), String> {
    canvas.write(
        DOCK_TOP,
        2,
        &format!("╭{}╮", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    canvas.write(
        DOCK_BOTTOM,
        2,
        &format!("╰{}╯", "─".repeat(114)),
        TextStyle::muted(),
    )?;
    Ok(())
}

fn dock_line(
    canvas: &mut Canvas,
    row: u16,
    text: &str,
    style: TextStyle,
) -> Result<(), String> {
    canvas.write(row, 2, "│", TextStyle::muted())?;
    canvas.write(row, 4, text, style)?;
    canvas.write(row, 117, "│", TextStyle::muted())?;
    Ok(())
}

fn fuzzy_file_matches(query: &str) -> Vec<FileCandidate> {
    let fragment = query.trim_start_matches('@').to_ascii_lowercase();
    let mut matches = PROJECT_FILES
        .iter()
        .copied()
        .filter(|file| fuzzy_subsequence(file.path, &fragment))
        .collect::<Vec<_>>();
    matches.sort_by_key(|file| fuzzy_rank(file.path, &fragment));
    matches
}

fn fuzzy_subsequence(path: &str, fragment: &str) -> bool {
    let mut remaining = fragment.chars();
    let mut wanted = remaining.next();
    for character in path.to_ascii_lowercase().chars() {
        if Some(character) == wanted {
            wanted = remaining.next();
            if wanted.is_none() {
                return true;
            }
        }
    }
    wanted.is_none()
}

fn fuzzy_rank<'a>(path: &'a str, fragment: &str) -> (usize, usize, &'a str) {
    let normalized = path.to_ascii_lowercase();
    let file_start = normalized.rfind('/').map_or(0, |index| index + 1);
    let direct_start = normalized[file_start..]
        .find(fragment)
        .map_or(usize::MAX, |index| index + file_start);
    let test_penalty = usize::from(normalized.contains("tests/"));
    (test_penalty, direct_start, path)
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

#[cfg(test)]
mod tests {
    use super::{fuzzy_file_matches, fuzzy_subsequence};

    #[test]
    fn fuzzy_file_results_are_query_matched_and_rank_the_project_policy_first() {
        let matches = fuzzy_file_matches("@retry");
        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].path, "src/net/retry_policy.rs");
        assert!(matches
            .iter()
            .all(|candidate| fuzzy_subsequence(candidate.path, "retry")));
        assert!(fuzzy_file_matches("@zzq").is_empty());
    }
}
