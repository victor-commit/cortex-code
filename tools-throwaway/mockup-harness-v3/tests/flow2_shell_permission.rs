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
    assert!(
        final_page
            .transitions
            .iter()
            .any(|transition| transition.action_id == "action.permission.command-return")
    );

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
fn every_public_page_reconstructs_its_exact_window_and_adjacent_offsets() {
    let command =
        b"printf '%s\\n' \"retry check\" && git diff -- src/net/retry_policy.rs > /tmp/retry.diff && printf '%s\\n' \"review complete\"";
    let pages = [
        ("permission-shell-long", 44, 88, Some(0), Some(88)),
        ("permission-shell-long-page-44", 88, 119, Some(44), None),
        ("permission-shell-long-final", 88, 119, Some(44), None),
    ];

    for (case_id, expected_offset, expected_end, expected_previous, expected_next) in pages {
        let replay = replay_case(case_id, 1).expect("page replay must render");
        replay.validate().expect("page replay must validate");
        let permission = replay.permission.as_ref().expect("page evidence");
        assert_eq!(permission.byte_offset, expected_offset);
        assert_eq!(
            permission.byte_offset + permission.reconstructed_page_bytes().len(),
            expected_end
        );
        assert_eq!(
            permission.reconstructed_page_bytes(),
            command[permission.byte_offset..expected_end],
            "{case_id} must show the exact execution byte slice"
        );
        assert_eq!(permission.previous_offset, expected_previous);
        assert_eq!(permission.next_offset, expected_next);
    }
}

#[test]
fn final_page_and_final_return_are_distinct_public_replays() {
    let middle =
        replay_case("permission-shell-long-middle-to-final", 1).expect("middle-to-final replay");
    let final_page = replay_case("permission-shell-long-final", 1).expect("final page replay");
    let final_return =
        replay_case("permission-shell-long-final-to-return", 1).expect("final return replay");

    assert_ne!(middle.case_id, final_page.case_id);
    assert_eq!(middle.after.selection, "long-command-page-88");
    assert_eq!(final_page.after.selection, "long-command-page-88");
    assert_eq!(
        final_page.permission.as_ref().unwrap().previous_offset,
        Some(44)
    );
    assert_eq!(
        final_return.transitions[0].action_id,
        "action.permission.command-return"
    );
    assert_eq!(final_return.before.selection, "long-command-page-88");
    assert_eq!(final_return.after.selection, "permission-choice");
    assert!(
        final_return
            .audit
            .render_txt()
            .contains("Permission choices")
    );
}

#[test]
fn paging_actions_render_their_resulting_after_state() {
    let page_zero_next =
        replay_case("permission-shell-long", 1).expect("page-zero Next must replay");
    page_zero_next
        .validate()
        .expect("page-zero Next must validate");
    let page_zero_text = page_zero_next.audit.render_txt();
    assert_eq!(page_zero_next.after.selection, "long-command-page-44");
    assert_eq!(page_zero_next.permission.as_ref().unwrap().byte_offset, 44);
    assert!(page_zero_text.contains("bytes 44..88 of 119"));
    assert!(page_zero_text.contains("Previous page: 0 · Next page: 88"));
    assert_eq!(
        page_zero_next.actions[0].action_id,
        "action.permission.command-next"
    );
    assert_eq!(
        page_zero_next.transitions[0].action_id,
        "action.permission.command-next"
    );

    let page_forty_four_next =
        replay_case("permission-shell-long-page-44", 1).expect("page-44 Next must replay");
    page_forty_four_next
        .validate()
        .expect("page-44 Next must validate");
    let final_text = page_forty_four_next.audit.render_txt();
    let permission = page_forty_four_next.permission.as_ref().unwrap();
    assert_eq!(page_forty_four_next.after.selection, "long-command-page-88");
    assert_eq!(permission.byte_offset, 88);
    assert_eq!(permission.page_end, permission.execution_bytes.len());
    assert_eq!(
        permission.reconstructed_page_bytes(),
        permission.execution_bytes[88..]
    );
    assert!(final_text.contains("bytes 88..119 of 119"));
    assert!(final_text.contains("printf '%s\\n' \"review complete\""));
    assert!(final_text.contains("Previous page: 44 · Next page: disabled"));
    assert_eq!(
        page_forty_four_next.actions[0].action_id,
        "action.permission.command-next-final"
    );
    assert_eq!(
        page_forty_four_next.transitions[0].action_id,
        "action.permission.command-next-final"
    );
}

#[test]
fn permission_request_bytes_are_bound_to_execution_evidence() {
    for case_id in [
        "permission-shell-short",
        "permission-shell-long",
        "permission-shell-long-page-44",
        "permission-shell-long-final-to-return",
        "permission-shell-unsafe-byte",
        "permission-persistent-scope-outside",
    ] {
        let replay = replay_case(case_id, 1).expect("permission replay must exist");
        let permission = replay.permission.as_ref().expect("permission evidence");
        assert_eq!(
            replay.after.requested_command.as_bytes(),
            permission.execution_bytes,
            "{case_id} must bind the resulting request to exact execution bytes"
        );
    }
}

#[test]
fn final_return_renders_a_completed_permission_choice_destination() {
    let replay =
        replay_case("permission-shell-long-final-to-return", 1).expect("final Return must replay");
    replay.validate().expect("final Return must validate");
    let text = replay.audit.render_txt();
    assert_eq!(replay.after.selection, "permission-choice");
    assert_eq!(replay.after.focus, "permission-choice");
    assert!(!text.contains("Command detail · focused disclosure"));
    assert!(!text.contains("[r] Return to permission choices"));
    assert!(!replay.audit.events.iter().any(|event| {
        event
            .action
            .as_ref()
            .is_some_and(|action| action.action_id == "action.permission.command-return")
    }));
    assert!(
        !replay
            .audit
            .transitions
            .iter()
            .any(|transition| transition.action_id == "action.permission.command-return")
    );
    assert_eq!(
        replay.actions[0].action_id, "action.permission.command-return",
        "the replay still records the completed action that reached this destination"
    );
}

#[test]
fn permission_paging_rejects_gaps_overlaps_wrong_offsets_and_fabricated_notation() {
    for fixture in [
        "permission-page-gap",
        "permission-page-overlap",
        "permission-page-wrong-offset",
        "permission-page-fabricated-notation",
    ] {
        let result = mockup_harness_v3::negative_fixtures::run_public_negative_fixture(fixture);
        assert!(
            result.is_err(),
            "{fixture} must reject invalid paging evidence"
        );
    }
}

#[test]
fn permission_replays_reject_pre_transition_rendering_command_mismatch_and_stale_return() {
    for (fixture, diagnostic) in [
        (
            "permission-pre-transition-render",
            "permission replay rendered pre-transition state",
        ),
        (
            "permission-request-execution-mismatch",
            "requested_command bytes do not equal permission execution_bytes",
        ),
        (
            "permission-stale-terminal-return",
            "completed permission-choice destination retains stale Return",
        ),
    ] {
        let error = mockup_harness_v3::negative_fixtures::run_public_negative_fixture(fixture)
            .expect_err("invalid permission replay must be rejected");
        assert!(
            error.contains(diagnostic),
            "{fixture} rejected with unexpected diagnostic: {error}"
        );
    }
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
    assert!(
        replay
            .audit
            .render_txt()
            .contains("Persistent Allow: unavailable")
    );
}

#[test]
fn persistent_allow_is_conditional_and_audited() {
    let unsupported = audit_screen("permission-shell", "flow2-permission-shell-short")
        .expect("unsupported permission branch must render");
    assert!(
        unsupported
            .render_txt()
            .contains("Persistent Allow: unavailable")
    );
    assert!(
        !unsupported
            .events
            .iter()
            .any(|event| event.identity.subject_id == "action.permission.allow-persistent")
    );

    let supported = audit_screen(
        "permission-shell",
        "flow2-permission-shell-persistent-supported",
    )
    .expect("supported permission branch must render");
    assert!(supported.render_txt().contains("Allow for this project"));
    assert!(
        supported
            .events
            .iter()
            .any(|event| event.identity.subject_id == "action.permission.allow-persistent")
    );
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
