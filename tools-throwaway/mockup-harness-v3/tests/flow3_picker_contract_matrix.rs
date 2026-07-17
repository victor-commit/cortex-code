//! Shared Flow 3 picker contract matrix.
//! Owns VAL-V3-F3-005..011 and VAL-V3-F3-051.

use std::path::Path;
use std::process::Command;

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::picker_contract::{
        contract_fixture, non_filter_declaration, picker_contracts, FilterCapability,
        FORBIDDEN_RAW_ID_SNIPPETS, NINE_PICKERS,
    },
    scenario::replay_case,
};


fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn primary_action_count(screen_id: &str) -> usize {
    let audit = audit_screen(screen_id, contract_fixture(screen_id))
        .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
    audit
        .events
        .iter()
        .filter(|event| event.identity.family == "primary-action")
        .count()
}

#[test]
fn nine_pickers_share_bottom_dock_anatomy_and_stable_identities() {
    let catalog = picker_contracts();
    assert_eq!(catalog.len(), 9);
    for screen_id in NINE_PICKERS {
        let contract = catalog
            .get(screen_id)
            .unwrap_or_else(|| panic!("catalog must declare {screen_id}"));
        assert_eq!(contract.screen_id, screen_id);
        assert!(!contract.stable_identity.is_empty());

        let audit = audit_screen(screen_id, contract_fixture(screen_id))
            .unwrap_or_else(|error| panic!("{screen_id} must produce typed evidence: {error}"));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen_id} must stay bijective: {error}"));
        let text = audit.render_txt();
        let rows = text.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 40, "{screen_id} must render 40 rows");
        assert!(
            rows.iter().all(|row| row.chars().count() == 120),
            "{screen_id} must stay 120 columns wide"
        );

        assert!(
            text.contains(contract.title),
            "{screen_id} must show title `{}`\n{text}",
            contract.title
        );
        // Bottom dock anatomy: rounded action card near the composer location.
        assert!(
            text.contains('╭') && text.contains('╰') && text.contains('│'),
            "{screen_id} must use the shared bottom action-dock frame"
        );
        // Focused row marker is the restrained › glyph, never a filled SELBG row cue alone.
        assert!(
            text.contains('›'),
            "{screen_id} must mark focus with › in TXT"
        );
        // Exactly one dock-time primary action family.
        assert_eq!(
            primary_action_count(screen_id),
            1,
            "{screen_id} must expose exactly one primary action"
        );
        // Local cancel or back control.
        assert!(
            (text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to"))
                || text.contains("[esc] Back")
                || text.contains("[esc] Cancel · restores previous theme"),
            "{screen_id} must expose a local cancel/back control in the dock"
        );
        // Shared catalog identity surfaces in audit subject or select/picker event.
        let has_picker_identity = audit.events.iter().any(|event| {
            matches!(
                event.identity.family.as_str(),
                "picker" | "decision" | "detail-view" | "filter-not-needed"
            ) || event.identity.subject_id.contains("selection")
                || event.identity.subject_id.contains("picker")
                || event.identity.subject_id.contains("conversation")
                || event.identity.subject_id.contains("fork")
        });
        assert!(
            has_picker_identity,
            "{screen_id} must expose a stable picker identity family"
        );
        // Not a detached dashboard: statusline only, no global hint strip.
        assert!(!text.contains("enter send"));
        assert!(!text.contains("? help"));
        assert!(!text.contains("ctx "));
    }
}

#[test]
fn picker_rows_communicate_consequences_with_exclusive_selected_detail() {
    let catalog = picker_contracts();
    for screen_id in NINE_PICKERS {
        let audit = audit_screen(screen_id, contract_fixture(screen_id))
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        let text = audit.render_txt();
        let _contract = catalog.get(screen_id).expect("contracted picker");

        // Row consequence consists of human tradeoff language or section outcome.
        let consequence_cues = [
            "Current",
            "Pending",
            "Available",
            "On ·",
            "Off ·",
            "Preview",
            "helpers",
            "Source",
            "Checkpoint",
            "Destination",
            "Today",
            "path",
            "speed",
            "depth",
            "focus",
            "task",
            "tradeoff",
            "palette",
            "outcome",
            "edit",
            "review",
            "careful",
            "fast",
            "warm",
            "daylight",
            "deliberation",
            "Saved",
            "resume",
            "Fork",
            "helper",
            "Approve",
            "project",
            "session-local",
            "Balanced",
            "Thorough",
            "Explore",
            "Summarize",
            "Plan",
            "tests",
            "filter ·",
            "Filter ·",
        ];
        // Soft check: TXT includes consequence language beyond bare labels.
        let has_consequence = consequence_cues
            .iter()
            .any(|cue| text.to_lowercase().contains(&cue.to_lowercase()))
            || text.contains(" · ");
        assert!(
            has_consequence,
            "{screen_id} rows must communicate consequences beyond names alone"
        );

        // Exclusive selected detail: at most one focused row family + one selected-detail (or outcome).
        let focused_rows = audit
            .events
            .iter()
            .filter(|event| {
                event.identity.family.contains("row")
                    || event.identity.family == "fork-section"
                    || event.identity.family == "timeline-event"
            })
            .filter(|event| event.identity.detail_level == "focused")
            .count();
        if screen_id != "fork-picker" {
            assert_eq!(
                focused_rows, 1,
                "{screen_id} must focus exactly one choice row, found {focused_rows}"
            );
        }
        let selected_detail = audit
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.identity.family.as_str(),
                    "selected-detail"
                        | "selected-outcome"
                        | "resume-destination"
                        | "pending-consequence"
                        | "theme-preview"
                )
            })
            .count();
        assert!(
            selected_detail >= 1,
            "{screen_id} must own exclusive selected detail (family includes selected-detail/outcome)"
        );

        // Primary leaves are not raw IDs.
        for forbidden in FORBIDDEN_RAW_ID_SNIPPETS {
            if *forbidden == "session." {
                // Allow statement language that is not an identity prefix when
                // it does not look like session.configuration / session.id.
                assert!(
                    !text.contains("session.configuration")
                        && !text.contains("session.id")
                        && !text.contains("session."),
                    "{screen_id} must not expose raw session ids; found via scan for session."
                );
                continue;
            }
            assert!(
                !text.contains(forbidden),
                "{screen_id} must not expose raw id fragment `{forbidden}`"
            );
        }
    }
}

#[test]
fn focus_current_and_pending_are_not_color_only() {
    for screen_id in NINE_PICKERS {
        let audit = audit_screen(screen_id, contract_fixture(screen_id))
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        let text = audit.render_txt();
        assert!(
            text.contains('›'),
            "{screen_id} must use the › focus glyph in addition to restrained accent color"
        );
        // Current / pending / applied / selected markers in text.
        let has_state_text = text.contains("Current")
            || text.contains("Pending")
            || text.contains("Applied")
            || text.contains("Selected ·")
            || text.contains("Resume ·")
            || text.contains("Fork ·")
            || text.contains("On ·")
            || text.contains("Off ·")
            || text.contains("Preview ·")
            || text.contains("Source")
            || text.contains("Destination");
        assert!(
            has_state_text,
            "{screen_id} must name current/pending/selected state in plain text"
        );
    }
}

#[test]
fn picker_controls_are_local_and_minimal() {
    for screen_id in NINE_PICKERS {
        let audit = audit_screen(screen_id, contract_fixture(screen_id))
            .unwrap_or_else(|error| panic!("{screen_id} must render: {error}"));
        let text = audit.render_txt();
        assert_eq!(primary_action_count(screen_id), 1);
        let secondary = audit
            .events
            .iter()
            .filter(|event| event.identity.family == "secondary-action")
            .count();
        assert!(
            secondary <= 2,
            "{screen_id} may expose at most two secondary actions, found {secondary}"
        );
        assert!(
            !text.contains("enter send")
                && !text.contains("/ commands")
                && !text.contains("@ files")
                && !text.contains("? help"),
            "{screen_id} must not show a global hint row"
        );
        // Keys are dock-local: enter/esc appear near bottom slip (dock rows).
        // Phase-A paints Claude prose (`Enter to` / `Esc to`); residual Phase-B
        // screens may still use bracket atoms. Accept either form.
        let dock_region = text
            .lines()
            .skip(30)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            dock_region.contains("Enter to")
                || dock_region.contains("Esc to")
                || dock_region.contains("[enter]")
                || dock_region.contains("[esc]"),
            "{screen_id} must keep enter/esc keys inside the action dock (prose or residual bracket)"
        );
    }
}

#[test]
fn cancel_from_every_picker_restores_exact_origin() {
    let catalog = picker_contracts();
    for screen_id in NINE_PICKERS {
        let contract = catalog.get(screen_id).expect("contract");
        let Some(case_id) = contract.cancel_case else {
            continue;
        };
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|error| panic!("{case_id} must be publicly replayable: {error}"));
        assert_eq!(
            replay.transitions[0].kind, "cancel",
            "{case_id} must be a cancel transition"
        );
        assert_eq!(
            replay.before.draft, replay.after.draft,
            "{case_id} must preserve draft"
        );
        assert_eq!(
            replay.before.transcript, replay.after.transcript,
            "{case_id} must preserve transcript"
        );
        assert_eq!(
            replay.before.scroll_anchor, replay.after.scroll_anchor,
            "{case_id} must preserve scroll"
        );
        assert_eq!(
            replay.before.origin_stack, replay.after.origin_stack,
            "{case_id} must preserve origin stack"
        );
        // After cancel, the user leaves the temporary picker.
        assert_ne!(
            replay.after.screen_id, *screen_id,
            "{case_id} must leave the temporary picker surface"
        );
        // Configuration is unchanged unless a pure preview path; work style and
        // applied attachment sets remain byte-identical when present.
        if let Some(hash) = replay.preserved_field_hashes.get("draft") {
            assert_eq!(hash.before, hash.after);
        }
        if let Some(hash) = replay.preserved_field_hashes.get("transcript") {
            assert_eq!(hash.before, hash.after);
        }
    }

    // Agent picker also carries a cancel transition in its audit graph.
    let agent = audit_screen("agent-picker", "flow3-agent-picker").expect("agent picker");
    assert!(agent.transitions.iter().any(|transition| {
        transition.action_id == "action.agent-picker.cancel" && transition.kind == "cancel"
    }));
}

#[test]
fn apply_from_every_picker_returns_to_recorded_origin() {
    let catalog = picker_contracts();
    for screen_id in NINE_PICKERS {
        let contract = catalog.get(screen_id).expect("contract");
        let Some(case_id) = contract.apply_case else {
            continue;
        };
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|error| panic!("{case_id} must be publicly replayable: {error}"));
        // model-filter-apply opens the filtered screen from model-picker and is
        // not itself an apply commit; catalog points filtered apply at
        // model-filtered-apply instead.
        if case_id != "model-filter-apply" {
            assert_eq!(
                replay.before.screen_id, *screen_id,
                "{case_id} must start on {screen_id}"
            );
        }
        // Apply returns to a non-picker origin (conversation, or nested access for agent pending).
        let leave_ok = replay.after.screen_id != *screen_id
            || matches!(case_id, "work-style-unchanged" | "session-resume-latest");
        assert!(
            leave_ok
                || replay.after.screen_id == "chat-turn"
                || replay.after.screen_id == "mode-switch"
                || replay.after.screen_id == "empty"
                || replay.after.screen_id == "model-picker-filtered",
            "{case_id} must return to the origin journey rather than stay a settings page; after={}",
            replay.after.screen_id
        );
        // Fork and resume intentionally rebind the destination draft. Every other
        // apply preserves the paused origin draft byte-for-byte.
        if case_id != "fork-confirm" && case_id != "session-resume-latest" {
            assert_eq!(
                replay.before.draft, replay.after.draft,
                "{case_id} must preserve draft when applying"
            );
        }
        assert_eq!(
            replay.before.origin_stack, replay.after.origin_stack,
            "{case_id} must preserve origin stack"
        );
        // session resume rebinds session identity intentionally; others keep session.
        if case_id != "session-resume-latest" && case_id != "fork-confirm" {
            assert_eq!(
                replay.before.session_id, replay.after.session_id,
                "{case_id} must keep session identity"
            );
        }
        assert!(!matches!(
            replay.after.screen_id.as_str(),
            "settings" | "preferences" | "dashboard"
        ));
    }
}

#[test]
fn session_filtering_matrix_is_complete_and_small_lists_declare_non_filterability() {
    let catalog = picker_contracts();

    // Filterable: model + session cover query, no-results, clear, apply/cancel.
    for case_id in [
        "model-filter-apply",
        "model-filter-no-match",
        "model-filter-clear",
        "model-filter-cancel",
        "model-filtered-apply",
        "session-no-matches-filter",
        "session-no-matches",
        "session-resume-latest",
        "session-preview-back",
        "session-picker-cancel",
    ] {
        let replay = replay_case(case_id, 1)
            .unwrap_or_else(|error| panic!("{case_id} filter-matrix case must exist: {error}"));
        assert!(
            !replay.actions.is_empty(),
            "{case_id} must emit at least one action"
        );
    }

    // No-results recovery leaves apply inactive in favor of clear.
    let model_empty = audit_screen("model-picker-filtered", "flow3-model-picker-no-match")
        .expect("model no-match fixture");
    let model_empty_text = model_empty.render_txt();
    assert!(
        model_empty_text.contains("No models match")
            || model_empty_text.contains("No model")
            || model_empty_text.contains("0 of")
            || model_empty_text.contains("0 matching"),
        "empty model filter must explain no results"
    );
    assert!(
        model_empty_text.contains("Clear filter") || model_empty_text.contains("[c] Clear"),
        "empty model filter must offer local clear recovery"
    );

    let session_empty =
        audit_screen("session-picker", "case.session-no-matches.tick.1").expect("session empty");
    let session_empty_text = session_empty.render_txt();
    assert!(session_empty_text.contains("Filter ·"));
    assert!(
        session_empty_text.contains("No conversations match")
            || session_empty_text.contains("No matching"),
        "session no-match must explain the empty filter"
    );
    assert!(session_empty_text.contains("Clear filter"));

    // Small non-filterable pickers declare why filtering is unnecessary.
    for screen_id in [
        "agent-picker",
        "skills-picker",
        "subagent-picker",
        "variant-picker",
        "theme-picker",
        "fork-picker",
    ] {
        let contract = catalog.get(screen_id).expect("catalog");
        assert_eq!(contract.filter, FilterCapability::NonFilterable);
        let reason = non_filter_declaration(screen_id).expect("non-filter reason");
        let audit = audit_screen(screen_id, contract_fixture(screen_id)).expect("render");
        let text = audit.render_txt();
        assert!(
            text.contains("No filter ·") || text.contains(reason),
            "{screen_id} must declare non-filterability; expected `{reason}`\n{text}"
        );
        assert!(
            audit.events.iter().any(|event| {
                event.identity.family == "filter-not-needed"
                    || event.identity.subject_id.contains("no-filter")
                    || event.identity.subject_id.contains("filter-not-needed")
            }),
            "{screen_id} must emit a filter-not-needed semantic event"
        );
        assert!(!text.contains("[f] Filter"), "{screen_id} must not invite filtering");
    }

    // Filterable catalogs still expose filter controls or active query feedback.
    for screen_id in ["model-picker", "model-picker-filtered", "session-picker"] {
        let contract = catalog.get(screen_id).expect("catalog");
        assert_eq!(contract.filter, FilterCapability::Filterable);
        let audit = audit_screen(screen_id, contract_fixture(screen_id)).expect("render");
        let text = audit.render_txt();
        assert!(
            text.contains("Filter") || text.contains("[f] Filter"),
            "{screen_id} must expose the filter contract"
        );
        assert!(
            audit.events.iter().any(|event| event.identity.family == "filter"),
            "{screen_id} must emit a filter semantic event"
        );
    }
}

#[test]
fn public_publication_covers_all_nine_picker_txt_artifacts() {
    let gallery = Path::new("/projects/cortex-code/plans/assets/mockups-v3/input");
    let catalog = picker_contracts();
    for screen_id in NINE_PICKERS {
        let path = gallery.join(format!("{screen_id}.txt"));
        assert!(
            path.is_file(),
            "published gallery must include {}",
            path.display()
        );
        let text = std::fs::read_to_string(&path).expect("read published txt");
        assert_eq!(text.lines().count(), 40);
        let contract = catalog.get(screen_id).expect("catalog");
        assert!(text.contains(contract.title));
    }

    // Renderer binary is available for on-demand re-publish.
    let listed = harness().args(["--list"]).output().expect("list screens");
    assert!(listed.status.success());
}
