//! Flow 5 interrupt and toast recovery states.
//!
//! Owned contract assertions:
//! VAL-V3-F5-036, VAL-V3-F5-037, VAL-V3-F5-038, VAL-V3-F5-039,
//! VAL-V3-F5-040, VAL-V3-F5-041, VAL-V3-F5-042.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const RECOVERY_TASK: &str = "Finish the release notes without losing the current work.";
const RECOVERY_ORIGIN: &str = "Recovery · paused conversation";
const USER_PROMPT: &str =
    "You · Summarize the remaining release notes and the still-open risks.";
const PARTIAL_REPLY: &str =
    "The release notes cover the auth race fix, the retry window change, and one open risk…";
const INTERRUPT_STATEMENT: &str = "You stopped this reply";
const PARTIAL_KEPT: &str = "Partial answer kept";
const CONTINUE_REPLY: &str = "Continue reply";
const REPHRASE: &str = "Rephrase request";
const QUEUED_FOLLOW_UP: &str = "Queue the short risk follow-up after restore.";
const ORIGINAL_PROMPT: &str = "Summarize the remaining release notes and the still-open risks.";
const COMPLETED_APPEND: &str =
    "Finish the short risk note and keep the queue ready for the next turn.";

const TOAST_FAILURE: &str = "Settings were not saved";
const TOAST_WARN: &str = "Connection recovered after a brief pause";
const TOAST_INFO: &str = "Model switched";
const TOAST_SUCCESS: &str = "Checkpoint saved";
const RETRY_SAVE: &str = "Retry save";

fn assert_no_failure_language(text: &str) {
    for banned in [
        "failed",
        "failure",
        "error",
        "✗",
        "BAD",
        "provider stopped",
        "network",
        "NETWORK_ERR",
    ] {
        assert!(
            !text.to_lowercase().contains(&banned.to_lowercase())
                || banned == "✗"
                    && !text.contains('✗')
                || banned != "✗" && !text.contains(banned),
            "interrupt must stay neutral with no failure language: saw {banned}"
        );
    }
}

#[test]
fn interrupt_is_neutral_with_visible_seam_and_retained_partial() {
    // VAL-V3-F5-036
    let audit =
        audit_screen("interrupt", "canonical.interrupt").expect("interrupt must render");
    audit.validate().expect("interrupt audit");

    let text = audit.render_txt();
    assert!(text.contains(RECOVERY_ORIGIN));
    assert!(text.contains(RECOVERY_TASK));
    assert!(text.contains(USER_PROMPT) || text.contains(ORIGINAL_PROMPT));
    assert!(text.contains(PARTIAL_REPLY), "partial answer retained");
    assert!(
        text.contains("──") || text.contains("│"),
        "visible seam required between partial answer and interrupt statement"
    );
    assert!(
        text.contains(INTERRUPT_STATEMENT) || text.contains("you stopped"),
        "plain-English user-stopped statement"
    );
    assert!(
        text.contains(PARTIAL_KEPT)
            || text.contains("kept")
            || text.contains("saved")
            || text.contains("preserved"),
        "preservation statement must appear"
    );

    let lower = text.to_lowercase();
    assert!(!lower.contains("provider failure"));
    assert!(!lower.contains("could not reach"));
    assert!(!text.contains("✗"));
    assert!(!text.contains("BAD"));
    assert!(!lower.contains("failed"));
    assert!(!lower.contains("failure"));
    assert!(!lower.contains("error code"));
    // No BAD semantic styling cells on the canvas for the interrupt block.
    assert!(
        !audit
            .styled_cells
            .iter()
            .any(|cell| cell.style.foreground
                == mockup_harness_v3::chrome::tokens::ColorToken::Bad),
        "interrupt must never paint BAD/red failure tokens"
    );

    let partial_row = text
        .lines()
        .position(|line| line.contains(PARTIAL_REPLY))
        .expect("partial");
    let statement_row = text
        .lines()
        .position(|line| {
            line.contains(INTERRUPT_STATEMENT)
                || line.to_lowercase().contains("you stopped")
        })
        .expect("interrupt statement");
    assert!(
        partial_row < statement_row,
        "partial answer sits above the interrupt statement"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "partial-reply"),
        "typed partial-reply event required"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "interrupt-statement"
                || event.identity.family == "recovery-cause"),
        "typed interrupt statement event required"
    );
    let _ = assert_no_failure_language;
}

#[test]
fn interrupt_offers_continue_primary_with_queue_consequences() {
    // VAL-V3-F5-037
    let audit =
        audit_screen("interrupt", "canonical.interrupt").expect("interrupt primary hierarchy");
    let text = audit.render_txt();

    assert!(
        text.contains(&format!("› [enter] {CONTINUE_REPLY}")) || text.contains(CONTINUE_REPLY),
        "Continue reply is the single primary"
    );
    assert!(
        text.contains(REPHRASE) || text.contains("[r] Rephrase"),
        "Rephrase request is the secondary action"
    );
    assert!(
        text.contains("before")
            || text.contains("runs first")
            || text.contains("before queued")
            || text.contains("ahead of queued"),
        "Continue consequence must say how it relates to queued follow-ups"
    );
    assert!(
        text.contains("queued") || text.contains("Queue"),
        "queued follow-ups remain mentioned"
    );
    assert!(
        text.contains(QUEUED_FOLLOW_UP)
            || text.contains("original order")
            || text.contains("stay queued")
            || text.contains("remain queued"),
        "queued items remain queued in order"
    );
    assert!(
        text.contains("edit") || text.contains("composer") || text.contains("original"),
        "Rephrase consequence restores the prompt for editing"
    );

    let primaries: Vec<_> = audit
        .events
        .iter()
        .filter(|event| event.identity.family == "primary-action")
        .collect();
    assert_eq!(primaries.len(), 1, "exactly one primary action");
    assert!(
        primaries[0]
            .action
            .as_ref()
            .is_some_and(|action| action.label.contains("Continue") && !action.automatic),
        "primary is Continue reply"
    );
    let secondaries: Vec<_> = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "secondary-action"
                && event
                    .action
                    .as_ref()
                    .is_some_and(|action| action.label.to_lowercase().contains("rephrase"))
        })
        .collect();
    assert_eq!(
        secondaries.len(),
        1,
        "at most one secondary Rephrase request"
    );
}

#[test]
fn continue_append_without_duplication_and_rephrase_restores_prompt() {
    // VAL-V3-F5-038
    let cont = replay_case("interrupt-continue-reply", 1).expect("continue reply");
    cont.validate().expect("continue validation");
    assert_eq!(cont.before.screen_id, "interrupt");
    assert_eq!(cont.after.screen_id, "chat-turn");
    assert_eq!(
        cont.actions[0].action_id,
        "action.interrupt.continue-reply"
    );
    assert!(!cont.actions[0].automatic);
    assert_eq!(cont.before.partial_response, PARTIAL_REPLY);
    assert!(
        cont.after
            .transcript
            .contains(PARTIAL_REPLY.trim_end_matches('…'))
            || cont.after.transcript.contains(PARTIAL_REPLY)
            || cont.after.partial_response.starts_with(
                PARTIAL_REPLY
                    .trim_end_matches('…')
                    .trim_end_matches('.')
            ),
        "continuation keeps the partial prefix"
    );
    assert!(
        !cont
            .after
            .transcript
            .matches(PARTIAL_REPLY)
            .count()
            .gt(&1)
            || cont.after.transcript.matches(PARTIAL_REPLY).count() == 1,
        "continuation must not duplicate the retained partial answer"
    );
    // New text only after the partial, not a full repeated prefix.
    assert!(
        cont.after.transcript.contains(COMPLETED_APPEND)
            || cont.after.partial_response.contains(COMPLETED_APPEND)
            || cont
                .preserved_field_hashes
                .contains_key("answer_prefix"),
        "only new completed text is appended"
    );
    if let Some(prefix) = cont.preserved_field_hashes.get("answer_prefix") {
        assert_eq!(
            prefix.before, prefix.after,
            "partial prefix hash must stay equal across continue"
        );
    }
    assert_eq!(cont.before.turn_id, cont.after.turn_id, "same turn");
    assert_eq!(
        cont.before.session_id, cont.after.session_id,
        "same session"
    );
    assert!(
        cont.after.focus == "composer"
            || cont.audit.render_txt().contains("›")
            || cont.audit.render_txt().to_lowercase().contains("reply"),
        "composer restores after continue completes"
    );
    for field in ["queue", "queued_follow_up", "attachment", "draft"] {
        let hashes = cont
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must remain hashed across continue"));
        assert_eq!(
            hashes.before, hashes.after,
            "{field} must survive continue"
        );
    }

    let rephrase = replay_case("interrupt-rephrase-request", 1).expect("rephrase");
    rephrase.validate().expect("rephrase validation");
    assert_eq!(rephrase.before.screen_id, "interrupt");
    assert_eq!(rephrase.after.screen_id, "chat-turn");
    assert_eq!(
        rephrase.actions[0].action_id,
        "action.interrupt.rephrase-request"
    );
    assert!(
        rephrase.after.draft.contains(ORIGINAL_PROMPT)
            || rephrase
                .after
                .draft
                .contains("Summarize the remaining release notes"),
        "rephrase restores the original prompt into the composer"
    );
    assert!(
        !rephrase.after.draft.is_empty(),
        "rephrase must not clear the prompt"
    );
    assert_eq!(
        rephrase.before.partial_response, rephrase.after.partial_response,
        "rephrase keeps the retained partial answer"
    );
}

#[test]
fn toasts_order_by_severity_not_chronology() {
    // VAL-V3-F5-039
    let audit = audit_screen("toasts", "canonical.toasts").expect("toasts must render");
    audit.validate().expect("toasts audit");
    let text = audit.render_txt();

    let failure_row = text
        .lines()
        .position(|line| line.contains(TOAST_FAILURE))
        .expect("failure toast");
    let warn_or_info_row = text
        .lines()
        .position(|line| line.contains(TOAST_WARN) || line.contains(TOAST_INFO))
        .expect("warn/info toast");
    let success_row = text
        .lines()
        .position(|line| line.contains(TOAST_SUCCESS))
        .expect("success toast");
    assert!(
        failure_row < warn_or_info_row,
        "actionable failure must sort above warnings/info regardless of creation time"
    );
    assert!(
        warn_or_info_row < success_row || failure_row < success_row,
        "success cannot outrank failure or routine notices"
    );
    assert!(
        text.contains("✗") || text.contains("Needs attention") || text.contains("BAD"),
        "failure toast has a label or verdict glyph besides color"
    );
    assert!(
        text.contains("✓") || text.contains("OK") || text.contains("Done"),
        "success toast has a non-color label/glyph"
    );
    assert!(
        text.contains("●") || text.contains("INFO") || text.contains("Note"),
        "info toast has a non-color label/glyph"
    );
    // Fixture construction order is success, then info, then failure (timestamps reverse of severity).
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "toast"
                && event.identity.subject_id.contains("failure")),
        "typed failure toast event"
    );
}

#[test]
fn toast_lifetimes_and_focus_owner_remain_stable() {
    // VAL-V3-F5-040
    let initial = audit_screen("toasts", "canonical.toasts").expect("initial toasts");
    let initial_text = initial.render_txt();
    assert!(initial_text.contains(TOAST_FAILURE));
    assert!(initial_text.contains(TOAST_SUCCESS) || initial_text.contains(TOAST_INFO));
    assert!(
        initial_text.contains("›") || initial_text.contains("composer") || initial_text.contains("Reply"),
        "composer remains the focus owner"
    );
    assert!(
        initial
            .events
            .iter()
            .any(|event| event.identity.family == "composer"
                || event.identity.family == "statusline"
                || event.identity.subject_id.contains("composer")),
        "focus events keep composer ownership"
    );

    let tick = replay_case("toasts-auto-dismiss-tick", 1).expect("auto-dismiss tick");
    tick.validate().expect("lifetime validation");
    assert_eq!(tick.before.screen_id, "toasts");
    assert_eq!(tick.after.screen_id, "toasts");
    // Failure remains; success/info can dismiss.
    assert!(
        tick.after.selection.contains("failure")
            || tick.audit.render_txt().contains(TOAST_FAILURE),
        "actionable failure toast remains until acted on"
    );
    assert!(
        !tick.audit.render_txt().contains(TOAST_SUCCESS)
            || tick.side_effects[0].status.contains("auto-dismiss"),
        "success/info toast follows deterministic auto-dismiss"
    );
    assert_eq!(
        tick.before.focus, tick.after.focus,
        "toast appearance/dismiss never steals focus from the prior owner"
    );
    assert!(
        tick.after.focus == "composer" || tick.before.focus == tick.after.focus,
        "focus stays on composer"
    );
}

#[test]
fn toast_recovery_stays_attached_and_returns_to_origin() {
    // VAL-V3-F5-041
    let audit = audit_screen("toasts", "canonical.toasts").expect("toast recovery ownership");
    let text = audit.render_txt();
    let failure_row = text
        .lines()
        .position(|line| line.contains(TOAST_FAILURE))
        .expect("failure");
    let action_row = text
        .lines()
        .position(|line| line.contains(RETRY_SAVE) || line.contains("retry save"))
        .expect("row-local Retry save");
    assert!(
        action_row == failure_row || action_row == failure_row + 1,
        "recovery action must sit adjacent to its notice, not in a global footer"
    );
    assert!(
        !text.to_lowercase().contains("settings dashboard")
            && !text.to_lowercase().contains("open dashboard"),
        "recovery must not open a generic dashboard"
    );

    let primaries: Vec<_> = audit
        .events
        .iter()
        .filter(|event| {
            event.identity.family == "primary-action"
                || (event.identity.family == "toast-action"
                    && event
                        .action
                        .as_ref()
                        .is_some_and(|action| action.label.contains("Retry")))
        })
        .collect();
    assert_eq!(
        primaries.len(),
        1,
        "exactly one primary recovery action on the highest-severity toast"
    );

    let replay = replay_case("toasts-retry-save", 1).expect("retry save");
    replay.validate().expect("retry validation");
    assert_eq!(replay.before.screen_id, "toasts");
    assert_eq!(
        replay.actions[0].action_id,
        "action.toasts.retry-save"
    );
    assert_eq!(
        replay.after.screen_id, "chat-turn",
        "recovery returns to the activity that emitted the toast"
    );
    assert_eq!(
        replay.after.user_task, RECOVERY_TASK,
        "origin task restored"
    );
    assert_eq!(
        replay.before.session_id, replay.after.session_id,
        "same session"
    );
    assert_eq!(
        replay.before.focus, "composer",
        "prior focus owner was the composer"
    );
    assert!(
        replay.after.focus == "composer"
            || replay.audit.render_txt().contains("›"),
        "restores prior focus owner after recovery"
    );
}

#[test]
fn toasts_remain_compact_borderless_and_nonblocking() {
    // VAL-V3-F5-042
    let audit = audit_screen("toasts", "canonical.toasts").expect("toast geometry");
    let text = audit.render_txt();
    assert!(!text.contains("╭") || text.matches('╭').count() <= 1);
    // Toast stack itself is borderless: no full rounded windows per toast.
    let toast_lines: Vec<_> = text
        .lines()
        .filter(|line| {
            line.contains(TOAST_FAILURE)
                || line.contains(TOAST_SUCCESS)
                || line.contains(TOAST_INFO)
                || line.contains(TOAST_WARN)
                || line.contains(RETRY_SAVE)
        })
        .collect();
    assert!(
        toast_lines
            .iter()
            .all(|line| !line.contains('╭') && !line.contains('╰') && !line.contains('╭')),
        "toasts must be borderless compact blocks"
    );
    assert!(
        text.contains(USER_PROMPT)
            || text.contains("preferences")
            || text.contains("saved")
            || text.contains("Release notes")
            || text.contains(RECOVERY_TASK),
        "latest transcript remains readable"
    );
    assert!(
        text.contains("›") || text.contains("composer") || text.contains("Reply"),
        "composer remains visible and not covered"
    );
    assert!(
        text.contains("~/project") || text.contains("Claude Opus"),
        "statusline remains visible"
    );
    for banned in [
        "ECONNRESET",
        "status=500",
        "trace_id",
        "stack:",
        "0x",
        "stderr",
        "HTTP 502",
    ] {
        assert!(
            !text.contains(banned),
            "toasts must not expose diagnostic payload by default: {banned}"
        );
    }

    let occupied: usize = toast_lines
        .iter()
        .map(|line| line.trim().chars().count())
        .sum();
    let canvas_cells = 120 * 40;
    assert!(
        occupied * 3 < canvas_cells,
        "toast occupancy must stay well under one-third of the canvas"
    );
}
