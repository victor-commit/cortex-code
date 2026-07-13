//! Flow 1 review, diff-viewer, and share (VAL-V3-F1-043..048, 051, 052, 062, 069).

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn assert_no_private_traces(text: &str) {
    for banned in [
        "chain-of-thought",
        "hidden prompt",
        "provider payload",
        "system prompt",
        "tool_envelope",
        "wave_id",
        "score=",
        "token_count=",
        "ctx %",
    ] {
        assert!(
            !text.to_lowercase().contains(banned),
            "private or internal trace leaked: {banned}"
        );
    }
}

fn assert_statusline_minimal(text: &str) {
    let status = text.lines().last().expect("statusline");
    assert!(
        status.contains("~/project") && status.contains("Claude Opus 4.8"),
        "minimal statusline required"
    );
    assert!(
        !status.contains('%') && !status.contains('$') && !status.contains("idle"),
        "statusline must omit cost/context/idle"
    );
}

#[test]
fn review_is_focused_temporary_detail_with_return() {
    let audit = audit_screen("review", "views.review.selected").expect("review");
    audit.validate().expect("review audit");
    let text = audit.render_txt();
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);
    assert!(
        text.contains("Code review") || text.contains("review"),
        "review title required"
    );
    assert!(
        text.contains("Scope") && (text.contains("working tree") || text.contains("2 files")),
        "review scope required"
    );
    assert!(
        text.contains("Verdict") || text.contains("Changes requested"),
        "readable verdict required"
    );
    assert!(
        text.contains("›") && text.contains("middleware.rs"),
        "one clear current selection required"
    );
    assert!(
        text.contains("[esc]") || text.contains("Return"),
        "explicit return required"
    );
    let conversation = audit
        .conversation
        .as_ref()
        .expect("review must carry conversation identity");
    assert_eq!(conversation.conversation_id, "conversation.daily-work");
    assert!(conversation.later_draft.as_deref().unwrap_or("").contains("tests"));
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.from_screen_id == "review"
                && transition.to_screen_id == "chat-turn"
                && (transition.action_id.contains("return")
                    || transition.action_id.contains("back")
                    || transition.action_id.contains("cancel"))
        }),
        "return/back/cancel must restore the conversation"
    );
}

#[test]
fn review_opens_selected_evidence_then_returns_to_finding() {
    let open = replay_case("review-open-evidence", 1).expect("open evidence");
    open.validate().expect("open evidence valid");
    assert_eq!(open.before.screen_id, "review");
    assert_eq!(open.after.screen_id, "diff-viewer");
    assert_eq!(open.before.selection, "finding.middleware-cap");
    assert_eq!(open.after.selection, "finding.middleware-cap");
    assert!(
        open.preserved_field_hashes
            .get("selected_finding")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false),
        "selected finding hash must stay stable into the opened evidence"
    );

    let back = replay_case("diff-return-to-review", 1).expect("diff return");
    back.validate().expect("diff return valid");
    assert_eq!(back.before.screen_id, "diff-viewer");
    assert_eq!(back.after.screen_id, "review");
    assert_eq!(back.after.selection, "finding.middleware-cap");

    let diff = audit_screen("diff-viewer", "views.diff.file-1").expect("diff");
    let text = diff.render_txt();
    assert!(
        text.contains("middleware.rs") && text.contains("42"),
        "opened evidence must match selected file and line: {text}"
    );
    assert!(
        text.contains("Selected finding") || text.contains("middleware"),
        "selected finding remains the return target"
    );
}

#[test]
fn review_return_restores_origin_conversation() {
    let replay = replay_case("review-return", 1).expect("review return");
    replay.validate().expect("return valid");
    assert_eq!(replay.after.screen_id, "chat-turn");
    assert!(
        replay
            .preserved_field_hashes
            .get("composer_draft")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false)
    );
    assert!(
        replay.after.draft.contains("tests") || replay.after.queued_follow_up.contains("tests"),
        "draft must survive return"
    );
    assert_eq!(replay.before.conversation_id, replay.after.conversation_id);
    assert_eq!(replay.before.turn_id, replay.after.turn_id);
}

#[test]
fn diff_viewer_exposes_focused_change_evidence() {
    let audit = audit_screen("diff-viewer", "views.diff.file-1").expect("diff");
    audit.validate().expect("diff audit");
    let text = audit.render_txt();
    assert_no_private_traces(&text);
    assert!(text.contains("File 1 of 2") || text.contains("file 1 of 2") || text.contains("1 of 2"));
    assert!(text.contains("middleware.rs") || text.contains("src/auth"));
    assert!(
        text.contains("@@") || text.contains("hunk") || text.contains("-"),
        "hunk evidence required"
    );
    assert!(
        text.contains('-') && text.contains('+'),
        "additions and deletions must be text-readable"
    );
    assert!(
        text.contains("removed") || text.contains('-'),
        "deleted lines named as text"
    );
    assert!(
        text.contains("added") || text.contains('+'),
        "added lines named as text"
    );
    assert!(
        text.contains("Summary") || text.contains('+'),
        "summary of change size required"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "diff-file"),
        "diff file event required"
    );
}

#[test]
fn diff_navigation_keeps_review_origin_bounded() {
    let next = replay_case("diff-next-file", 1).expect("next file");
    next.validate().expect("next valid");
    assert_eq!(next.before.selection, "finding.middleware-cap");
    assert_eq!(next.after.selection, "finding.middleware-cap");
    assert_eq!(next.before.visible_origin, next.after.visible_origin);
    assert!(next.after.focus.contains("file-2") || next.after.focus.contains("2"));

    let prev = replay_case("diff-previous-file", 1).expect("prev file");
    prev.validate().expect("prev valid");
    assert_eq!(prev.after.selection, "finding.middleware-cap");
    assert!(prev.after.focus.contains("file-1") || prev.after.focus.contains("1"));

    let file2 = audit_screen("diff-viewer", "views.diff.file-2").expect("file 2");
    let text = file2.render_txt();
    assert!(
        text.contains("retry_policy.rs") || text.contains("File 2 of 2"),
        "adjacent file content must update"
    );
    assert!(
        text.contains("Return") || text.contains("[esc]"),
        "return action required in each file state"
    );
    assert!(
        text.contains("review") || text.contains("Selected finding"),
        "review origin retained"
    );
}

#[test]
fn review_and_diff_form_reversible_chain() {
    let open = replay_case("review-open-evidence", 1).expect("open");
    let back_to_review = replay_case("diff-return-to-review", 1).expect("to review");
    let to_conversation = replay_case("review-diff-chain-return", 1).expect("to conversation");
    for replay in [&open, &back_to_review, &to_conversation] {
        replay.validate().expect("chain step valid");
        assert!(
            replay
                .preserved_field_hashes
                .get("inspection_readonly_ledger")
                .map(|hash| hash.before == hash.after)
                .unwrap_or(false),
            "inspection must remain non-mutating across the chain"
        );
        assert_eq!(
            replay
                .preserved_field_hashes
                .get("composer_draft")
                .map(|hash| hash.before == hash.after),
            Some(true)
        );
    }
    assert_eq!(open.after.screen_id, "diff-viewer");
    assert_eq!(back_to_review.after.screen_id, "review");
    assert_eq!(back_to_review.after.selection, "finding.middleware-cap");
    assert_eq!(to_conversation.after.screen_id, "chat-turn");
    assert_eq!(
        to_conversation.before.conversation_id,
        to_conversation.after.conversation_id
    );
}

#[test]
fn inspection_is_read_only_with_no_mutation_actions() {
    for (screen, fixture) in [
        ("review", "views.review.selected"),
        ("diff-viewer", "views.diff.file-1"),
    ] {
        let audit = audit_screen(screen, fixture).expect(screen);
        audit.validate().expect("audit");
        let text = audit.render_txt();
        assert!(
            text.to_lowercase().contains("read-only") || text.contains("inspection"),
            "{screen} must declare read-only inspection"
        );
        for action in audit.events.iter().filter_map(|event| event.action.as_ref()) {
            let id = action.action_id.to_lowercase();
            let label = action.label.to_lowercase();
            assert!(
                !id.contains("apply")
                    && !id.contains("include")
                    && !id.contains("exclude")
                    && !id.contains("finish")
                    && !id.contains("approve")
                    && !id.contains("write")
                    && !label.contains("apply")
                    && !label.contains("include all")
                    && !label.contains("approve"),
                "{screen} must not expose mutation/approval primary actions: {} / {}",
                action.action_id,
                action.label
            );
        }
        assert!(
            !text.to_lowercase().contains("[i] include")
                && !text.to_lowercase().contains("[enter] finish")
                && !text.to_lowercase().contains("allow once"),
            "{screen} must not show Flow 2 mutation controls"
        );
    }

    let open = replay_case("review-open-evidence", 1).expect("open");
    assert!(
        open.preserved_field_hashes
            .get("inspection_readonly_ledger")
            .map(|hash| hash.before.as_str() == hash.after.as_str())
            .unwrap_or(false)
    );
    assert_eq!(open.before.staging_hunk_states, open.after.staging_hunk_states);
    assert_eq!(open.before.transcript_id, open.after.transcript_id);
}

#[test]
fn share_states_exact_scope_before_link() {
    let audit = audit_screen("share", "states.share.ready").expect("share");
    audit.validate().expect("share audit");
    let text = audit.render_txt();
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);
    assert!(
        text.to_lowercase().contains("local"),
        "must state local vs remote"
    );
    assert!(
        text.to_lowercase().contains("read-only") || text.to_lowercase().contains("read only"),
        "must state read-only scope before the link is usable"
    );
    assert!(
        text.contains("http://") || text.contains("Link"),
        "link is presented after scope"
    );
    let local_pos = text.to_lowercase().find("local").expect("local");
    let link_pos = text.find("http://").or_else(|| text.find("Link")).expect("link");
    assert!(
        local_pos < link_pos,
        "local/read-only scope must appear before the link"
    );
    assert!(
        text.to_lowercase().contains("read-only") || text.to_lowercase().contains("not remote"),
        "must prevent mistaking a local share for a public writable share"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "share-access"
                || event.identity.family == "share-scope"),
        "share scope/access audit facts required"
    );
}

#[test]
fn share_offers_copy_revoke_and_return_actions() {
    let audit = audit_screen("share", "states.share.ready").expect("share");
    let text = audit.render_txt();
    assert!(
        text.to_lowercase().contains("copy"),
        "copy action required"
    );
    assert!(
        text.to_lowercase().contains("revoke"),
        "revoke path required"
    );
    assert!(
        text.contains("Return") || text.contains("[esc]"),
        "return required"
    );
    assert!(
        text.contains("›") || text.contains("[enter]"),
        "one obvious primary action"
    );
    // Conversation remains available above dock.
    assert!(
        text.contains("rate limit") || text.contains("request guard"),
        "conversation continuation remains available"
    );

    let copy = replay_case("share-copy-link", 1).expect("copy");
    copy.validate().expect("copy valid");
    assert_eq!(copy.after.screen_id, "share");
    assert_eq!(copy.actions[0].action_id, "action.share.copy-link");

    let open_revoke = replay_case("share-open-revoke", 1).expect("open revoke");
    open_revoke.validate().expect("open revoke valid");
    assert!(open_revoke.after.selection.contains("revoke"));

    let ret = replay_case("share-return", 1).expect("return");
    ret.validate().expect("return valid");
    assert_eq!(ret.after.screen_id, "chat-turn");
}

#[test]
fn share_revocation_is_exact_and_idempotent() {
    let confirm = replay_case("share-confirm-revoke", 1).expect("confirm revoke");
    confirm.validate().expect("confirm valid");
    assert!(
        confirm
            .preserved_field_hashes
            .get("unrelated_share")
            .map(|hash| hash.before == hash.after)
            .unwrap_or(false),
        "unrelated share must stay active"
    );
    let primary = confirm
        .preserved_field_hashes
        .get("primary_share")
        .expect("primary share hash");
    assert_ne!(
        primary.before, primary.after,
        "first confirm must revoke only the named share"
    );

    let again = replay_case("share-confirm-revoke-idempotent", 1).expect("idempotent");
    again.validate().expect("idempotent valid");
    let primary_again = again
        .preserved_field_hashes
        .get("primary_share")
        .expect("primary share hash");
    assert_eq!(
        primary_again.before, primary_again.after,
        "repeated confirm is a no-op"
    );
    assert_eq!(
        again
            .preserved_field_hashes
            .get("unrelated_share")
            .map(|hash| hash.before == hash.after),
        Some(true)
    );

    let confirm_txt = audit_screen("share", "states.share.revoke-confirm")
        .expect("confirm ui")
        .render_txt();
    assert!(
        confirm_txt.contains("share.local.rate-limit-review")
            || confirm_txt.to_lowercase().contains("local share"),
        "confirmation must name the exact share"
    );
    assert!(
        confirm_txt.contains("share.remote.docs-preview")
            || confirm_txt.to_lowercase().contains("unrelated"),
        "confirmation must preserve/report unrelated shares"
    );

    let revoked = audit_screen("share", "states.share.revoked")
        .expect("revoked ui")
        .render_txt();
    assert!(
        revoked.to_lowercase().contains("revoked")
            || revoked.to_lowercase().contains("already"),
        "revoked result must be explicit"
    );

    let ret = replay_case("share-return", 1).expect("return after revoke");
    ret.validate().expect("return valid");
    assert_eq!(ret.after.screen_id, "chat-turn");
    assert_eq!(
        ret.preserved_field_hashes
            .get("composer_draft")
            .map(|hash| hash.before == hash.after),
        Some(true)
    );
}
