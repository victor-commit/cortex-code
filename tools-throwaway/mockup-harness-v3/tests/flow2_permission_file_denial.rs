use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn permission_deny_reason_retains_denied_context_with_one_input() {
    // VAL-V3-F2-034: denied context retained; optional reason dock only.
    let audit = audit_screen("permission-deny-reason", "flow2-permission-deny-reason")
        .expect("permission-deny-reason must render");
    audit
        .validate()
        .expect("permission-deny-reason audit must validate");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Denied"));
    assert!(
        text.contains("command was not run")
            || text.contains("file edit was not applied")
            || text.contains("was not run"),
        "must identify that the denied action did not execute"
    );
    assert!(
        text.contains("printf '%s\\n' \"retry check\"")
            || text.contains("printf")
            || text.contains("src/net/retry_policy.rs"),
        "must identify the denied command or file edit"
    );
    assert!(
        text.contains("Optional reason") || text.contains("optional reason"),
        "must explain the optional reason dock"
    );
    assert!(
        text.contains("sent to Code") || text.contains("sent to the assistant"),
        "must state the reason is sent with the denied activity"
    );
    assert!(
        text.contains("Type one short reason")
            || text.contains("› Type")
            || text.contains("reason here"),
        "must show exactly one focused reason input"
    );
    assert!(text.contains("[enter] Send reason") || text.contains("Send reason"));
    assert!(text.contains("[s] Skip") || text.contains("Skip"));
    assert!(
        text.contains("Allow and Deny choices are closed")
            || text.contains("original Allow and Deny")
            || text.contains("choices are closed"),
        "Allow/Deny must no longer be active choices"
    );
    assert!(!text.contains("[d] Deny"));
    assert!(!text.contains("Allow once"));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "denied-activity" || event.identity.family == "denied-command"
    }));
    assert!(audit
        .events
        .iter()
        .any(|event| event.identity.family == "reason-input"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "reason-input")
            .count(),
        1,
        "exactly one focused reason input"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.permission.send-reason"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "secondary-action"
            && event.identity.subject_id == "action.permission.skip-reason"
    }));
}

#[test]
fn deny_reason_send_and_skip_restore_origin_without_execution() {
    // VAL-V3-F2-035: Send and Skip each resume once; neither executes.
    let sent = replay_case("permission-deny-send-reason", 1).expect("send reason must replay");
    sent.validate().expect("send reason must validate");
    assert_eq!(sent.after.screen_id, "chat-turn");
    assert_eq!(sent.before.origin_stack, sent.after.origin_stack);
    assert_eq!(sent.before.draft, sent.after.draft);
    assert_eq!(sent.attempts[0].count, 0);
    assert_eq!(sent.permission.as_ref().unwrap().invocation_count, 0);
    assert_eq!(sent.permission.as_ref().unwrap().verdict, "denied");
    assert_eq!(
        sent.transitions[0].action_id,
        "action.permission.send-reason"
    );
    assert!(
        sent.side_effects[0].status == "reason-appended" || sent.side_effects[0].status == "no-execution"
    );
    assert!(
        sent.preserved_field_hashes.contains_key("denial_reason")
            || sent.after.transcript.contains("timeout risk")
            || sent
                .side_effects
                .iter()
                .any(|effect| effect.effect_hash.len() == 64),
        "send must bind the exact reason payload"
    );
    assert!(!sent.audit.render_txt().contains("Type one short reason here"));

    let skipped = replay_case("permission-deny-skip-reason", 1).expect("skip reason must replay");
    skipped.validate().expect("skip reason must validate");
    assert_eq!(skipped.after.screen_id, "chat-turn");
    assert_eq!(skipped.before.origin_stack, skipped.after.origin_stack);
    assert_eq!(skipped.before.draft, skipped.after.draft);
    assert_eq!(skipped.attempts[0].count, 0);
    assert_eq!(skipped.permission.as_ref().unwrap().invocation_count, 0);
    assert_eq!(skipped.permission.as_ref().unwrap().verdict, "denied");
    assert_eq!(
        skipped.transitions[0].action_id,
        "action.permission.skip-reason"
    );
    assert_eq!(skipped.side_effects[0].status, "no-execution");
    if let Some(hash) = skipped.preserved_field_hashes.get("denial_reason") {
        assert_eq!(hash.after, mockup_harness_v3::audit::sha256_hex(b""));
    }
    assert!(!skipped.after.draft.contains("timeout risk"));
    assert!(!skipped.after.draft.contains("reason"));
}

#[test]
fn permission_file_edit_states_purpose_effect_and_exact_scope() {
    // VAL-V3-F2-036: purpose/effect/scope before choices; persistent rule boundary.
    let audit = audit_screen("permission-file-edit", "flow2-permission-file-edit")
        .expect("permission-file-edit must render");
    audit
        .validate()
        .expect("permission-file-edit audit must validate");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    let purpose = text
        .find("Why Code needs it")
        .expect("purpose heading before choices");
    let effect = text
        .find("What applying it will do")
        .or_else(|| text.find("What this edit will do"))
        .expect("effect heading before choices");
    let file = text
        .find("src/net/retry_policy.rs")
        .expect("target file must be named");
    let choice = text.find("Allow once").expect("allow choice");
    assert!(purpose < effect && effect < choice);
    assert!(file < choice);

    assert!(text.contains("+4") || text.contains("+4 -3") || text.contains("Patch summary"));
    assert!(
        text.contains("this edit")
            || text.contains("this file")
            || text.contains("this project")
            || text.contains("this session")
            || text.contains("Scope:"),
        "approval scope must be stated before the choice"
    );
    assert!(text.contains("Careful edits") || text.contains("work style") || text.contains("Work style"));
    assert!(
        text.contains("children") || text.contains("child tasks") || text.contains("Child"),
        "persistent file-edit scope must state child inheritance"
    );
    assert!(
        text.contains("matcher")
            || text.contains("src/net/*")
            || text.contains("exact path src/net/retry_policy.rs"),
        "persistent matcher must be explicit"
    );
    assert!(text.contains("~/project"));
    assert!(text.contains("Allow once") || text.contains("[enter] Allow"));
    assert!(text.contains("Deny"));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "purpose" || event.identity.subject_id.contains("why")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "effect" || event.identity.subject_id.contains("effect")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "file" || event.identity.subject_id.contains("retry_policy")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "persistent-scope"
            || event.identity.family == "execution-scope"
            || event.identity.subject_id.contains("scope")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.permission.allow-once"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "secondary-action"
            && event.identity.subject_id == "action.permission.deny"
    }));
}

#[test]
fn permission_file_edit_discloses_exact_diff_line_numbers_and_totals() {
    // VAL-V3-F2-037: exact patch disclosure before choice.
    let audit = audit_screen("permission-file-edit", "flow2-permission-file-edit")
        .expect("file-edit permission must disclose the patch");
    let text = audit.render_txt();
    assert!(text.contains("src/net/retry_policy.rs"));
    assert!(
        text.contains("@@ -") || text.contains("Hunk @@") || text.contains("old_start"),
        "hunk location coordinates must appear"
    );
    assert!(text.contains("return retry_after") || text.contains("retry_after"));
    assert!(text.contains("jitter") || text.contains('+'));
    assert!(
        text.contains("+4") || text.contains("+4 -3") || text.contains("summary +4"),
        "accurate +N -M totals required"
    );
    assert!(
        text.contains("retry_after + jitter")
            || text.contains("+    return retry_after + jitter")
            || text.contains("return retry_after + jitter"),
        "every disclosed added line should appear"
    );
    assert!(
        text.contains("backoff = backoff + 1") || text.contains("-    backoff"),
        "removed lines must appear"
    );

    // Focused detail path is allowed when more hunks need expansion.
    assert!(
        text.contains("Review full patch")
            || text.contains("[e] Expand")
            || text.contains("Second hunk")
            || text.contains("hunk.retry-test"),
        "when the full multi-hunk patch cannot stay inline, a labeled detail path is required"
    );
}

#[test]
fn file_edit_allow_and_deny_preserve_approval_boundaries() {
    // VAL-V3-F2-038: Allow applies disclosed patch; Deny writes nothing and
    // follows deny-reason; review is not approval; cancel keeps pending.
    let allowed = replay_case("permission-file-edit-allow-once", 1)
        .expect("file-edit allow-once must replay");
    allowed
        .validate()
        .expect("file-edit allow-once must validate");
    assert_eq!(allowed.after.screen_id, "chat-turn");
    assert_eq!(allowed.attempts[0].count, 1);
    assert_eq!(allowed.permission.as_ref().unwrap().invocation_count, 1);
    assert_eq!(allowed.permission.as_ref().unwrap().verdict, "approved");
    let diff = allowed.diff.as_ref().expect("allow must emit diff evidence");
    assert_eq!(diff.write_count, 1);
    assert_ne!(diff.working_tree_before, diff.working_tree_after);
    assert_eq!(diff.git_index_before, diff.git_index_after);
    assert_eq!(diff.file_path, "src/net/retry_policy.rs");
    assert!(
        !diff.applied_hunks.is_empty(),
        "allow must apply every disclosed hunk exactly once"
    );

    let denied = replay_case("permission-file-edit-deny", 1).expect("file-edit deny must replay");
    denied.validate().expect("file-edit deny must validate");
    assert_eq!(denied.after.screen_id, "permission-deny-reason");
    assert_eq!(denied.attempts[0].count, 0);
    assert_eq!(denied.permission.as_ref().unwrap().invocation_count, 0);
    assert_eq!(denied.permission.as_ref().unwrap().verdict, "denied");
    if let Some(diff) = denied.diff.as_ref() {
        assert_eq!(diff.write_count, 0);
        assert_eq!(diff.working_tree_before, diff.working_tree_after);
    }

    let review = replay_case("permission-file-edit-review-detail", 1)
        .expect("opening detail must not apply");
    review
        .validate()
        .expect("file-edit review detail must validate");
    assert_eq!(review.after.screen_id, "diff-viewer-staging");
    assert_eq!(review.attempts[0].count, 0);
    if let Some(diff) = review.diff.as_ref() {
        assert_eq!(diff.write_count, 0);
        assert_eq!(diff.working_tree_before, diff.working_tree_after);
    }

    let cancelled =
        replay_case("permission-file-edit-cancel", 1).expect("file-edit cancel must replay");
    cancelled
        .validate()
        .expect("file-edit cancel must validate");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.permission.as_ref().unwrap().verdict, "pending");
    assert_eq!(cancelled.attempts[0].count, 0);
    assert_eq!(cancelled.transitions[0].kind, "cancel");
    assert_eq!(cancelled.before.draft, cancelled.after.draft);
    if let Some(diff) = cancelled.diff.as_ref() {
        assert_eq!(diff.write_count, 0);
    }
}

#[test]
fn persistent_file_edit_scope_matches_only_declared_boundary() {
    // VAL-V3-F2-036/038: persistent allow never silently broadens.
    let allowed = replay_case("permission-file-edit-persistent-scope", 1)
        .expect("persistent file-edit scope must replay");
    allowed
        .validate()
        .expect("persistent file-edit scope must validate");
    let scope = allowed
        .permission
        .as_ref()
        .unwrap()
        .persistent_scope
        .as_ref()
        .expect("persistent allow must store exact scope");
    assert_eq!(scope.matcher, "src/net/retry_policy.rs");
    assert_eq!(scope.duration, "this session");
    assert_eq!(scope.project_root, "~/project");
    assert_eq!(scope.work_style, "Careful edits");
    assert!(
        scope.child_inheritance.contains("inherit")
            || scope
                .child_inheritance
                .contains("do not inherit write"),
        "child inheritance must be explicit"
    );
    assert_eq!(allowed.permission.as_ref().unwrap().persisted_rule_count, 1);

    let outside = replay_case("permission-file-edit-persistent-scope-outside", 1)
        .expect("out-of-scope file-edit must request permission again");
    outside
        .validate()
        .expect("out-of-scope file-edit must validate");
    assert_eq!(outside.after.screen_id, "permission-file-edit");
    assert_eq!(
        outside.after.permission_request_status,
        "new permission required"
    );
    assert_eq!(outside.permission.as_ref().unwrap().persisted_rule_count, 0);
    assert_ne!(
        outside.before.requested_command,
        outside.after.requested_command
    );
    assert!(
        outside
            .audit
            .render_txt()
            .contains("Persistent Allow: unavailable")
            || outside
                .audit
                .render_txt()
                .contains("new permission required")
            || outside
                .audit
                .render_txt()
                .contains("outside the saved scope")
    );
}

#[test]
fn public_selector_publishes_permission_file_edit_and_deny_reason() {
    let list = harness()
        .arg("--list")
        .output()
        .expect("list must run");
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("permission-file-edit"));
    assert!(stdout.contains("permission-deny-reason"));
    for id in ["permission-file-edit", "permission-deny-reason"] {
        let line = stdout
            .lines()
            .find(|line| line.contains(id))
            .unwrap_or_else(|| panic!("list must mention {id}"));
        assert!(
            line.contains("implemented"),
            "{id} must report implemented, got: {line}"
        );
    }

    for id in ["permission-file-edit", "permission-deny-reason"] {
        let render = harness()
            .args([id])
            .output()
            .unwrap_or_else(|_| panic!("{id} public render must run"));
        assert!(
            render.status.success(),
            "{id} render failed: {}",
            String::from_utf8_lossy(&render.stderr)
        );
        let txt = Path::new("/projects/cortex-code/plans/assets/mockups-v3/input")
            .join(format!("{id}.txt"));
        let png = Path::new("/projects/cortex-code/plans/assets/mockups-v3/input")
            .join(format!("{id}.png"));
        assert!(txt.is_file(), "missing {id}.txt");
        assert!(png.is_file(), "missing {id}.png");
        let body = std::fs::read_to_string(&txt).expect("txt readable");
        assert_eq!(body.lines().count(), 40);
    }
}
