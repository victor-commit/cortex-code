use std::fs;
use std::process::Command;

use mockup_harness_v3::{
    audit::{audit_screen, CANVAS_CELL_COUNT},
    scenario::replay_case,
};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn typed_audit_is_stable_bijective_and_includes_styled_cells() {
    let first = audit_screen("permission-shell", "foundation-permission")
        .expect("the foundation fixture must render an audit document");
    let second = audit_screen("permission-shell", "foundation-permission")
        .expect("the same fixture must be repeatable");

    first
        .validate()
        .expect("candidate/event/action evidence must be fail-closed and complete");
    assert_eq!(first, second);
    assert_eq!(first.styled_cells.len(), CANVAS_CELL_COUNT);
    assert!(first.events.iter().all(|event| {
        !event.identity.screen_id.is_empty()
            && !event.identity.fixture_id.is_empty()
            && !event.identity.candidate_id.is_empty()
            && !event.identity.event_id.is_empty()
            && !event.identity.family.is_empty()
            && !event.identity.subject_id.is_empty()
            && !event.identity.render_refs.is_empty()
    }));
    assert!(first.events.iter().any(|event| event
        .action
        .as_ref()
        .is_some_and(|action| !action.action_id.is_empty())));
    assert!(
        first
            .render_txt()
            .lines()
            .nth(33)
            .is_some_and(|line| line.contains("[enter] Allow once")),
        "permission rendering must use the mandatory bottom DecisionDock"
    );
    assert_eq!(first.to_json(), second.to_json());
}

#[test]
fn replay_tracks_origin_hashes_ticks_transitions_attempts_and_effects() {
    let replay = replay_case("permission-allow-once", 1)
        .expect("the named permission case must replay at its allow tick");

    replay
        .validate()
        .expect("the deterministic replay trace must remain internally consistent");
    assert_eq!(replay.case_id, "permission-allow-once");
    assert_eq!(replay.virtual_tick, 1);
    assert_eq!(replay.before.screen_id, "permission-shell");
    assert_eq!(replay.after.screen_id, "diff-edit");
    assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
    assert_eq!(
        replay
            .preserved_field_hashes
            .get("draft")
            .expect("draft preservation must be hashed")
            .before,
        replay
            .preserved_field_hashes
            .get("draft")
            .expect("draft preservation must be hashed")
            .after
    );
    assert_eq!(replay.actions[0].action_id, "action.permission.allow-once");
    assert_eq!(
        replay.transitions[0].action_id,
        "action.permission.allow-once"
    );
    assert_eq!(replay.attempts[0].count, 1);
    assert_eq!(replay.side_effects[0].effect_id, "effect.shell.pending");
    assert_eq!(
        replay.to_json(),
        replay_case("permission-allow-once", 1).unwrap().to_json()
    );
}

#[test]
fn automatic_case_uses_a_stable_automatic_action_and_single_attempt() {
    let replay = replay_case("automatic-retry", 1)
        .expect("the deterministic automatic retry case must replay");

    assert_eq!(replay.before.screen_id, "rate-limit");
    assert_eq!(replay.after.screen_id, "chat-turn");
    assert_eq!(replay.actions[0].action_id, "action.recovery.retry-window");
    assert!(replay.actions[0].automatic);
    assert_eq!(replay.attempts[0].count, 1);
    assert_eq!(replay.side_effects.len(), 1);
}

#[test]
fn conversation_pilot_screens_keep_the_transcript_and_one_clear_next_step() {
    let home = audit_screen("home", "conversation-home")
        .expect("the home conversation shell must produce typed evidence");
    let completed = audit_screen("chat-turn", "conversation-completed")
        .expect("the completed conversation must produce typed evidence");
    let streaming = audit_screen("streaming", "conversation-streaming")
        .expect("the live conversation must produce typed evidence");

    for audit in [&home, &completed, &streaming] {
        audit
            .validate()
            .expect("each conversation screen must remain semantically bijective");
        let rendered = audit.render_txt();
        let rows = rendered.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 40);
        assert!(
            rows[39].contains("~/project · main · Claude Opus 4.8"),
            "the shared minimal statusline must stay on the canvas"
        );
    }

    let home_text = home.render_txt();
    assert!(home_text.contains("Recent work"));
    assert!(home_text.contains("Ask Code to continue"));
    assert_eq!(
        home.transitions[0].to_screen_id, "chat-turn",
        "home must begin the resumed conversation"
    );

    let completed_text = completed.render_txt();
    let request = completed_text
        .find("Why are stale sessions rejected")
        .expect("the existing request must remain visible");
    let answer = completed_text
        .find("A password change invalidates")
        .expect("the completed answer must remain visible");
    assert!(request < answer, "the request must precede the readable answer");
    assert!(completed_text.contains("Reply to Code"));
    assert_eq!(completed.transitions[0].to_screen_id, "streaming");

    let streaming_text = streaming.render_txt();
    assert!(streaming_text.contains("The answer is still being written"));
    assert!(streaming_text.contains("will wait until this reply finishes"));
    assert_eq!(
        streaming
            .events
            .iter()
            .filter(|event| event.identity.family == "live-state")
            .count(),
        1,
        "a live response must have exactly one semantic owner"
    );
    assert_eq!(
        streaming
            .events
            .iter()
            .filter(|event| event.action.is_some())
            .count(),
        1,
        "only the local interrupt should be actionable while streaming"
    );
    assert!(streaming.transitions.iter().any(|transition| {
        transition.action_id == "action.streaming.interrupt"
            && transition.to_screen_id == "chat-turn"
    }));
}

#[test]
fn streaming_interrupt_preserves_partial_answer_and_queued_follow_up() {
    let replay = replay_case("streaming-interrupt", 1)
        .expect("the local streaming interrupt case must replay");

    replay
        .validate()
        .expect("the interrupt must be one stable, reversible transition");
    assert_eq!(replay.before.screen_id, "streaming");
    assert_eq!(replay.after.screen_id, "chat-turn");
    assert_eq!(replay.actions[0].action_id, "action.streaming.interrupt");
    assert_eq!(replay.attempts[0].count, 1);
    assert_eq!(replay.side_effects[0].status, "generation-stopped");
    for field in ["draft", "transcript", "queued_follow_up"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} preservation must be recorded"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive interruption");
    }
    assert!(replay.audit.render_txt().contains("Reply to Code"));
    let interrupted = replay.audit.render_txt();
    assert!(interrupted.contains("It keeps sign-in, refresh, and sign-out on the same safety rule."));
    assert!(interrupted.contains("Reply stopped safely. Partial answer and draft are saved."));
}

#[test]
fn audit_json_is_json_only_on_stdout() {
    let output = harness()
        .arg("--audit-json")
        .output()
        .expect("the audit CLI must run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("audit output must be UTF-8");
    assert!(stdout.starts_with("{\"document_type\":\"typed-render-audit\""));
    assert!(stdout.contains("\"styled_cells\""));
    assert!(stdout.contains("\"candidate_id\""));
    assert!(stdout.contains("\"event_id\""));
    assert!(stdout.contains("\"action_id\""));
}

#[test]
fn case_audit_and_txt_are_stdout_only_and_canonical_sized() {
    let audit = harness()
        .args([
            "--case",
            "permission-allow-once",
            "--tick",
            "1",
            "--audit-json",
        ])
        .output()
        .expect("the case audit CLI must run");
    assert!(audit.status.success());
    assert!(audit.stderr.is_empty());
    let audit_stdout = String::from_utf8(audit.stdout).expect("case audit must be UTF-8");
    assert!(audit_stdout.starts_with("{\"document_type\":\"case-replay\""));
    assert!(audit_stdout.contains("\"preserved_field_hashes\""));
    assert!(audit_stdout.contains("\"side_effects\""));

    let text = harness()
        .args(["--case", "permission-allow-once", "--tick", "1", "--txt"])
        .output()
        .expect("the case TXT CLI must run");
    assert!(text.status.success());
    assert!(text.stderr.is_empty());
    let text_stdout = String::from_utf8(text.stdout).expect("case TXT must be UTF-8");
    let rows = text_stdout.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
}

#[cfg(feature = "png")]
#[test]
fn case_png_requires_an_explicit_temporary_destination_and_reports_its_hash() {
    let path = std::env::temp_dir().join(format!(
        "mockup-harness-v3-audit-scenario-{}.png",
        std::process::id()
    ));
    assert!(!path.exists(), "test destination must not pre-exist");

    let output = harness()
        .args(["--case", "permission-allow-once", "--tick", "1", "--png"])
        .arg(&path)
        .output()
        .expect("the case PNG CLI must run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let hash = String::from_utf8(output.stdout).expect("PNG hash must be UTF-8");
    assert_eq!(hash.trim().len(), 64);
    assert!(hash
        .trim()
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert!(path.is_file());
    let png = image::open(&path).expect("scenario output must be a valid PNG");
    assert_eq!(png.width(), 1200);
    assert_eq!(png.height(), 840);
    fs::remove_file(&path).expect("the test must clean up its own temporary PNG");
}

#[cfg(feature = "png")]
#[test]
fn case_png_rejects_canonical_and_gate_destinations_without_writing() {
    for destination in [
        "/projects/cortex-code/plans/assets/mockups-v3/forbidden-scenario.png",
        "/projects/cortex-code/plans/assets/mockups-v3-gates/forbidden-scenario.png",
    ] {
        let path = std::path::Path::new(destination);
        assert!(!path.exists(), "test destination must not pre-exist");
        let output = harness()
            .args([
                "--case",
                "permission-allow-once",
                "--tick",
                "1",
                "--png",
                destination,
            ])
            .output()
            .expect("the scenario PNG CLI must report invalid destinations");

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!path.exists());
        assert!(String::from_utf8_lossy(&output.stderr).contains("temporary"));
    }
}

#[cfg(feature = "png")]
#[test]
fn synthetic_elbow_rasterization_is_stable_and_non_empty_at_txt_and_png_scales() {
    let mut audit = audit_screen("permission-shell", "synthetic-elbow")
        .expect("the elbow fixture must produce a valid canonical audit");
    let cell = audit
        .styled_cells
        .iter_mut()
        .find(|cell| cell.row == 12 && cell.column == 10)
        .expect("the canonical audit must have a target cell");
    cell.glyph = '⎿';
    audit
        .validate()
        .expect("the approved elbow glyph must remain valid audit content");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert_eq!(rows[12].chars().nth(10), Some('⎿'));

    let first = mockup_harness_v3::scenario::render_audit_png(&audit)
        .expect("the synthetic elbow must rasterize for canonical PNG output");
    let second = mockup_harness_v3::scenario::render_audit_png(&audit)
        .expect("the synthetic elbow must rasterize deterministically");
    assert_eq!(first, second);

    let png = image::load_from_memory(&first)
        .expect("the elbow raster must encode as a valid PNG")
        .to_rgb8();
    assert_eq!(png.dimensions(), (1200, 840));
    let canvas = png.get_pixel(0, 0);
    let non_canvas_pixels = (12 * 21 + 1..=12 * 21 + 14)
        .flat_map(|y| (10 * 10 + 4..=10 * 10 + 5).map(move |x| (x, y)))
        .chain((10 * 10 + 4..10 * 10 + 10).flat_map(|x| {
            [12 * 21 + 13, 12 * 21 + 14]
                .into_iter()
                .map(move |y| (x, y))
        }))
        .filter(|(x, y)| png.get_pixel(*x, *y) != canvas)
        .count();
    assert!(
        non_canvas_pixels > 0,
        "the deterministic synthetic elbow must write visible non-canvas pixels"
    );
}

#[test]
fn invalid_case_or_tick_is_a_usage_error_without_output() {
    for arguments in [
        vec!["--case", "unknown-case", "--tick", "1", "--audit-json"],
        vec![
            "--case",
            "permission-allow-once",
            "--tick",
            "99",
            "--audit-json",
        ],
    ] {
        let output = harness()
            .args(arguments)
            .output()
            .expect("the case CLI must report a usage failure");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}
