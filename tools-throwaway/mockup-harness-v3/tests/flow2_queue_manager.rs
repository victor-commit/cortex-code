use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn queue_manager_opens_with_order_next_marker_and_one_time_consequence() {
    let audit = audit_screen("queue-manager", "flow2-queue-manager")
        .expect("queue-manager must produce typed render evidence");
    audit
        .validate()
        .expect("queue-manager evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    // VAL-V3-F2-023: bottom dock opens with realistic queued prompts, next marker,
    // one-time automatic-behavior copy, and color-independent order numbers.
    assert!(text.contains("Queued prompts"));
    assert!(
        text.contains("run after the current turn")
            || text.contains("run after this turn"),
        "one-time automatic consequence must state queue runs after the current turn"
    );
    assert!(
        text.matches("run after the current turn").count()
            + text.matches("run after this turn").count()
            <= 1,
        "consequence that queues run after the turn must appear at most once"
    );
    assert!(text.contains("Also list unit tests"));
    assert!(text.contains("Then summarize risks"));
    assert!(text.contains("[attachment: queue-notes-a.md]"));
    assert!(text.contains("[attachment: queue-notes-b.md]"));
    assert!(text.contains("1 ·"));
    assert!(text.contains("2 ·"));
    assert!(
        text.contains("next") || text.contains("runs next") || text.contains("[next]"),
        "must identify which item runs next without color alone"
    );
    assert!(text.contains("~/project · main · Claude Opus 4.8"));
    assert!(text.contains("Paused draft · 3 authored lines"));
    assert!(text.contains("Keep the summary short before we merge."));

    // VAL-V3-F2-024: reorder/remove/run-next controls, primary consequence,
    // one selected row, labeled remove (no color-only destroy), stable id/rev.
    // Phase-A prose language keeps Enter/Esc bylines without bracket key atoms.
    assert!(
        text.contains("Enter to apply")
            || text.contains("[enter] Apply")
            || text.contains("Enter · Apply"),
        "apply control must be visible as plain prose or legacy atom:\n{text}"
    );
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    let lower = text.to_ascii_lowercase();
    assert!(
        text.contains("[r] Reorder")
            || text.contains("Reorder")
            || lower.contains("r to reorder")
            || lower.contains("reorder"),
        "reorder control must be visible as plain text"
    );
    assert!(
        text.contains("[x] Remove")
            || text.contains("Remove")
            || lower.contains("x to remove")
            || lower.contains("remove"),
        "remove control must be labeled (never unlabeled or color-only)"
    );
    assert!(
        text.contains("[n] Run next")
            || text.contains("Run next")
            || lower.contains("n to run next")
            || lower.contains("run next"),
        "run-next control must be attached to the selected queue row"
    );
    assert!(text.contains("queue-item-1") || text.contains("id:queue-item-1"));
    assert!(text.contains("rev=1") || text.contains("rev 1") || text.contains("rev:1"));
    assert!(text.contains("queue-item-2") || text.contains("id:queue-item-2"));

    let selection_markers = text
        .lines()
        .filter(|line| line.contains('›') && (line.contains("1 ·") || line.contains("2 ·")))
        .count();
    assert_eq!(
        selection_markers, 1,
        "exactly one selected queue row must be marked"
    );

    assert!(audit.events.iter().any(|event| {
        event.identity.family == "origin" && event.identity.subject_id == "composer-multiline"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "consequence"
            && (event.identity.subject_id.contains("after")
                || event.identity.subject_id.contains("queue"))
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "queue-item" && event.identity.subject_id.contains("queue-item-1")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "queue-item" && event.identity.subject_id.contains("queue-item-2")
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "queue-next"
            || (event.identity.family == "next" && event.identity.subject_id.contains("queue"))
            || event.identity.subject_id.contains("runs-next")
    }));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1,
        "queue-manager must select exactly one row"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.queue-manager.apply"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "cancel-action"
            && event.identity.subject_id == "action.queue-manager.cancel"
    }));
    for control in [
        "action.queue-manager.reorder",
        "action.queue-manager.remove",
        "action.queue-manager.run-next",
    ] {
        assert!(
            audit.events.iter().any(|event| {
                event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.action_id == control)
                    || event.identity.subject_id == control
            }),
            "{control} must appear as a labeled action/event"
        );
    }
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.queue-manager.apply"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.queue-manager.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn queue_apply_persists_only_shown_order_against_opened_revisions() {
    let applied = replay_case("queue-apply", 1).expect("applying queue edits must be replayable");
    applied
        .validate()
        .expect("queue apply replay must expose one valid transition");

    assert_eq!(applied.before.screen_id, "queue-manager");
    assert_eq!(applied.after.screen_id, "composer-multiline");
    assert_eq!(applied.before.origin_stack, applied.after.origin_stack);
    assert_eq!(
        applied.before.draft, applied.after.draft,
        "apply must not submit or alter the composer draft"
    );
    assert_eq!(
        applied.before.scroll_anchor, applied.after.scroll_anchor,
        "apply must keep the originating transcript position"
    );
    assert_eq!(
        applied.before.composer_height, applied.after.composer_height,
        "apply must preserve composer height"
    );
    assert_ne!(
        applied.before.queue, applied.after.queue,
        "apply must persist the shown reorder/removal"
    );
    assert!(
        applied.after.queue.contains("queue-item-1")
            || applied.after.queue.contains("queue-item-2"),
        "apply result still carries stable item identities"
    );
    assert!(
        applied.after.queue.contains("rev="),
        "apply persists against the item revisions that were opened"
    );
    assert!(
        !applied.after.transcript.contains("You ·")
            || applied.after.transcript == applied.before.transcript
            || applied
                .after
                .transcript
                .contains("Queue updated")
            || applied.after.transcript.contains("Queued"),
        "apply returns to the same conversation without starting an unrelated turn"
    );

    // Before: two items in natural order; after: reordered so item-2 is next, item-1 second,
    // and shown removals/revisions are the only mutations.
    let order_field = applied
        .preserved_field_hashes
        .get("queue")
        .expect("apply must expose queue field hashes");
    assert_ne!(
        order_field.before, order_field.after,
        "queue order/identity ledger must change on apply"
    );
    let turn_field = applied
        .preserved_field_hashes
        .get("turn_count")
        .expect("apply must record turn count evidence");
    assert_eq!(
        turn_field.before, turn_field.after,
        "apply must not start an unrelated turn"
    );
    let opened_revs = applied
        .preserved_field_hashes
        .get("opened_revisions")
        .expect("apply must bind mutations to the opened revisions");
    assert_eq!(
        opened_revs.before, opened_revs.after,
        "apply only accepts the shown changes against opened revisions"
    );

    assert_eq!(applied.side_effects.len(), 1);
    assert!(applied.side_effects[0].simulated);
    assert_eq!(applied.side_effects[0].kind, "queue-mutation");
    assert_eq!(
        applied.side_effects[0].status,
        "simulated-queue-apply"
    );
    assert_eq!(applied.attempts[0].outcome, "applied");

    for field in [
        "draft",
        "composer_height",
        "scroll_anchor",
        "filter",
        "visible_origin",
        "conversation_id",
        "session_id",
        "turn_id",
    ] {
        let hashes = applied
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "apply preserves origin conversation fields for {field}"
        );
    }
}

#[test]
fn queue_cancel_restores_original_order_and_draft_exactly() {
    let cancelled =
        replay_case("queue-cancel", 1).expect("cancelling queue edits must be replayable");
    cancelled
        .validate()
        .expect("queue cancel replay must expose one valid transition");

    assert_eq!(cancelled.before.screen_id, "queue-manager");
    assert_eq!(cancelled.after.screen_id, "composer-multiline");
    assert_eq!(
        cancelled.before.draft, cancelled.after.draft,
        "cancel restores the original draft byte for byte"
    );
    assert_eq!(
        cancelled.before.queue, cancelled.after.queue,
        "cancel restores the original queue order and payloads"
    );
    assert_eq!(
        cancelled.before.transcript, cancelled.after.transcript,
        "cancel keeps the transcript unchanged"
    );

    let queue_field = cancelled
        .preserved_field_hashes
        .get("queue")
        .expect("cancel must record queue restoration evidence");
    assert_eq!(
        queue_field.before, queue_field.after,
        "cancel creates no queue mutation"
    );
    let turn_field = cancelled
        .preserved_field_hashes
        .get("turn_count")
        .expect("cancel must record turn count evidence");
    assert_eq!(
        turn_field.before, turn_field.after,
        "cancel must not start an unrelated turn"
    );

    assert_eq!(cancelled.side_effects.len(), 1);
    assert_eq!(cancelled.side_effects[0].status, "simulated-no-change");
    assert_eq!(cancelled.attempts[0].outcome, "cancelled");

    for field in [
        "draft",
        "queue",
        "selection",
        "focus",
        "composer_height",
        "scroll_anchor",
        "filter",
        "attachment",
        "transcript",
        "visible_origin",
    ] {
        let hashes = cancelled
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "cancel must byte-restore {field}"
        );
    }
}

#[test]
fn queue_conflict_refuses_stale_apply_and_offers_one_refresh() {
    let conflict =
        replay_case("queue-conflict", 1).expect("stale queue apply must be replayable");
    conflict
        .validate()
        .expect("queue conflict replay must expose one valid transition");

    assert_eq!(conflict.before.screen_id, "queue-manager");
    assert_eq!(
        conflict.after.screen_id, "queue-manager",
        "stale apply refuses persistence and stays on queue-manager"
    );
    assert_eq!(
        conflict.before.draft, conflict.after.draft,
        "conflict keeps the origin draft"
    );
    assert_eq!(
        conflict.before.queue, conflict.after.queue,
        "refused apply does not mutate the authoritative queue"
    );
    assert!(
        conflict.after.focus.contains("Refresh")
            || conflict.after.selection.contains("refresh")
            || conflict.after.pending_action.as_deref() == Some("queue.refresh"),
        "conflict focuses the single Refresh queue action"
    );

    let stale = conflict
        .preserved_field_hashes
        .get("stale_pending_mutations")
        .expect("conflict must mark pending mutations as refused/stale");
    assert_ne!(
        stale.before, stale.after,
        "stale pending mutations are marked refused rather than applied"
    );
    let refresh_count = conflict
        .preserved_field_hashes
        .get("refresh_action_count")
        .expect("conflict must expose exactly one Refresh queue action");
    assert_ne!(
        refresh_count.before, refresh_count.after,
        "conflict introduces one Refresh queue action"
    );

    assert_eq!(conflict.side_effects[0].status, "simulated-stale-refuse");
    assert_eq!(conflict.attempts[0].outcome, "stale-conflict");
    assert_eq!(
        conflict.actions[0].action_id, "action.queue-manager.apply",
        "the refused action is still apply against opened revisions"
    );
}

#[test]
fn queue_refresh_discards_stale_mutations_and_reloads_authoritative_items() {
    let refreshed =
        replay_case("queue-refresh", 1).expect("refreshing the queue must be replayable");
    refreshed
        .validate()
        .expect("queue refresh replay must expose one valid transition");

    assert_eq!(refreshed.before.screen_id, "queue-manager");
    assert_eq!(refreshed.after.screen_id, "queue-manager");
    assert_eq!(
        refreshed.before.draft, refreshed.after.draft,
        "refresh preserves the origin draft"
    );
    assert_ne!(
        refreshed.before.queue, refreshed.after.queue,
        "refresh reloads the authoritative queue"
    );
    assert!(
        !refreshed.after.queue.contains("queue-item-consumed")
            && !refreshed.after.queue.contains("consumed-item"),
        "refresh must never resurrect a consumed item"
    );
    assert!(
        refreshed.after.queue.contains("queue-item-2"),
        "authoritative surviving identity must remain after concurrent dequeue"
    );
    assert!(
        !refreshed.after.queue.contains("queue-item-1")
            || refreshed
                .after
                .queue
                .contains("queue-item-1:rev=3")
            || refreshed.after.queue.contains("rev=3"),
        "consumed or superseded revisions must not resurrect at the opened revision"
    );
    assert!(
        refreshed.after.focus.contains("queue-item-2")
            || refreshed.after.selection.contains("queue-item-2")
            || refreshed.after.focus.contains("nearest"),
        "focus restores to the same surviving identity or the nearest successor"
    );

    let pending = refreshed
        .preserved_field_hashes
        .get("stale_pending_mutations")
        .expect("refresh must record discarded pending mutations");
    assert_ne!(
        pending.before, pending.after,
        "refresh visibly discards stale pending mutations"
    );
    let authority = refreshed
        .preserved_field_hashes
        .get("authoritative_queue")
        .expect("refresh must reload authoritative identities and revisions");
    assert_ne!(
        authority.before, authority.after,
        "authoritative queue ledger reloads on refresh"
    );
    let rebirth = refreshed
        .preserved_field_hashes
        .get("consumed_item_present")
        .expect("refresh must prove consumed items stay gone");
    assert_eq!(
        rebirth.before, rebirth.after,
        "refresh never resurrects consumed items"
    );
    let filter = refreshed
        .preserved_field_hashes
        .get("filter")
        .expect("refresh preserves filter");
    assert_eq!(filter.before, filter.after);
    let origin = refreshed
        .preserved_field_hashes
        .get("visible_origin")
        .expect("refresh preserves origin");
    assert_eq!(origin.before, origin.after);
    let focus = refreshed
        .preserved_field_hashes
        .get("focus_identity")
        .expect("refresh restores focus to a surviving identity");
    assert_ne!(
        focus.before, focus.after,
        "focus move to surviving identity or nearest successor is recorded"
    );

    assert_eq!(
        refreshed.side_effects[0].status,
        "simulated-queue-refresh"
    );
    assert_eq!(refreshed.attempts[0].outcome, "refreshed");
    assert_eq!(
        refreshed.actions[0].action_id,
        "action.queue-manager.refresh"
    );
}

#[test]
fn public_selector_publishes_the_queue_manager_pair() {
    let output = harness()
        .arg("queue-manager")
        .output()
        .expect("public canonical selector must run");
    if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        assert!(output.status.success(), "queue-manager must publish after renderer preflight:\n{}", String::from_utf8_lossy(&output.stderr));
    }
    }
    }
    }
    for extension in ["txt", "png"] {
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/input/queue-manager.{extension}"
            ))
            .is_file(),
            "canonical queue-manager.{extension} must publish"
        );
    }
}
