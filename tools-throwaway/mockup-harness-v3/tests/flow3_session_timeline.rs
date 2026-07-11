use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

#[test]
fn session_rows_and_preview_identify_readable_resume_destinations() {
    let audit = audit_screen("session-picker", "flow3-session-picker")
        .expect("session picker must render typed evidence");
    audit
        .validate()
        .expect("session picker audit must be bijective");

    let text = audit.render_txt();
    assert!(text.contains("Resume a conversation"));
    assert!(text.contains("Retry policy review"));
    assert!(text.contains("~/project"));
    assert!(text.contains("Today"));
    assert!(text.contains("Current"));
    assert!(text.contains("Latest stable work"));
    assert!(text.contains("Tests pass; review is ready"));
    assert!(text.contains("Your saved draft"));
    assert!(text.contains("[enter] Resume latest"));
    assert!(text.contains("[t] View timeline"));
    assert!(text.contains("[f] Fork instead"));
    for forbidden in ["session.", "session_", "database key", "transcript store"] {
        assert!(
            !text.contains(forbidden),
            "session picker must not expose raw identifier-led copy: {forbidden}"
        );
    }

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "session-row")
            .count(),
        3
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selected-detail")
            .count(),
        1
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn session_empty_and_no_match_states_recover_locally() {
    let no_sessions = audit_screen("session-picker", "case.session-no-sessions.tick.1")
        .expect("no-session fixture must render");
    let no_sessions_text = no_sessions.render_txt();
    assert!(no_sessions_text.contains("No saved conversations yet"));
    assert!(no_sessions_text.contains("[enter] Start a conversation"));
    assert!(!no_sessions_text.contains("Clear filter"));

    let no_matches = audit_screen("session-picker", "case.session-no-matches.tick.1")
        .expect("no-match fixture must render");
    let no_matches_text = no_matches.render_txt();
    assert!(no_matches_text.contains("Filter · billing"));
    assert!(no_matches_text.contains("No conversations match"));
    assert!(no_matches_text.contains("[enter] Clear filter"));
    assert!(!no_matches_text.contains("Start a conversation"));

    for audit in [&no_sessions, &no_matches] {
        assert_eq!(
            audit
                .events
                .iter()
                .filter(|event| event.identity.family == "primary-action")
                .count(),
            1
        );
    }
}

#[test]
fn timeline_is_chronological_focused_and_reversible() {
    let audit =
        audit_screen("timeline", "flow3-timeline").expect("timeline must render typed evidence");
    audit.validate().expect("timeline audit must be bijective");

    let text = audit.render_txt();
    assert!(text.contains("Retry policy review"));
    assert!(text.contains("Oldest first"));
    let opened = text.find("Opened the retry review").unwrap();
    let edited = text.find("Updated the retry policy").unwrap();
    let tested = text.find("Tests passed").unwrap();
    let current = text.find("Current · Review ready").unwrap();
    assert!(opened < edited && edited < tested && tested < current);
    assert!(text.contains("Fork available"));
    assert!(text.contains("[enter] Resume latest"));
    assert!(text.contains("[f] Fork from selected point"));
    assert!(text.contains("[esc] Back to session preview"));
    assert!(!text.contains("event_type"));
    assert!(!text.contains("payload"));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "timeline-event")
            .count(),
        4
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.timeline.back"
            && transition.to_screen_id == "session-picker"
    }));
}

#[test]
fn session_preview_timeline_back_and_resume_preserve_the_exact_origin() {
    let preview =
        replay_case("session-preview-open", 1).expect("session preview must replay safely");
    assert_eq!(preview.before.screen_id, "session-picker");
    assert_eq!(preview.after.screen_id, "session-picker");
    assert_eq!(preview.before.session_id, preview.after.session_id);
    assert_eq!(preview.before.transcript, preview.after.transcript);
    assert_eq!(preview.before.draft, preview.after.draft);
    let other_preview = replay_case("session-selected-docs-preview", 1)
        .expect("a second selected preview must replay safely");
    assert_eq!(other_preview.after.selection, "document-release-steps");
    assert!(
        other_preview
            .audit
            .render_txt()
            .contains("Selected · Document release steps")
    );

    let preview_back =
        replay_case("session-preview-back", 1).expect("preview Back must replay safely");
    assert_eq!(preview_back.after.screen_id, "session-picker");
    assert_eq!(
        preview_back.before.session_id,
        preview_back.after.session_id
    );
    assert_eq!(
        preview_back.before.scroll_anchor,
        preview_back.after.scroll_anchor
    );

    let timeline_open = replay_case("session-timeline-open", 1).expect("timeline open must replay");
    assert_eq!(timeline_open.after.screen_id, "timeline");
    let timeline_back = replay_case("session-timeline-back", 1).expect("timeline Back must replay");
    assert_eq!(timeline_back.after.screen_id, "session-picker");
    assert_eq!(
        timeline_back.before.selection,
        timeline_back.after.selection
    );
    assert_eq!(timeline_back.before.draft, timeline_back.after.draft);

    let resumed = replay_case("session-resume-latest", 1).expect("latest resume must replay");
    assert_eq!(resumed.before.screen_id, "session-picker");
    assert_eq!(resumed.after.screen_id, "chat-turn");
    assert_ne!(resumed.before.session_id, resumed.after.session_id);
    assert_eq!(resumed.after.work_style, "Careful edits");
    assert!(resumed.after.transcript.contains("Retry policy review"));
    assert!(resumed.after.draft.contains("Add the final review note"));
    assert_eq!(resumed.attempts[0].count, 1);
    assert_eq!(resumed.side_effects[0].status, "selected-session-resumed");
}

#[test]
fn no_session_and_no_match_actions_preserve_origin_state() {
    let no_sessions_open =
        replay_case("session-no-sessions-open", 1).expect("no-session state must open locally");
    assert_eq!(no_sessions_open.after.screen_id, "session-picker");
    assert!(
        no_sessions_open
            .audit
            .render_txt()
            .contains("No saved conversations yet")
    );

    let no_sessions =
        replay_case("session-no-sessions", 1).expect("no-session recovery must replay");
    assert_eq!(no_sessions.after.screen_id, "empty");
    assert_eq!(no_sessions.before.draft, no_sessions.after.draft);
    assert_eq!(
        no_sessions.before.origin_stack,
        no_sessions.after.origin_stack
    );

    let no_matches_filtered =
        replay_case("session-no-matches-filter", 1).expect("no-match filter must replay");
    assert!(
        no_matches_filtered
            .audit
            .render_txt()
            .contains("No conversations match")
    );
    let no_matches = replay_case("session-no-matches", 1).expect("no-match recovery must replay");
    assert_eq!(no_matches.after.screen_id, "session-picker");
    assert_eq!(no_matches.before.session_id, no_matches.after.session_id);
    assert_eq!(no_matches.before.draft, no_matches.after.draft);
    assert_eq!(no_matches.after.selection, "retry-policy-review");
    assert_eq!(no_matches.attempts[0].count, 0);
}
