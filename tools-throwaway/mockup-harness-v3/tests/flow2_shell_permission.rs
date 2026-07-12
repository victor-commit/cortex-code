use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

#[test]
fn permission_shell_puts_purpose_and_effect_before_choices_and_discloses_short_command() {
    let audit = audit_screen("permission-shell", "flow2-permission-shell-short")
        .expect("short shell permission must render");
    audit
        .validate()
        .expect("short permission audit must validate");
    let text = audit.render_txt();
    let purpose = text.find("Why Code needs it").expect("purpose heading");
    let effect = text
        .find("What running it will do")
        .expect("effect heading");
    let command = text.find("Command").expect("command disclosure");
    let choice = text.find("Allow once").expect("allow choice");
    assert!(purpose < effect && effect < command && command < choice);
    assert!(text.contains("printf '%s\\n' \"retry check\""));
    assert!(text.contains("Working directory: ~/project"));
    assert!(text.contains("Scope: this command only"));
    assert!(text.contains("Allow once"));
    assert!(text.contains("Deny"));
}

#[test]
fn long_permission_command_pages_have_exact_byte_boundaries_and_hash_evidence() {
    let command = b"printf '%s\\n' \"retry check\" && git diff -- src/net/retry_policy.rs > /tmp/retry.diff && printf '%s\\n' \"review complete\"";
    let expected_hash = mockup_harness_v3::audit::sha256_hex(command);
    let first = audit_screen("permission-shell", "flow2-permission-shell-long-offset-0")
        .expect("first command page must render");
    let next = audit_screen("permission-shell", "flow2-permission-shell-long-offset-44")
        .expect("next command page must render");
    first.validate().expect("first page must validate");
    next.validate().expect("next page must validate");
    let first_text = first.render_txt();
    let next_text = next.render_txt();
    assert!(first_text.contains("Command detail"));
    assert!(first_text.contains("bytes 0..44"));
    assert!(first_text.contains("Previous page: disabled"));
    assert!(first_text.contains("Next page: 44"));
    assert!(next_text.contains("bytes 44..88"));
    assert!(next_text.contains("Previous page: 0"));
    assert!(next_text.contains("SHA-256"));
    assert!(first_text.contains(&expected_hash));
    assert!(next_text.contains(&expected_hash));
}

#[test]
fn public_long_page_boundaries_cover_middle_final_and_return_transitions() {
    let command_len = 119;
    let final_page = audit_screen("permission-shell", "flow2-permission-shell-long-offset-88")
        .expect("final command page must render");
    final_page.validate().expect("final page must validate");
    let final_text = final_page.render_txt();
    assert!(final_text.contains("bytes 88..119 of 119"));
    assert!(final_text.contains("Previous page: 44"));
    assert!(final_text.contains("Next page: disabled"));
    assert!(final_text.contains("Return to permission choices"));
    assert_eq!(command_len, 119);
    assert!(final_page
        .transitions
        .iter()
        .any(|transition| transition.action_id == "action.permission.command-return"));

    let middle =
        replay_case("permission-shell-long-middle", 1).expect("middle page transition must replay");
    middle.validate().expect("middle page replay must validate");
    assert_eq!(middle.before.selection, "long-command-page-44");
    assert_eq!(middle.after.selection, "long-command-page-88");
    assert_eq!(middle.permission.as_ref().unwrap().byte_offset, 88);
    assert_eq!(
        middle.permission.as_ref().unwrap().next_offset,
        None,
        "the resulting final page must expose a disabled next boundary"
    );

    let returned =
        replay_case("permission-shell-long-return", 1).expect("return from final page must replay");
    returned.validate().expect("return replay must validate");
    assert_eq!(returned.before.selection, "long-command-page-88");
    assert_eq!(returned.after.selection, "permission-choice");
    assert_eq!(returned.before.screen_id, returned.after.screen_id);
    assert_eq!(
        returned.transitions[0].action_id,
        "action.permission.command-return"
    );
    assert!(returned.audit.render_txt().contains("Permission choices"));
}

#[test]
fn unsafe_permission_bytes_use_reversible_escaping_bound_to_exact_hash() {
    let replay = replay_case("permission-shell-unsafe-byte", 1)
        .expect("unsafe-byte permission replay must render");
    replay.validate().expect("unsafe-byte replay must validate");
    let permission = replay
        .permission
        .as_ref()
        .expect("permission replay must include command evidence");
    assert!(permission.display_notation.contains("\\x00"));
    assert!(permission.display_notation.contains("\\x1b"));
    assert_eq!(
        permission.command_sha256,
        mockup_harness_v3::audit::sha256_hex(&permission.execution_bytes)
    );
    assert_eq!(permission.reconstructed_bytes(), permission.execution_bytes);
}

#[test]
fn allow_once_executes_exactly_once_without_persisting_a_rule() {
    let replay =
        replay_case("permission-allow-once", 1).expect("allow-once permission replay must render");
    replay.validate().expect("allow-once replay must validate");
    assert_eq!(replay.after.screen_id, "diff-edit");
    assert_eq!(replay.attempts[0].count, 1);
    assert_eq!(replay.side_effects[0].status, "executed-once");
    let permission = replay.permission.as_ref().unwrap();
    assert_eq!(permission.invocation_count, 1);
    assert_eq!(permission.persisted_rule_count, 0);
    assert!(permission.execution_working_directory == "~/project");
}

#[test]
fn persistent_allow_discloses_exact_scope_and_rejects_out_of_scope_replays() {
    let replay =
        replay_case("permission-persistent-scope", 1).expect("persistent scope replay must render");
    replay
        .validate()
        .expect("persistent scope replay must validate");
    let permission = replay.permission.as_ref().unwrap();
    let scope = permission.persistent_scope.as_ref().expect("scope");
    assert_eq!(scope.matcher, "git diff -- src/net/*");
    assert_eq!(scope.duration, "this session");
    assert_eq!(scope.project_root, "~/project");
    assert_eq!(scope.work_style, "Careful edits");
    assert_eq!(
        scope.child_inheritance,
        "children inherit only read-only checks"
    );
    assert_eq!(
        scope.child_cap,
        "read-only, same project root, same session"
    );
    assert_eq!(permission.persisted_rule_count, 1);
    assert_eq!(
        replay_case("permission-persistent-scope-outside", 1)
            .unwrap()
            .permission
            .unwrap()
            .persisted_rule_count,
        0
    );
}

#[test]
fn out_of_scope_replay_changes_request_and_renders_new_permission_boundary() {
    let replay = replay_case("permission-persistent-scope-outside", 1)
        .expect("out-of-scope replay must render a fresh request");
    replay
        .validate()
        .expect("out-of-scope replay must validate");
    assert_eq!(replay.before.screen_id, "permission-shell");
    assert_eq!(replay.after.screen_id, "permission-shell");
    assert_ne!(
        replay.before.requested_command, replay.after.requested_command,
        "the fresh request must change the command input"
    );
    assert_ne!(
        replay.before.requested_working_directory, replay.after.requested_working_directory,
        "the fresh request must change at least one scope input"
    );
    assert_eq!(
        replay.after.permission_request_status,
        "new permission required"
    );
    assert_eq!(replay.permission.as_ref().unwrap().persisted_rule_count, 0);
    assert!(replay
        .audit
        .render_txt()
        .contains("Persistent Allow: unavailable"));
}

#[test]
fn persistent_allow_is_conditional_and_audited() {
    let unsupported = audit_screen("permission-shell", "flow2-permission-shell-short")
        .expect("unsupported permission branch must render");
    assert!(unsupported
        .render_txt()
        .contains("Persistent Allow: unavailable"));
    assert!(!unsupported
        .events
        .iter()
        .any(|event| event.identity.subject_id == "action.permission.allow-persistent"));

    let supported = audit_screen(
        "permission-shell",
        "flow2-permission-shell-persistent-supported",
    )
    .expect("supported permission branch must render");
    assert!(supported.render_txt().contains("Allow for this project"));
    assert!(supported
        .events
        .iter()
        .any(|event| event.identity.subject_id == "action.permission.allow-persistent"));
}

#[test]
fn deny_and_cancel_never_execute_and_preserve_origin() {
    let denied = replay_case("permission-deny", 1).expect("deny replay");
    let cancelled = replay_case("permission-cancel", 1).expect("cancel replay");
    for replay in [&denied, &cancelled] {
        replay.validate().expect("permission replay must validate");
        assert_eq!(replay.attempts[0].count, 0);
        assert_eq!(replay.permission.as_ref().unwrap().invocation_count, 0);
        assert_eq!(replay.before.origin_stack, replay.after.origin_stack);
        assert_eq!(replay.before.draft, replay.after.draft);
    }
    assert_eq!(denied.permission.as_ref().unwrap().verdict, "denied");
    assert_eq!(cancelled.permission.as_ref().unwrap().verdict, "pending");
    assert_eq!(cancelled.transitions[0].kind, "cancel");
}
