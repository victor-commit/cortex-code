use std::{path::Path, process::Command};

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn rich_response_keeps_prose_table_code_and_actions_in_the_conversation() {
    let audit = audit_screen("rich-response", "flow1-rich-response")
        .expect("the rich response must render typed evidence");
    audit
        .validate()
        .expect("the rich response must remain semantically bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert!(text.contains("The shared request guard applies the limit before route work begins."));
    assert!(text.contains("Where"));
    assert!(text.contains("How the guard applies it"));
    assert!(text.contains("Login route"));
    assert!(text.contains("API routes"));
    assert!(text.contains("fn authorize_request"));
    assert!(text.contains("shared_guard(request)?;"));
    assert!(text.contains("[c] Copy code"));
    assert!(text.contains("Reply to Code"));
    assert!(!text.contains("Open inspector"));
    assert!(!text.contains("? help"));

    let local_copy = audit
        .events
        .iter()
        .find(|event| {
            event
                .action
                .as_ref()
                .is_some_and(|action| action.action_id == "action.rich-response.copy-code")
        })
        .expect("copy must be a local semantic action");
    assert!(
        local_copy
            .identity
            .render_refs
            .iter()
            .all(|reference| reference.row < 30),
        "copy must stay beside the code rather than in global chrome"
    );
    assert!(
        audit.events.iter().any(|event| {
            event.action.as_ref().is_some_and(|action| {
                action.action_id == "action.rich-response.continue" && action.label == "Send reply"
            })
        }),
        "continuing the conversation must remain the primary composer action"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.action_id == "action.rich-response.continue"
                && transition.from_screen_id == "rich-response"
                && transition.to_screen_id == "chat-turn"
                && transition.kind == "direct"
        }),
        "the completed answer must continue in the same conversation"
    );
}

#[test]
fn daily_work_storyboard_reconciles_all_six_manifest_transitions() {
    let flow = flow_manifests()
        .iter()
        .find(|flow| flow.name == "daily-work")
        .expect("daily work must be a public flow");
    let expected = [
        ("home", "chat-turn", "direct"),
        ("chat-turn", "streaming", "direct"),
        ("streaming", "tool-running", "automatic"),
        ("tool-running", "tool-success", "automatic"),
        ("tool-success", "rich-response", "direct"),
        ("rich-response", "chat-turn", "direct"),
    ];
    assert_eq!(flow.steps.len(), expected.len());

    for (step, (screen, destination, kind)) in flow.steps.iter().zip(expected) {
        assert_eq!(step.screen_id, screen);
        assert_eq!(step.expected_next_frame, destination);
        assert_eq!(
            step.transition_kind,
            match kind {
                "direct" => TransitionKind::Direct,
                "automatic" => TransitionKind::Automatic,
                _ => unreachable!("the fixture only uses public transition kinds"),
            }
        );
        assert!(!step.user_action.is_empty());
        assert!(!step.transition_path.is_empty());
        assert!(!step.origin.is_empty());
        assert!(!step.success_return.is_empty());
        assert!(!step.cancel_return.is_empty());
        assert!(!step.acceptance_checks.is_empty());

        let audit = audit_screen(screen, &format!("flow1-storyboard.{screen}"))
            .unwrap_or_else(|error| panic!("{screen} must render: {error}"));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{screen} must validate: {error}"));
        assert_eq!(
            audit
                .transitions
                .iter()
                .filter(|transition| {
                    transition.from_screen_id == screen
                        && transition.to_screen_id == destination
                        && transition.kind == kind
                })
                .count(),
            1,
            "{screen} must have exactly one declared storyboard transition to {destination}"
        );
    }

    let streaming = audit_screen("streaming", "flow1-storyboard.streaming-automatic")
        .expect("streaming must render typed evidence");
    assert!(
        streaming
            .render_txt()
            .contains("When this reply is complete, Code will automatically search project files."),
        "the automatic tool handoff must be visible before the next state"
    );

    let output = harness()
        .args(["--flow", "daily-work"])
        .output()
        .expect("the public daily-work selector must run");
    assert!(
        output.status.success(),
        "daily work must publish only after all six canonical screens preflight:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = String::from_utf8(output.stdout).expect("flow metadata must be UTF-8");
    assert!(manifest.contains("\"flow\":\"daily-work\""));
    assert!(manifest.contains("\"screen_id\":\"rich-response\""));
    assert!(manifest.contains("\"transition_kind\":\"automatic\""));

    for (index, (screen, _, _)) in expected.iter().enumerate() {
        let number = index + 1;
        for extension in ["txt", "png"] {
            assert!(
                Path::new(&format!(
                    "/projects/cortex-code/plans/assets/mockups-v3/session/{screen}.{extension}"
                ))
                .is_file(),
                "canonical {screen}.{extension} must publish"
            );
            assert!(
                Path::new(&format!(
                    "/projects/cortex-code/plans/assets/mockups-v3-gates/daily-work/{number:02}-{screen}.{extension}"
                ))
                .is_file(),
                "gate frame {number:02}-{screen}.{extension} must publish"
            );
        }
    }
}

#[test]
fn tool_success_continuation_preserves_the_daily_work_origin() {
    let replay = replay_case("tool-success-continue", 1)
        .expect("the rich response continuation must be replayable");
    replay
        .validate()
        .expect("the continuation trace must have exact typed evidence");
    assert_eq!(replay.before.screen_id, "tool-success");
    assert_eq!(replay.after.screen_id, "rich-response");
    assert_eq!(replay.actions[0].action_id, "action.tool-success.continue");
    assert_eq!(replay.transitions[0].to_screen_id, "rich-response");
    for field in ["draft", "queued_follow_up", "scroll_anchor", "origin_stack"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} preservation must be recorded"));
        assert_eq!(
            hashes.before, hashes.after,
            "{field} must survive the final answer transition"
        );
    }
    assert!(replay.audit.render_txt().contains("fn authorize_request"));
}

#[test]
fn chat_turn_uses_plain_file_copy_and_fills_the_transcript_measure() {
    let audit = audit_screen("chat-turn", "canonical.chat-turn")
        .expect("the canonical chat turn must render typed evidence");
    audit
        .validate()
        .expect("the canonical chat turn must remain semantically bijective");

    let text = audit.render_txt();
    assert!(text.contains("retry_policy.rs"));
    assert!(text.matches("retry_policy.rs").count() >= 2);
    assert!(!text.contains("src/net/retry_policy.rs"));
    assert!(!text.contains("hunk"));
    assert!(!text.contains("patch revision"));
    assert!(text.contains("Updated retry_policy.rs."));
    assert!(text.contains("The retry check now verifies the current session before continuing."));
    assert!(text.contains("The request is complete; a new draft stays separate."));
    assert!(text.contains("Reply to Code"));
}

#[cfg(feature = "png")]
#[test]
fn chat_turn_png_raster_keeps_filename_underscores_at_the_baseline() {
    let audit = audit_screen("chat-turn", "canonical.chat-turn")
        .expect("the canonical chat turn must render typed evidence");
    let png = mockup_harness_v3::scenario::render_audit_png(&audit)
        .expect("the canonical chat turn must rasterize");
    let image = image::load_from_memory(&png)
        .expect("the canonical chat turn PNG must decode")
        .to_rgb8();
    let background = image.get_pixel(0, 0).0;
    let underscores = audit
        .styled_cells
        .iter()
        .filter(|cell| cell.glyph == '_')
        .collect::<Vec<_>>();
    assert_eq!(underscores.len(), 4);
    for cell in underscores {
        let x = u32::from(cell.column) * 10;
        let y = u32::from(cell.row) * 21;
        assert!(
            (y + 17..=y + 18).any(|row| {
                (x + 1..x + 9).any(|column| image.get_pixel(column, row).0 != background)
            }),
            "underscore at row {}, column {} must render on the baseline",
            cell.row,
            cell.column
        );
    }
}
