//! Focused coverage for model filtering and non-destructive session forking.
//! Owns VAL-V3-F3-015..017, VAL-V3-F3-030..031, and VAL-V3-F3-054.

use std::path::Path;
use std::process::Command;

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}



fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn model_picker_filtered_shows_active_query_counts_and_matching_rows() {
    let unfiltered = audit_screen("model-picker", "flow3-model-picker")
        .expect("unfiltered model picker must still render");
    unfiltered
        .validate()
        .expect("unfiltered model picker must stay bijective");
    let unfiltered_txt = unfiltered.render_txt();

    let filtered = audit_screen("model-picker-filtered", "flow3-model-picker-filtered")
        .expect("filtered model picker must produce typed evidence");
    filtered
        .validate()
        .expect("filtered model picker evidence must stay bijective");
    let text = filtered.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Choose a model"));
    assert!(text.contains("Filter · claude") || text.contains("query: claude"));
    assert!(
        text.contains("2 of 3") || text.contains("2 matches") || text.contains("matching"),
        "filtered picker must show match count feedback; got:\n{text}"
    );
    assert!(text.contains("Claude Sonnet 4.5"));
    assert!(text.contains("Claude Opus 4.8"));
    assert!(!text.contains("GPT-5.2 Codex"));
    assert!(text.contains("Current"));
    assert!(text.contains("› Claude Opus 4.8") || text.contains("Selected · Claude Opus 4.8"));
    assert!(text.contains("[enter] Apply model") || (text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    assert!(text.contains("[c] Clear filter") || text.contains("Clear filter"));
    assert!(!text.contains("anthropic/"));
    assert!(!text.contains("openai/"));
    assert!(!text.contains("ctx "));

    // Filtered state must not look identical to unfiltered.
    assert_ne!(unfiltered_txt, text);
    assert!(unfiltered_txt.contains("GPT-5.2 Codex"));
    assert!(unfiltered_txt.contains("Claude Sonnet 4.5"));
    assert!(unfiltered_txt.contains("Claude Opus 4.8"));

    let sonnet = text.find("Claude Sonnet 4.5").expect("sonnet remains");
    let opus = text.find("Claude Opus 4.8").expect("opus remains");
    assert!(
        sonnet < opus,
        "relative ordering of matching rows must stay stable"
    );

    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "filter"
            && (event.identity.subject_id.contains("claude")
                || event.identity.subject_id.contains("query"))
    }));
    assert_eq!(
        filtered
            .events
            .iter()
            .filter(|event| event.identity.family == "model-row")
            .count(),
        2,
        "only matching model rows remain"
    );
    assert_eq!(
        filtered
            .events
            .iter()
            .filter(|event| event.identity.family == "model-row"
                && event.identity.detail_level == "focused")
            .count(),
        1
    );
    assert_eq!(
        filtered
            .events
            .iter()
            .filter(|event| event.identity.family == "selected-detail")
            .count(),
        1
    );
    assert_eq!(
        filtered
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn model_filter_no_match_and_clear_recover_in_place() {
    let no_match = audit_screen("model-picker-filtered", "flow3-model-picker-no-match")
        .expect("no-match model filter must render");
    no_match
        .validate()
        .expect("no-match filter evidence must stay bijective");
    let text = no_match.render_txt();
    assert!(text.contains("Filter · zzq") || text.contains("query: zzq"));
    assert!(
        text.contains("No models match") || text.contains("no models match"),
        "zero-result copy must be plain English"
    );
    assert!(text.contains("Clear filter") || text.contains("[c] Clear filter"));
    // Matching rows must be gone; the statusline may still name the applied model.
    let body: String = text.lines().take(38).collect::<Vec<_>>().join("\n");
    assert!(!body.contains("Claude Opus 4.8"));
    assert!(!body.contains("GPT-5.2 Codex"));
    assert!(
        !body
            .lines()
            .any(|line| line.contains("› Claude") || line.contains("  Claude Sonnet")),
        "no model rows may remain selected in the empty filter body:\n{body}"
    );
    assert!(
        !no_match.events.iter().any(|event| {
            event.identity.family == "primary-action"
                && event.identity.subject_id == "action.model-picker.apply"
        }),
        "Apply must not stay primary when no valid row remains"
    );
    assert!(no_match.events.iter().any(|event| {
        event.identity.family == "empty-result" || event.identity.family == "no-match"
    }));

    let filtered = replay_case("model-filter-apply", 1).expect("filter apply must be public");
    assert_eq!(filtered.before.screen_id, "model-picker");
    assert_eq!(filtered.after.screen_id, "model-picker-filtered");
    assert_eq!(filtered.before.session_id, filtered.after.session_id);
    assert_eq!(filtered.before.draft, filtered.after.draft);
    assert_eq!(filtered.before.origin_stack, filtered.after.origin_stack);
    assert!(
        filtered
            .audit
            .render_txt()
            .contains("Claude Opus 4.8")
    );
    assert!(!filtered.audit.render_txt().contains("GPT-5.2 Codex"));

    let no_match_filter =
        replay_case("model-filter-no-match", 1).expect("no-match filter must be public");
    assert_eq!(no_match_filter.after.screen_id, "model-picker-filtered");
    assert!(
        no_match_filter
            .audit
            .render_txt()
            .contains("No models match")
            || no_match_filter
                .audit
                .render_txt()
                .contains("no models match")
    );

    let cleared = replay_case("model-filter-clear", 1).expect("clear filter must be public");
    assert_eq!(cleared.before.screen_id, "model-picker-filtered");
    assert_eq!(cleared.after.screen_id, "model-picker");
    assert_eq!(cleared.before.session_id, cleared.after.session_id);
    assert_eq!(cleared.before.draft, cleared.after.draft);
    assert_eq!(cleared.before.origin_stack, cleared.after.origin_stack);
    let restored = cleared.audit.render_txt();
    assert!(restored.contains("Claude Sonnet 4.5"));
    assert!(restored.contains("Claude Opus 4.8"));
    assert!(restored.contains("GPT-5.2 Codex"));
    assert_eq!(cleared.side_effects[0].status, "filter-cleared");

    let cancelled = replay_case("model-filter-cancel", 1).expect("filter cancel must be public");
    assert_eq!(cancelled.before.screen_id, "model-picker-filtered");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.before.session_id, cancelled.after.session_id);
    assert_eq!(cancelled.before.draft, cancelled.after.draft);
    assert_eq!(cancelled.side_effects[0].status, "no-change");
}

#[test]
fn fork_picker_states_source_checkpoint_and_destination_semantics() {
    let audit =
        audit_screen("fork-picker", "flow3-fork-picker").expect("fork picker must render");
    audit
        .validate()
        .expect("fork picker evidence must stay bijective");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Fork conversation") || text.contains("Start a new session from"));
    assert!(text.contains("Retry policy review"));
    assert!(text.contains("Today, 09:37") || text.contains("Updated the retry policy"));
    assert!(
        text.contains("new session")
            || text.contains("New conversation")
            || text.contains("destination"),
        "destination summary must be visible"
    );
    assert!(
        text.contains("leaves the source")
            || text.contains("source stays")
            || text.contains("source unchanged")
            || text.contains("without changing the source"),
        "plain non-destructive copy must be explicit"
    );
    assert!(
        text.contains("› Fork")
            || text.contains("Enter to fork")
            || text.contains("[enter] Fork")
            || text.contains("› [enter] Fork"),
        "fork picker must expose Fork as the primary (prose or residual bracket)"
    );
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")) || text.contains("Cancel"));
    for forbidden in ["rewinds history", "branch name", "export conversation"] {
        assert!(
            !text.to_ascii_lowercase().contains(forbidden),
            "fork must not confuse with resume/export/rewrite; saw forbidden phrasing: {forbidden}"
        );
    }
    assert!(
        !text.to_ascii_lowercase().contains("overwrite the original")
            || text.contains("never overwrites"),
        "overwrite language must only deny mutation"
    );
    assert!(!text.contains("session."));
    assert!(!text.contains("session_"));

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "source" || event.identity.family == "source-session"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "checkpoint" || event.identity.family == "selected-point"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "destination" || event.identity.family == "fork-destination"
    }));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.fork-picker.confirm"
            && transition.to_screen_id == "chat-turn"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.fork-picker.cancel"
            && transition.kind == "cancel"
    }));
}

#[test]
fn fork_confirm_creates_distinct_session_and_cancel_preserves_source() {
    let confirmed = replay_case("fork-confirm", 1).expect("fork confirm must replay");
    confirmed
        .validate()
        .expect("fork confirm must validate preservation evidence");
    assert_eq!(confirmed.before.screen_id, "fork-picker");
    assert_eq!(confirmed.after.screen_id, "chat-turn");
    assert_ne!(
        confirmed.before.session_id, confirmed.after.session_id,
        "fork must create a distinct session identity"
    );
    assert!(
        confirmed.after.session_id.contains("fork")
            || confirmed.after.session_id != confirmed.before.session_id
    );
    assert!(
        confirmed.after.transcript.contains("Updated the retry policy")
            || confirmed
                .after
                .transcript
                .contains("history through the selected checkpoint"),
        "fork transcript must only include history through the checkpoint"
    );
    assert!(
        !confirmed
            .after
            .transcript
            .contains("Current · Review ready")
            && !confirmed.after.transcript.contains("Tests passed with final"),
        "later source events must not leak into the fork"
    );
    assert_eq!(
        confirmed.preserved_field_hashes["source_history"].before,
        confirmed.preserved_field_hashes["source_history"].after,
        "source history hash must remain unchanged after fork"
    );
    assert_eq!(confirmed.side_effects[0].status, "session-forked");
    assert_eq!(confirmed.attempts[0].count, 1);

    let cancelled = replay_case("fork-cancel", 1).expect("fork cancel must replay");
    cancelled
        .validate()
        .expect("fork cancel must validate");
    assert_eq!(cancelled.before.screen_id, "fork-picker");
    assert!(
        cancelled.after.screen_id == "timeline" || cancelled.after.screen_id == "session-picker"
    );
    assert_eq!(cancelled.before.session_id, cancelled.after.session_id);
    assert_eq!(cancelled.before.draft, cancelled.after.draft);
    assert_eq!(
        cancelled.preserved_field_hashes["source_history"].before,
        cancelled.preserved_field_hashes["source_history"].after
    );
    assert_eq!(cancelled.side_effects[0].status, "no-change");
}

#[test]
fn historical_resume_always_forks_and_latest_resume_does_not() {
    let latest = replay_case("session-resume-latest", 1).expect("latest resume stays non-forking");
    assert_eq!(latest.before.screen_id, "session-picker");
    assert_eq!(latest.after.screen_id, "chat-turn");
    assert_eq!(latest.side_effects[0].status, "selected-session-resumed");
    assert!(
        !latest
            .transitions
            .iter()
            .any(|transition| transition.to_screen_id == "fork-picker")
    );

    let historical = replay_case("timeline-historical-fork", 1)
        .expect("historical timeline points must route to fork-picker");
    assert_eq!(historical.before.screen_id, "timeline");
    assert_eq!(historical.after.screen_id, "fork-picker");
    assert_eq!(historical.before.session_id, historical.after.session_id);
    assert_eq!(
        historical.preserved_field_hashes["source_history"].before,
        historical.preserved_field_hashes["source_history"].after
    );
    assert_eq!(historical.side_effects[0].status, "read-only");
    assert!(historical.audit.render_txt().contains("Fork"));
    assert!(
        !historical
            .audit
            .transitions
            .iter()
            .any(|transition| transition.kind == "destructive-rewind"
                || transition.path.contains("destructively rewinds"))
    );

    // Completing the fork must still leave the source history byte-stable and
    // produce a distinct destination session.
    let forked = replay_case("fork-confirm", 1).expect("historical confirmation must fork");
    assert_ne!(forked.before.session_id, forked.after.session_id);
    assert_eq!(
        forked.preserved_field_hashes["source_history"].before,
        forked.preserved_field_hashes["source_history"].after
    );
    assert!(!forked.transitions[0].path.contains("destructive"));
}

#[test]
fn model_filter_and_fork_publish_exact_selectors() {
    if pure_mode() {
        // Pure dump ownership: hybrid public publish selectors refuse on live roots.
        return;
    }

    for screen in ["model-picker-filtered", "fork-picker"] {
        let output = harness()
            .arg(screen)
            .output()
            .unwrap_or_else(|error| panic!("{screen} must publish: {error}"));
        assert!(
            output.status.success(),
            "{screen} publish failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for ext in ["txt", "png"] {
            let path = format!("/projects/cortex-code/plans/assets/mockups-v3/input/{screen}.{ext}");
            assert!(
                Path::new(&path).is_file(),
                "canonical {ext} must exist at {path}"
            );
        }
    }
}
