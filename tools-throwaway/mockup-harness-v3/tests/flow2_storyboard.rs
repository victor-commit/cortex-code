use std::{fs, path::Path, process::Command};

use mockup_harness_v3::{
    audit::{audit_screen, sha256_hex},
    flow_manifests,
    scenario::replay_case,
    TransitionKind,
};

const SUBMITTED_PROMPT: &str = "Check the retry path in @src/net/retry_policy.rs\n\
Then explain the failure path that still needs coverage before the merge; include a concise test plan with the visible recovery behavior.\n\
Keep the summary short before we merge.";

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn input_and_approval_storyboard_declares_every_ordered_edge_and_return() {
    let flow = flow_manifests()
        .iter()
        .find(|flow| flow.name == "input-and-approval")
        .expect("input and approval must be a public flow");
    let expected = [
        (
            "composer-multiline",
            "file-mention",
            TransitionKind::Direct,
            "direct",
            "multiline-draft-is-preserved",
        ),
        (
            "file-mention",
            "permission-shell",
            TransitionKind::Direct,
            "direct",
            "submitted-prompt-is-immutable",
        ),
        (
            "permission-shell",
            "diff-edit",
            TransitionKind::Direct,
            "direct",
            "paused-activity-is-preserved",
        ),
        (
            "diff-edit",
            "diff-viewer-staging",
            TransitionKind::Direct,
            "direct",
            "pending-edit-is-read-only",
        ),
        (
            "diff-viewer-staging",
            "chat-turn",
            TransitionKind::Direct,
            "direct",
            "included-hunks-apply-once",
        ),
        (
            "chat-turn",
            "chat-turn",
            TransitionKind::Summarized,
            "summarized",
            "exact-paused-activity-resumes",
        ),
    ];
    assert_eq!(flow.steps.len(), expected.len());

    for (step, (screen, destination, transition_kind, audit_kind, acceptance)) in
        flow.steps.iter().zip(expected)
    {
        assert_eq!(step.screen_id, screen);
        assert_eq!(step.expected_next_frame, destination);
        assert_eq!(step.transition_kind, transition_kind);
        assert!(!step.user_action.trim().is_empty());
        assert!(!step.transition_path.trim().is_empty());
        assert!(!step.origin.trim().is_empty());
        assert!(!step.success_return.trim().is_empty());
        assert!(!step.cancel_return.trim().is_empty());
        assert!(
            step.acceptance_checks.contains(&acceptance),
            "{screen} must publish its journey-specific acceptance check"
        );
        if matches!(screen, "diff-viewer-staging" | "chat-turn") {
            assert_eq!(step.terminal_return, Some("chat-turn"));
        }

        let audit = audit_screen(screen, &format!("flow2-storyboard.{screen}"))
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
                        && transition.kind == audit_kind
                })
                .count(),
            1,
            "{screen} must have exactly one storyboard edge to {destination}"
        );
    }

    let mention = audit_screen("file-mention", "flow2-storyboard.submit")
        .expect("file mention must expose the submit bridge");
    assert!(mention
        .render_txt()
        .contains("[enter] Submit prepared prompt"));
    assert!(mention.transitions.iter().any(|transition| {
        transition.action_id == "action.file-mention.submit-prepared-prompt"
            && transition.to_screen_id == "permission-shell"
    }));

    let resumed = audit_screen("chat-turn", "flow2-storyboard.resumed")
        .expect("the final chat turn must render");
    let resumed_text = resumed.render_txt();
    assert!(resumed_text.contains("Updated retry_policy.rs."));
    assert!(resumed_text.contains("The request is complete; a new draft stays separate."));
}

#[test]
fn explicit_submit_and_approvals_keep_one_session_prompt_and_paused_activity() {
    let submit = replay_case("flow2-submit-bridge", 1)
        .expect("the explicit prompt submission must be replayable");
    let permission =
        replay_case("permission-allow-once", 1).expect("permission approval must replay");
    let review = replay_case("diff-review", 1).expect("diff review must replay");
    let finish = replay_case("staging-finish", 1).expect("staging finish must replay");

    for replay in [&submit, &permission, &review, &finish] {
        replay
            .validate()
            .unwrap_or_else(|error| panic!("{} must validate: {error}", replay.case_id));
    }
    assert_eq!(submit.before.screen_id, "file-mention");
    assert_eq!(submit.after.screen_id, "permission-shell");
    assert_eq!(
        submit.actions[0].action_id,
        "action.file-mention.submit-prepared-prompt"
    );
    assert_eq!(submit.attempts[0].count, 1);
    assert_eq!(submit.side_effects[0].kind, "prompt-submission");

    let submitted_hash = sha256_hex(SUBMITTED_PROMPT.as_bytes());
    let later_draft_hash = sha256_hex(submit.after.draft.as_bytes());
    assert_ne!(
        submitted_hash, later_draft_hash,
        "the immutable submitted prompt and later unsent draft are distinct fields"
    );
    assert_eq!(
        submit.after.submitted_prompt.as_deref(),
        Some(SUBMITTED_PROMPT)
    );
    assert!(submit.after.transcript.contains(SUBMITTED_PROMPT));

    let states = [
        &submit.before,
        &submit.after,
        &permission.before,
        &permission.after,
        &review.before,
        &review.after,
        &finish.before,
        &finish.after,
    ];
    for state in states {
        assert_eq!(state.conversation_id, "conversation.retry-review");
        assert_eq!(state.transcript_id, "transcript.retry-review");
        assert_eq!(state.session_id, "session.retry-review.18");
        assert_eq!(state.turn_id, "turn.retry-review.18");
        assert_eq!(state.user_task, "Check the retry path before we merge.");
        assert_eq!(
            state.visible_origin,
            "Input and approval · paused conversation"
        );
        assert_eq!(state.paused_activity_id, "activity.retry-policy-edit.18");
    }
    for state in [
        &submit.after,
        &permission.before,
        &permission.after,
        &review.before,
        &review.after,
        &finish.before,
        &finish.after,
    ] {
        assert_eq!(state.submitted_prompt.as_deref(), Some(SUBMITTED_PROMPT));
        assert_eq!(
            sha256_hex(state.submitted_prompt.as_ref().unwrap().as_bytes()),
            submitted_hash
        );
        assert_eq!(state.draft, submit.after.draft);
    }
    assert_eq!(finish.after.screen_id, "chat-turn");
    assert_eq!(finish.after.scroll_anchor, "turn-18");
    assert_eq!(finish.after.return_policy, "return-to-paused-turn");
    assert_eq!(finish.diff.as_ref().unwrap().write_count, 1);
    assert_eq!(
        finish.after.draft,
        "Add a short note about the remaining test hunk."
    );
    assert_ne!(finish.after.draft, SUBMITTED_PROMPT);

    for replay in [&permission, &review, &finish] {
        for field in [
            "session_id",
            "submitted_prompt",
            "paused_activity_id",
            "draft",
        ] {
            let hashes = replay
                .preserved_field_hashes
                .get(field)
                .unwrap_or_else(|| panic!("{field} continuity must be hash-bound"));
            assert_eq!(
                hashes.before, hashes.after,
                "{} must preserve {field}",
                replay.case_id
            );
        }
    }
}

#[test]
fn public_storyboard_audits_bind_the_exact_flow2_fixture_and_same_turn_identity() {
    let flow = flow_manifests()
        .iter()
        .find(|flow| flow.name == "input-and-approval")
        .expect("input and approval must be a public flow");

    for step in &flow.steps {
        let audit = audit_screen(step.screen_id, step.fixture_id)
            .unwrap_or_else(|error| panic!("{} must render: {error}", step.screen_id));
        let identity = audit
            .conversation
            .as_ref()
            .unwrap_or_else(|| panic!("{} must expose Flow 2 identity", step.screen_id));
        assert_eq!(audit.fixture_id, "flow2.input-and-approval.stable");
        assert_eq!(identity.conversation_id, "conversation.retry-review");
        assert_eq!(identity.transcript_id, "transcript.retry-review");
        assert_eq!(identity.session_id, "session.retry-review.18");
        assert_eq!(identity.turn_id, "turn.retry-review.18");
        assert_eq!(
            identity.paused_activity_id.as_deref(),
            Some("activity.retry-policy-edit.18")
        );
        assert_eq!(identity.submitted_prompt.as_deref(), Some(SUBMITTED_PROMPT));
        assert_eq!(
            identity.later_draft.as_deref(),
            Some("Add a short note about the remaining test hunk.")
        );
    }

    let output = harness()
        .args(["--audit-json", "--flow", "input-and-approval"])
        .output()
        .expect("the public Flow 2 audit selector must run");
    assert!(
        output.status.success(),
        "public Flow 2 audit failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json = String::from_utf8(output.stdout).expect("audit JSON must be UTF-8");
    assert_eq!(
        json.matches("\"document_type\":\"typed-render-audit\"")
            .count(),
        6
    );
    assert_eq!(json.matches("\"conversation\":{").count(), 6);
    assert_eq!(
        json.matches("\"turn_id\":\"turn.retry-review.18\"").count(),
        6
    );
    assert!(!json.contains("public-audit.input-and-approval"));
}

#[test]
fn final_storyboard_frame_visibly_separates_submitted_prompt_later_draft_and_resumption() {
    let audit = audit_screen("chat-turn", "flow2.input-and-approval.stable")
        .expect("the Flow 2 resumed turn must render");
    let text = audit.render_txt();

    for submitted_line in [
        "Check the retry path in @src/net/retry_policy.rs",
        "Then explain the failure path that still needs coverage before the merge;",
        "include a concise test plan with the visible recovery behavior.",
        "Keep the summary short before we merge.",
    ] {
        assert!(
            text.contains(submitted_line),
            "immutable transcript input must show {submitted_line:?}"
        );
    }
    assert!(text.contains("Paused activity · activity.retry-policy-edit.18"));
    assert!(text.contains("Session · session.retry-review.18"));
    assert!(text.contains("Turn · turn.retry-review.18 · resumed"));
    assert!(text.contains("Later draft · not sent"));
    assert!(text.contains("Add a short note about the remaining test hunk."));
    assert!(
        text.find("Keep the summary short before we merge.") < text.find("Later draft · not sent"),
        "submitted transcript input must remain above the later unsent draft"
    );
}

#[test]
fn public_flow_publishes_six_byte_identical_canonical_aliases() {
    let prior = harness()
        .args(["--flow", "daily-work"])
        .output()
        .expect("the previously published daily flow must run");
    assert!(prior.status.success());
    let output = harness()
        .args(["--flow", "input-and-approval"])
        .output()
        .expect("the public input-and-approval selector must run");
    assert!(
        output.status.success(),
        "input and approval must publish after preflight:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest = String::from_utf8(output.stdout).expect("flow metadata must be UTF-8");
    assert!(manifest.contains("\"flow\":\"input-and-approval\""));
    assert!(manifest.contains("\"action\":\"Submit the prepared prompt\""));
    assert!(manifest.contains("\"transition_kind\":\"summarized\""));
    assert!(manifest.contains("\"terminal_return\":\"chat-turn\""));
    assert!(manifest.contains("\"acceptance_checks\""));
    assert!(
        Path::new("/projects/cortex-code/plans/assets/mockups-v3-gates/daily-work/01-home.txt")
            .is_file(),
        "publishing Flow 2 must preserve the already published Flow 1 gate"
    );

    let frames = [
        ("input", "composer-multiline"),
        ("input", "file-mention"),
        ("input", "permission-shell"),
        ("session", "diff-edit"),
        ("views", "diff-viewer-staging"),
        ("session", "chat-turn"),
    ];
    for (index, (category, screen)) in frames.iter().enumerate() {
        for extension in ["txt", "png"] {
            let canonical = format!(
                "/projects/cortex-code/plans/assets/mockups-v3/{category}/{screen}.{extension}"
            );
            let gate = format!(
                "/projects/cortex-code/plans/assets/mockups-v3-gates/input-and-approval/{:02}-{screen}.{extension}",
                index + 1
            );
            assert!(Path::new(&canonical).is_file(), "missing {canonical}");
            assert!(Path::new(&gate).is_file(), "missing {gate}");
            let expected = audit_screen(screen, "flow2.input-and-approval.stable").unwrap();
            let expected_bytes = if extension == "txt" {
                expected.render_txt().into_bytes()
            } else {
                mockup_harness_v3::scenario::render_audit_png(&expected).unwrap()
            };
            assert_eq!(
                expected_bytes,
                fs::read(&gate).unwrap(),
                "{gate} must be canonical-equivalent for the declared Flow 2 fixture"
            );
            if *screen == "chat-turn" {
                assert_eq!(
                    expected_bytes,
                    fs::read(&canonical).unwrap(),
                    "canonical session/chat-turn must keep the Flow 2 paused retry identity"
                );
            }
            assert!(Path::new(&canonical).is_file());
        }
    }
}

#[test]
fn terminal_chat_turn_keeps_paused_retry_origin_after_staging_finish() {
    let finish = replay_case("staging-finish", 1).expect("staging finish must replay");
    finish
        .validate()
        .expect("staging finish must validate closed side effects");
    assert_eq!(finish.after.screen_id, "chat-turn");
    assert_eq!(finish.after.session_id, "session.retry-review.18");
    assert_eq!(finish.after.turn_id, "turn.retry-review.18");
    assert_eq!(
        finish.after.visible_origin,
        "Input and approval · paused conversation"
    );
    assert_eq!(
        finish.after.user_task,
        "Check the retry path before we merge."
    );
    assert!(
        finish.audit.render_txt().contains("session.retry-review.18"),
        "the after-state render must keep the paused retry session identity"
    );
    assert!(
        !finish.audit.render_txt().contains("Daily work · current conversation"),
        "the Flow 2 terminal destination must not fall back to Daily work identity"
    );

    let published = audit_screen("chat-turn", "flow2.input-and-approval.stable")
        .expect("the Flow 2 terminal chat-turn fixture must render");
    let published_text = published.render_txt();
    assert!(published_text.contains("Input and approval · paused conversation"));
    assert!(published_text.contains("Check the retry path before we merge."));
    assert!(published_text.contains("session.retry-review.18"));
    assert!(!published_text.contains("Daily work · current conversation"));
    assert!(!published_text.contains(
        "Find where the server applies the rate limit and explain the safe path."
    ));
}

/// P0-3 / human-gate rejection: banner task, You-phrase, and submitted draft body
/// must share one continuous identity for the input-and-approval pause window.
/// Synonym swaps (Check vs Review) and mid-window task rewrites without a bridge fail.
#[test]
fn input_and_approval_pause_window_keeps_one_task_you_and_draft_chain() {
    const TASK: &str = "Check the retry path before we merge.";
    const YOU_LINE: &str = "You · Check the retry path before we merge.";
    const STABLE: &str = "flow2.input-and-approval.stable";

    let ordered = [
        "composer-multiline",
        "file-mention",
        "permission-shell",
        "diff-edit",
        "diff-viewer-staging",
        "chat-turn",
    ];

    for screen in ordered {
        let audit = audit_screen(screen, STABLE)
            .unwrap_or_else(|error| panic!("{screen} must render under the Flow 2 fixture: {error}"));
        let text = audit.render_txt();
        let identity = audit
            .conversation
            .as_ref()
            .unwrap_or_else(|| panic!("{screen} must expose conversation identity"));

        assert_eq!(identity.user_task, TASK);
        assert_eq!(identity.visible_origin, "Input and approval · paused conversation");
        assert_eq!(identity.session_id, "session.retry-review.18");
        assert_eq!(identity.turn_id, "turn.retry-review.18");
        assert_eq!(
            identity.submitted_prompt.as_deref(),
            Some(SUBMITTED_PROMPT),
            "{screen} must keep the immutable submitted prompt identity"
        );
        assert!(
            text.contains("Input and approval · paused conversation"),
            "{screen} must keep the paused Input and approval origin chip"
        );
        assert!(
            text.contains(TASK),
            "{screen} must keep the stable banner task {TASK:?}"
        );
        // Continuous pause: no synonym swap or silent rewrite.
        assert!(
            !text.contains("You · Review the retry path before we merge."),
            "{screen} must not use the Check vs Review synonym for the You-line"
        );
        assert!(
            !text.contains("You · Check the project files before summarizing the retry result."),
            "{screen} must not rewrite the pause-window You-line to a different task without a bridge"
        );
        assert!(
            !text.contains("You · Submitted prompt"),
            "{screen} must not hide the human task behind a generic 'Submitted prompt' label"
        );

        // Sticky You-line (or staging detail that keeps the banner without inventing a new one).
        if screen != "diff-viewer-staging" {
            assert!(
                text.contains(YOU_LINE),
                "{screen} must show the continuous You-line matching the banner task"
            );
        } else {
            assert!(
                text.contains("From pending edit in the paused retry conversation"),
                "staging must bridge back to the same pause conversation without a new You-phrase"
            );
        }

        // Draft / submitted body starts with the same verb and subject as the task.
        if matches!(screen, "composer-multiline" | "file-mention") {
            assert!(
                text.contains("Check the retry path in @retry")
                    || text.contains("Check the retry path in @src/net/retry_policy.rs"),
                "{screen} draft body must start from the stable Check the retry path request"
            );
        }
        if screen == "chat-turn" {
            for submitted_line in [
                "Check the retry path in @src/net/retry_policy.rs",
                "Then explain the failure path that still needs coverage before the merge;",
                "include a concise test plan with the visible recovery behavior.",
                "Keep the summary short before we merge.",
            ] {
                assert!(
                    text.contains(submitted_line),
                    "terminal chat-turn must still show immutable submitted body line {submitted_line:?}"
                );
            }
            assert!(
                text.contains("session.retry-review.18"),
                "terminal chat-turn must retain session.retry-review.18 continuity"
            );
            assert!(
                text.find(YOU_LINE) < text.find("Check the retry path in @src/net/retry_policy.rs"),
                "You-line must label the submitted body, not replace it"
            );
        }
    }
}
