use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn slash_menu_opens_at_composer_with_one_selected_command_and_readable_consequences() {
    let audit = audit_screen("slash-menu", "flow2-slash-menu")
        .expect("slash-menu must produce typed render evidence");
    audit
        .validate()
        .expect("slash-menu evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("/ commands"));
    assert!(text.contains("query: /"));
    assert!(text.contains("matching"));
    assert!(text.contains("› /export"));
    assert!(text.contains("Export this conversation as a file"));
    assert!(text.contains("/help"));
    assert!(text.contains("Open the command reference"));
    assert!(text.contains("/queue"));
    assert!(text.contains("Manage queued prompts"));
    assert!(text.contains("/actions"));
    assert!(text.contains("Open the full action palette"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));
    // multiline origin remains above the dock
    assert!(text.contains("Paused draft · 3 authored lines"));
    assert!(text.contains("Keep the summary short before we merge."));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1,
        "slash-menu must preselect exactly one command"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "active-command-span"
            && event.identity.detail_level == "focused"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "origin"
            && event.identity.subject_id == "composer-multiline"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.slash-menu.confirm-selected"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.slash-menu.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn slash_filtering_is_live_with_match_count_and_explicit_no_results() {
    let filtered = audit_screen("slash-menu", "flow2-slash-filter-export")
        .expect("filtered slash menu must render");
    filtered
        .validate()
        .expect("filtered slash evidence must stay bijective");
    let filtered_txt = filtered.render_txt();
    assert!(filtered_txt.contains("query: /ex"));
    assert!(filtered_txt.contains("1 matching"));
    assert!(filtered_txt.contains("› /export"));
    assert!(!filtered_txt.contains("/help"));
    assert!(!filtered_txt.contains("/queue"));
    assert_eq!(
        filtered
            .events
            .iter()
            .filter(|event| event.identity.family == "command-match")
            .count(),
        1
    );
    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "filter" && event.identity.subject_id == "query-ex"
    }));

    let empty = audit_screen("slash-menu", "flow2-slash-no-results")
        .expect("no-results slash menu must render");
    empty
        .validate()
        .expect("no-results slash evidence must stay bijective");
    let empty_txt = empty.render_txt();
    assert!(empty_txt.contains("query: /zzq"));
    assert!(empty_txt.contains("No commands match /zzq."));
    assert!(empty_txt.contains("Paused draft"));
    assert!(empty.events.iter().any(|event| {
        event.identity.family == "empty-result"
            && event.identity.subject_id.contains("zzq")
    }));
}

#[test]
fn slash_confirm_and_cancel_restore_the_exact_multiline_origin() {
    let confirmed = replay_case("slash-confirm-export", 1)
        .expect("confirming a slash command must be replayable");
    let cancelled = replay_case("slash-cancel", 1).expect("cancelling slash must be replayable");
    let invalid = replay_case("slash-invalid-trigger", 1)
        .expect("invalid slash positions must stay on the composer");

    confirmed
        .validate()
        .expect("slash confirm replay must expose one valid transition");
    assert_eq!(confirmed.before.screen_id, "slash-menu");
    assert_eq!(confirmed.after.screen_id, "composer-multiline");
    assert_eq!(confirmed.before.origin_stack, confirmed.after.origin_stack);
    assert_eq!(
        confirmed.before.scroll_anchor, confirmed.after.scroll_anchor,
        "slash confirm must not move the transcript"
    );
    assert_eq!(
        confirmed.before.composer_height, confirmed.after.composer_height,
        "slash confirm must preserve composer height"
    );
    assert_ne!(
        confirmed.before.draft, confirmed.after.draft,
        "confirm must rewrite the active command span"
    );
    assert_eq!(
        confirmed
            .before
            .draft
            .replacen("/", "/export", 1),
        confirmed.after.draft,
        "only the active command span may change on confirm"
    );
    let surrounding = confirmed
        .preserved_field_hashes
        .get("draft_surrounding")
        .expect("confirm must hash unchanged draft surrounding text");
    assert_eq!(surrounding.before, surrounding.after);

    cancelled
        .validate()
        .expect("slash cancel replay must expose one valid transition");
    assert_eq!(cancelled.before.screen_id, "slash-menu");
    assert_eq!(cancelled.after.screen_id, "composer-multiline");
    for field in [
        "draft",
        "selection",
        "focus",
        "composer_height",
        "scroll_anchor",
    ] {
        let hashes = cancelled
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "cancel must byte-restore {field} from before / opened"
        );
    }
    assert_eq!(
        cancelled.after.draft, cancelled.before.draft,
        "cancel restores the pre-menu draft snapshot carried on the origin state"
    );

    invalid
        .validate()
        .expect("invalid trigger case must remain valid");
    assert_eq!(invalid.before.screen_id, "composer-multiline");
    assert_eq!(invalid.after.screen_id, "composer-multiline");
    assert_eq!(invalid.before.draft, invalid.after.draft);
    assert!(
        invalid.before.draft.contains("https://example.com/path"),
        "invalid trigger uses an embedded URL slash"
    );
    assert_eq!(invalid.after.selection, "no-menu");
}

#[test]
fn command_palette_opens_without_mutating_draft_and_discloses_effects() {
    let audit = audit_screen("command-palette", "flow2-command-palette")
        .expect("command-palette must produce typed render evidence");
    audit
        .validate()
        .expect("command-palette evidence must stay bijective");

    let text = audit.render_txt();
    assert!(text.contains("Filter actions"));
    assert!(text.contains("query:"));
    assert!(text.contains("matching"));
    assert!(text.contains("› Recall an older prompt"));
    assert!(text.contains("put an earlier prompt in the draft"));
    assert!(text.contains("Stash this draft"));
    assert!(text.contains("save it without sending"));
    assert!(text.contains("Restore stashed draft"));
    assert!(text.contains("Queued prompts"));
    assert!(text.contains("▸"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    assert!((text.contains("Paused conversation") || text.contains("Input and approval · paused conversation")));
    assert!(text.contains("Check the retry path before we merge."));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "submenu-affordance"
            && event.identity.subject_id.contains("queued")
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.command-palette.open-submenu"
            && transition.to_screen_id == "command-palette-submenu"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.command-palette.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn command_palette_filtering_is_deterministic() {
    let filtered = audit_screen("command-palette", "flow2-palette-filter-queue")
        .expect("filtered palette must render");
    filtered.validate().expect("filter evidence bijective");
    let text = filtered.render_txt();
    assert!(text.contains("query: queue"));
    assert!(text.contains("1 matching"));
    assert!(text.contains("› Queued prompts"));
    assert!(!text.contains("Recall an older prompt"));
    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "filter" && event.identity.subject_id == "query-queue"
    }));

    let empty = audit_screen("command-palette", "flow2-palette-no-results")
        .expect("no-results palette must render");
    empty.validate().expect("no-results bijective");
    let empty_txt = empty.render_txt();
    assert!(empty_txt.contains("query: zzq"));
    assert!(empty_txt.contains("No actions match zzq."));
}

#[test]
fn command_palette_submenu_has_explicit_scope_and_local_back() {
    let audit = audit_screen("command-palette-submenu", "flow2-palette-submenu")
        .expect("submenu must produce typed render evidence");
    audit.validate().expect("submenu evidence bijective");

    let text = audit.render_txt();
    assert!(text.contains("Palette › Queued prompts"));
    assert!((text.contains("Root") && (text.contains("Esc to root") || text.contains("back") || text.contains("Root"))));
    assert!(text.contains("› Add buffer cap tests"));
    assert!(text.contains("run this prompt next"));
    assert!(text.contains("Trace the retry backoff"));
    assert!(text.contains("Document the rate-limit path"));
    assert!(!text.contains("Recall an older prompt"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "scope"
            && event.identity.subject_id == "queued-prompts"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.command-palette-submenu.back"
            && transition.to_screen_id == "command-palette"
            && transition.kind == "cancel"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.command-palette-submenu.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));

    let root = audit_screen("command-palette", "flow2-command-palette")
        .expect("root palette for color-independent comparison");
    let root_plain = root
        .render_txt()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let sub_plain = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(
        root_plain, sub_plain,
        "submenu must remain distinguishable from root without color"
    );
}

#[test]
fn palette_submenu_back_and_cancel_are_safe() {
    let back = replay_case("palette-submenu-back", 1).expect("submenu back must replay");
    let cancel_root = replay_case("palette-cancel", 1).expect("palette cancel must replay");
    let cancel_sub =
        replay_case("palette-submenu-cancel", 1).expect("submenu cancel must replay");
    let open_sub = replay_case("palette-open-submenu", 1).expect("opening submenu must replay");

    back.validate().expect("back transition valid");
    assert_eq!(back.before.screen_id, "command-palette-submenu");
    assert_eq!(back.after.screen_id, "command-palette");
    assert_eq!(
        back.before.draft, back.after.draft,
        "back must not mutate the original multiline draft"
    );
    assert_eq!(
        back.after.selection, "root-action-queued-prompts",
        "back must restore the root selected row"
    );
    assert_eq!(
        back.after.focus, "root-query-",
        "back must restore the root query focus"
    );
    for field in ["draft", "composer_height", "scroll_anchor"] {
        let hashes = back.preserved_field_hashes.get(field).expect(field);
        assert_eq!(hashes.before, hashes.after, "back preserves {field}");
    }

    for replay in [&cancel_root, &cancel_sub] {
        replay.validate().expect("palette cancel must validate");
        assert_eq!(replay.after.screen_id, "composer-multiline");
        assert_eq!(
            replay.before.draft, replay.after.draft,
            "cancel must leave the original multiline draft byte-identical"
        );
        assert_eq!(replay.before.scroll_anchor, replay.after.scroll_anchor);
        assert_eq!(replay.before.composer_height, replay.after.composer_height);
        assert_eq!(replay.before.focus, replay.after.focus);
        for field in [
            "draft",
            "focus",
            "composer_height",
            "scroll_anchor",
            "selection",
        ] {
            let hashes = replay
                .preserved_field_hashes
                .get(field)
                .unwrap_or_else(|| panic!("{field} required"));
            assert_eq!(hashes.before, hashes.after, "cancel preserves {field}");
        }
    }

    open_sub.validate().expect("open submenu valid");
    assert_eq!(open_sub.before.screen_id, "command-palette");
    assert_eq!(open_sub.after.screen_id, "command-palette-submenu");
    assert_eq!(open_sub.before.draft, open_sub.after.draft);
}

#[test]
fn public_selectors_publish_the_three_slash_and_palette_pairs() {
    for screen in [
        "slash-menu",
        "command-palette",
        "command-palette-submenu",
    ] {
        let output = harness()
            .arg(screen)
            .output()
            .expect("public canonical selector must run");
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        assert!(output.status.success(), "{screen} must publish after renderer preflight:\n{}", String::from_utf8_lossy(&output.stderr));
    }
    }
    }
    }
        for extension in ["txt", "png"] {
            assert!(
                Path::new(&format!(
                    "/projects/cortex-code/plans/assets/mockups-v3/input/{screen}.{extension}"
                ))
                .is_file(),
                "canonical {screen}.{extension} must publish"
            );
        }
    }
}
