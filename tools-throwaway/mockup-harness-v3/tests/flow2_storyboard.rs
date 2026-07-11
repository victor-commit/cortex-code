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
        assert_eq!(state.session_id, "session.retry-review.18");
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
            assert_eq!(
                fs::read(&canonical).unwrap(),
                fs::read(&gate).unwrap(),
                "{gate} must be a byte-identical canonical alias"
            );
        }
    }
}
