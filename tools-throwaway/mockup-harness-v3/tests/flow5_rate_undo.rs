//! Flow 5 rate-limit and checkpoint-undo pilot coverage.
//!
//! Owned contract assertions:
//! VAL-V3-F5-032 through VAL-V3-F5-035, VAL-V3-F5-043 through VAL-V3-F5-047,
//! and VAL-V3-F5-058.

use mockup_harness_v3::{
    audit::audit_screen, flow_manifests, scenario::replay_case, TransitionKind,
};

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const RATE_TIMER: &str = "timer.rate-limit.wait";
const CHECKPOINT_NAME: &str = "Before risk-note expansion";
const AFFECTED_A: &str = "notes/release.md";
const AFFECTED_B: &str = "notes/risks.md";
const UNAFFECTED: &str = "drafts/later-note.md";

fn recovery_flow() -> &'static mockup_harness_v3::FlowManifest {
    flow_manifests()
        .iter()
        .find(|flow| flow.name == "recovery")
        .expect("recovery storyboard must exist")
}

#[test]
fn rate_limit_waits_automatically_with_saved_work_and_live_countdown() {
    let early = audit_screen("rate-limit", "rate-limit.wait.sample-early")
        .expect("early rate-limit sample must render");
    early
        .validate()
        .expect("early rate-limit audit must stay bijective");
    let early_text = early.render_txt();
    assert!(
        early_text.contains("saved") || early_text.contains("work is saved"),
        "rate-limit must state that work is saved"
    );
    assert!(
        early_text.contains("automatically") || early_text.contains("on its own"),
        "rate-limit must say retry will happen automatically"
    );
    assert!(
        early_text.contains("20s") || early_text.contains("20 s"),
        "early sample must include a live countdown"
    );
    assert_eq!(
        early
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        0,
        "automatic rate-limit must emit zero primary actions"
    );
    assert!(
        !early_text.contains("✗") && !early_text.to_lowercase().contains("failed"),
        "rate-limit must not be styled as a hard error"
    );

    let later = audit_screen("rate-limit", "rate-limit.wait.sample-later")
        .expect("later rate-limit sample must render");
    let later_text = later.render_txt();
    assert!(
        later_text.contains("12s") || later_text.contains("12 s"),
        "later sample must show a decreased countdown"
    );
    let early_seconds = extract_countdown_seconds(&early_text);
    let later_seconds = extract_countdown_seconds(&later_text);
    assert!(
        later_seconds < early_seconds,
        "countdown must decrease between deterministic samples: {early_seconds} -> {later_seconds}"
    );

    let gate = audit_screen("rate-limit", "gate.recovery.rate-limit")
        .expect("gate rate-limit must render");
    gate.validate()
        .expect("gate rate-limit audit must stay bijective");
}

#[test]
fn rate_limit_has_exactly_one_waiting_source_and_countdown() {
    let audit = audit_screen("rate-limit", "gate.recovery.rate-limit")
        .expect("rate-limit must render waiting ownsership");
    let text = audit.render_txt();
    assert_eq!(
        text.matches("○").count(),
        1,
        "exactly one waiting indicator is allowed"
    );
    let countdown_hits = text
        .lines()
        .filter(|line| {
            extract_optional_countdown(line).is_some()
                && (line.contains("retry in") || line.contains("s remaining"))
        })
        .count();
    assert_eq!(
        countdown_hits, 1,
        "exactly one countdown region must appear"
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "timer-owner")
            .count(),
        1
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "timer-owner"
                && event.identity.subject_id == RATE_TIMER
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.automatic && action.action_id.contains("rate-limit"))
        }),
        "one automatic timer-owner event is required"
    );
    assert!(!text.to_lowercase().contains("statusline countdown"));
    assert!(!text.contains("Toast"));
}

#[test]
fn rate_limit_manual_retry_is_visibly_secondary() {
    let audit =
        audit_screen("rate-limit", "gate.recovery.rate-limit").expect("rate-limit secondary retry");
    let text = audit.render_txt();
    assert!(
        text.contains("Retry now") || text.contains("optional"),
        "optional manual retry control must exist"
    );
    assert!(
        text.contains("optional") || text.contains("Optional"),
        "manual retry must be labeled optional"
    );
    assert!(!text.contains("› [enter] Retry now"));
    assert!(!text.contains("[enter] Retry now"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        0
    );
    assert!(
        audit.events.iter().any(|event| {
            event.identity.family == "secondary-action"
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.action_id == "action.rate-limit.retry-now")
        }),
        "retry-now must be secondary"
    );
    // The automatic waiting copy must appear before the manual control.
    let auto_row = text
        .lines()
        .position(|line| line.contains("automatically") || line.contains("on its own"))
        .expect("automatic wait copy");
    let retry_row = text
        .lines()
        .position(|line| line.contains("Retry now"))
        .expect("retry now");
    assert!(
        auto_row < retry_row,
        "automatic wait must rank above manual retry"
    );
}

#[test]
fn rate_limit_timer_retry_and_cancel_race_to_one_provider_attempt() {
    let success = replay_case("rate-limit-automatic-success", 1)
        .expect("timer expiry success must restore chat");
    assert_eq!(success.before.screen_id, "rate-limit");
    assert_eq!(success.after.screen_id, "chat-turn");
    assert!(success.actions[0].automatic);
    assert_eq!(success.actions[0].action_id, "action.rate-limit.timer-tick");
    assert_eq!(success.attempts[0].owner_id, RATE_TIMER);
    assert_eq!(success.attempts[0].count, 1);
    assert_eq!(success.attempts[0].outcome, "restored");
    assert_eq!(success.transitions.len(), 1);
    assert_eq!(success.before.user_task, RECOVERY_TASK);
    assert_eq!(success.after.user_task, RECOVERY_TASK);
    assert_eq!(success.before.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(success.after.visible_origin, RECOVERY_ORIGIN);
    assert_eq!(success.before.session_id, success.after.session_id);
    assert_eq!(success.before.draft, success.after.draft);
    assert_eq!(
        success.preserved_field_hashes["draft"].before,
        success.preserved_field_hashes["draft"].after
    );
    assert_eq!(
        success.preserved_field_hashes["transcript"].before,
        success.preserved_field_hashes["transcript"].after
    );
    assert_eq!(
        success.preserved_field_hashes["partial_response"].before,
        success.preserved_field_hashes["partial_response"].after
    );
    let after_text = success.audit.render_txt();
    assert!(after_text.contains("Reply to Code") || after_text.contains("›"));
    assert!(!after_text.contains("s remaining"));
    assert!(!after_text.contains("0s remaining"));

    let race_retry = replay_case("rate-limit-race-retry-wins", 1)
        .expect("same-tick retry-now must win once");
    assert_eq!(race_retry.transitions.len(), 1);
    assert_eq!(
        race_retry.actions[0].action_id,
        "action.rate-limit.retry-now"
    );
    assert_eq!(race_retry.attempts[0].count, 1);
    assert_eq!(race_retry.attempts[0].owner_id, RATE_TIMER);
    assert_eq!(race_retry.after.screen_id, "chat-turn");

    let race_cancel = replay_case("rate-limit-race-cancel-wins", 1)
        .expect("same-tick cancel must win once");
    assert_eq!(
        race_cancel.actions[0].action_id,
        "action.rate-limit.cancel"
    );
    assert_eq!(race_cancel.attempts[0].count, 0);
    assert_eq!(race_cancel.attempts[0].outcome, "cancelled");
    assert_eq!(race_cancel.after.screen_id, "rate-limit");
    assert!(
        race_cancel.audit.render_txt().contains("cancelled")
            || race_cancel.audit.render_txt().contains("stopped")
            || race_cancel
                .audit
                .render_txt()
                .contains("Automatic wait cancelled")
    );

    let retry_now = replay_case("rate-limit-retry-now", 1).expect("manual retry must succeed once");
    assert_eq!(retry_now.actions[0].action_id, "action.rate-limit.retry-now");
    assert!(!retry_now.actions[0].automatic);
    assert_eq!(retry_now.attempts[0].count, 1);
    assert_eq!(retry_now.after.screen_id, "chat-turn");
}

#[test]
fn checkpoint_undo_names_restored_checkpoint_once_and_lists_files() {
    let audit = audit_screen("checkpoint-undo", "gate.recovery.checkpoint-undo")
        .expect("checkpoint-undo must render");
    audit
        .validate()
        .expect("checkpoint-undo audit must stay bijective");
    let text = audit.render_txt();
    assert!(
        text.contains(CHECKPOINT_NAME),
        "undo must name the restored checkpoint"
    );
    assert!(
        text.contains("Restored") || text.contains("restored"),
        "confirmation must use plain past tense"
    );
    assert_eq!(
        text.matches(CHECKPOINT_NAME).count(),
        1,
        "checkpoint name must appear exactly once as the confirmation owner"
    );
    assert!(!text.to_lowercase().contains("toast"));
    assert!(!text.contains("Done") || text.contains(CHECKPOINT_NAME));
    assert!(text.contains("2 files") || text.contains("2 file"));
    assert!(text.contains(AFFECTED_A));
    assert!(text.contains(AFFECTED_B));
    assert!(!text.contains(UNAFFECTED));
    let path_list_lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains(AFFECTED_A) || line.contains(AFFECTED_B))
        .collect();
    assert_eq!(
        path_list_lines
            .iter()
            .filter(|line| line.contains(AFFECTED_A))
            .count(),
        1
    );
    assert_eq!(
        path_list_lines
            .iter()
            .filter(|line| line.contains(AFFECTED_B))
            .count(),
        1
    );
    let release_row = text
        .lines()
        .position(|line| line.contains(AFFECTED_A))
        .expect("release path");
    let risks_row = text
        .lines()
        .position(|line| line.contains(AFFECTED_B))
        .expect("risks path");
    let release_col = text
        .lines()
        .nth(release_row)
        .and_then(|line| line.find(AFFECTED_A))
        .expect("release path column");
    let risks_col = text
        .lines()
        .nth(risks_row)
        .and_then(|line| line.find(AFFECTED_B))
        .expect("risks path column");
    assert_eq!(
        release_col, risks_col,
        "affected paths must share one aligned column"
    );
}

#[test]
fn checkpoint_undo_preserves_unrelated_work_and_supports_symmetric_redo() {
    let undo = replay_case("checkpoint-undo-success", 1).expect("successful undo must replay");
    assert_eq!(undo.before.screen_id, "chat-turn");
    assert_eq!(undo.after.screen_id, "checkpoint-undo");
    assert_eq!(
        undo.actions[0].action_id,
        "action.checkpoint-undo.restore"
    );
    assert_eq!(undo.before.user_task, RECOVERY_TASK);
    assert_eq!(undo.after.user_task, RECOVERY_TASK);
    assert_eq!(undo.before.session_id, undo.after.session_id);
    assert_eq!(undo.before.draft, undo.after.draft);
    assert_eq!(
        undo.preserved_field_hashes["draft"].before,
        undo.preserved_field_hashes["draft"].after
    );
    assert_eq!(
        undo.preserved_field_hashes["transcript"].before,
        undo.preserved_field_hashes["transcript"].after
    );
    assert_eq!(
        undo.preserved_field_hashes["queue"].before,
        undo.preserved_field_hashes["queue"].after
    );
    assert!(
        undo.preserved_field_hashes["file.affected.notes/release.md"].before
            != undo.preserved_field_hashes["file.affected.notes/release.md"].after,
        "affected release file must change on undo"
    );
    assert!(
        undo.preserved_field_hashes["file.affected.notes/risks.md"].before
            != undo.preserved_field_hashes["file.affected.notes/risks.md"].after,
        "affected risks file must change on undo"
    );
    assert_eq!(
        undo.preserved_field_hashes["file.unaffected.drafts/later-note.md"].before,
        undo.preserved_field_hashes["file.unaffected.drafts/later-note.md"].after,
        "unaffected work must stay byte-identical"
    );
    assert!(
        undo.side_effects[0].status == "restored" || undo.side_effects[0].status == "applied"
    );
    let undo_text = undo.audit.render_txt();
    assert!(undo_text.contains(CHECKPOINT_NAME));
    assert!(
        undo_text.contains("Redo change") || undo_text.contains("[enter] Redo"),
        "valid redo must be adjacent"
    );
    assert!(
        undo
            .audit
            .events
            .iter()
            .any(|event| {
                event.identity.family == "primary-action"
                    && event
                        .action
                        .as_ref()
                        .is_some_and(|action| action.action_id == "action.checkpoint-undo.redo")
            })
    );

    let redo = replay_case("checkpoint-redo-success", 1).expect("symmetric redo must replay");
    assert_eq!(redo.before.screen_id, "checkpoint-undo");
    assert_eq!(redo.after.screen_id, "chat-turn");
    assert_eq!(
        redo.actions[0].action_id,
        "action.checkpoint-undo.redo"
    );
    assert_eq!(
        redo.preserved_field_hashes["file.affected.notes/release.md"].after,
        undo.preserved_field_hashes["file.affected.notes/release.md"].before,
        "redo must restore the pre-undo release file content"
    );
    assert_eq!(
        redo.preserved_field_hashes["file.affected.notes/risks.md"].after,
        undo.preserved_field_hashes["file.affected.notes/risks.md"].before,
        "redo must restore the pre-undo risks file content"
    );
    assert_eq!(
        redo.preserved_field_hashes["file.unaffected.drafts/later-note.md"].before,
        redo.preserved_field_hashes["file.unaffected.drafts/later-note.md"].after
    );
    assert_eq!(redo.after.focus, "composer");
    let redo_text = redo.audit.render_txt();
    assert!(redo_text.contains("Reply to Code") || redo_text.contains("composer") || redo_text.contains("›"));
}

#[test]
fn checkpoint_conflict_noop_and_stale_redo_are_safe() {
    let conflict =
        replay_case("checkpoint-undo-conflict", 1).expect("conflict must refuse writes");
    assert_eq!(conflict.before.screen_id, "checkpoint-undo");
    assert_eq!(conflict.after.screen_id, "checkpoint-undo");
    assert_eq!(
        conflict.actions[0].action_id,
        "action.checkpoint-undo.restore"
    );
    assert_eq!(conflict.side_effects[0].status, "conflict-refused");
    assert_eq!(
        conflict.preserved_field_hashes["file.affected.notes/release.md"].before,
        conflict.preserved_field_hashes["file.affected.notes/release.md"].after,
        "conflict must write zero affected bytes"
    );
    assert_eq!(
        conflict.preserved_field_hashes["file.affected.notes/risks.md"].before,
        conflict.preserved_field_hashes["file.affected.notes/risks.md"].after
    );
    assert_eq!(
        conflict.preserved_field_hashes["checkpoint"].before,
        conflict.preserved_field_hashes["checkpoint"].after,
        "checkpoint bag must remain available after conflict"
    );
    let conflict_text = conflict.audit.render_txt();
    assert!(conflict_text.contains("Review conflict") || conflict_text.contains("conflict"));
    assert!(
        conflict
            .audit
            .events
            .iter()
            .any(|event| {
                event.identity.family == "primary-action"
                    && event.action.as_ref().is_some_and(|action| {
                        action.action_id == "action.checkpoint-undo.review-conflict"
                    })
            })
    );

    let noop = replay_case("checkpoint-undo-noop", 1).expect("repeated undo must be no-op");
    assert_eq!(noop.side_effects[0].status, "no-op");
    assert_eq!(
        noop.preserved_field_hashes["file.affected.notes/release.md"].before,
        noop.preserved_field_hashes["file.affected.notes/release.md"].after
    );
    assert_eq!(
        noop.preserved_field_hashes["transcript"].before,
        noop.preserved_field_hashes["transcript"].after
    );
    let noop_text = noop.audit.render_txt();
    assert!(
        noop_text.contains("already restored")
            || noop_text.contains("Already restored")
            || noop_text.contains("no further changes")
    );
    // Exactly one confirmation phrase region; no duplicated confirmation blocks.
    assert!(
        noop_text.matches("Already restored").count()
            + noop_text.matches("already restored").count()
            + noop_text.matches("No further changes").count()
            + noop_text.matches("no further changes").count()
            <= 2
    );

    let stale = replay_case("checkpoint-redo-stale", 1)
        .expect("fresh mutation must invalidate redo");
    assert_eq!(stale.before.screen_id, "checkpoint-undo");
    assert_eq!(stale.after.screen_id, "checkpoint-undo");
    assert!(!stale.audit.render_txt().contains("[enter] Redo change"));
    assert!(
        !stale.audit.events.iter().any(|event| {
            event
                .action
                .as_ref()
                .is_some_and(|action| action.action_id == "action.checkpoint-undo.redo" && action.enabled)
        }),
        "stale redo must be disabled or removed"
    );

    let cont = replay_case("checkpoint-continue-chat", 1)
        .expect("continue must restore chat composer");
    assert_eq!(cont.before.screen_id, "checkpoint-undo");
    assert_eq!(cont.after.screen_id, "chat-turn");
    assert_eq!(
        cont.actions[0].action_id,
        "action.checkpoint-undo.continue"
    );
    assert_eq!(cont.after.focus, "composer");
    assert_eq!(cont.after.scroll_anchor, cont.before.scroll_anchor);
}

#[test]
fn recovery_flow_entries_bind_rate_limit_and_checkpoint_fixtures() {
    let flow = recovery_flow();
    assert_eq!(flow.steps[2].screen_id, "rate-limit");
    assert_eq!(flow.steps[2].fixture_id, "gate.recovery.rate-limit");
    assert_eq!(flow.steps[2].transition_kind, TransitionKind::Automatic);
    assert_eq!(flow.steps[3].screen_id, "chat-turn");
    assert_eq!(flow.steps[3].fixture_id, "gate.recovery.chat-turn");
    assert_eq!(flow.steps[4].screen_id, "checkpoint-undo");
    assert_eq!(
        flow.steps[4].fixture_id,
        "gate.recovery.checkpoint-undo"
    );
    for step in &flow.steps[2..=4] {
        assert_eq!(step.user_task, RECOVERY_TASK);
        assert_eq!(step.visible_origin, RECOVERY_ORIGIN);
        let audit = audit_screen(step.screen_id, step.fixture_id)
            .unwrap_or_else(|error| panic!("{} must render: {error}", step.screen_id));
        audit
            .validate()
            .unwrap_or_else(|error| panic!("{} must validate: {error}", step.screen_id));
        let text = audit.render_txt();
        if step.screen_id == "chat-turn" {
            assert!(
                text.contains(USER_PROMPT) || text.contains("release notes"),
                "recovery chat-turn must keep the paused conversation visible"
            );
            assert!(text.contains(PARTIAL_REPLY) || text.contains("retry window"));
        }
    }
}

fn extract_countdown_seconds(text: &str) -> u32 {
    text.lines()
        .find_map(extract_optional_countdown)
        .expect("countdown seconds in sample")
}

fn extract_optional_countdown(line: &str) -> Option<u32> {
    for token in line.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| !c.is_ascii_digit());
        if token.contains('s') && !trimmed.is_empty() {
            if let Ok(value) = trimmed.parse::<u32>() {
                if value > 0 && value < 3_600 {
                    return Some(value);
                }
            }
        }
    }
    None
}
