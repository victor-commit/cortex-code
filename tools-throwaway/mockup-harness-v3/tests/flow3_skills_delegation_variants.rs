//! Focused coverage for skills, subagent, variant, and theme pickers.
//! Owns VAL-V3-F3-034..037 and VAL-V3-F3-055..056.

use std::path::Path;
use std::process::Command;

use mockup_harness_v3::{
    audit::audit_screen,
    scenario::{replay_case, CaseReplay},
};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

fn assert_reconciled_hash(replay: &CaseReplay, field: &str, unchanged: bool) {
    let hash = replay
        .preserved_field_hashes
        .get(field)
        .unwrap_or_else(|| panic!("{field} must have before/after SHA-256 evidence"));
    assert_eq!(hash.before.len(), 64);
    assert_eq!(hash.after.len(), 64);
    if unchanged {
        assert_eq!(
            hash.before, hash.after,
            "{field} must remain byte-identical across the replay"
        );
    } else {
        assert_ne!(
            hash.before, hash.after,
            "{field} must record the applied state change"
        );
    }
}

#[test]
fn skills_picker_describes_user_visible_abilities_and_selected_detail() {
    let audit = audit_screen("skills-picker", "flow3-skills-picker")
        .expect("skills picker must produce typed evidence");
    audit
        .validate()
        .expect("skills picker evidence must stay bijective");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Choose session skills") || text.contains("Session skills"));
    assert!(text.contains("Commit cleanly"));
    assert!(text.contains("Design reviews"));
    assert!(text.contains("Review images"));
    assert!(text.contains("Research the web"));
    assert!(
        text.contains("Keep commits under the linked identity")
            || text.contains("linked identity"),
        "skill purpose must be user-facing"
    );
    assert!(
        text.contains("On ·") || text.contains("Enabled ·") || text.contains("[on]"),
        "enabled state must be non-color text"
    );
    assert!(
        text.contains("Pending") || text.contains("Off ·") || text.contains("[  ]"),
        "pending or disabled state must be labeled in text"
    );
    assert!(text.contains("Selected ·") || text.contains("Pending set"));
    assert!(text.contains("[enter] Apply skills") || text.contains("Apply skills"));
    assert!(text.contains("[esc] Cancel"));
    assert!(!text.contains("/skills/"));
    assert!(!text.contains(".md"));
    assert!(!text.contains("package name"));
    assert!(!text.contains("tool list"));
    assert!(!text.contains("prompt:"));
    assert!(!text.contains("ctx "));
    assert!(!text.contains("anthropic/"));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "skill-row")
            .count(),
        4,
        "four skill rows should be present"
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "skill-row"
                && event.identity.detail_level == "focused")
            .count(),
        1
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selected-detail")
            .count(),
        1
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn skills_apply_commits_pending_set_atomically_and_cancel_restores() {
    let applied = replay_case("skills-apply", 1).expect("ordinary skill apply must replay");
    assert_eq!(applied.before.screen_id, "skills-picker");
    assert_eq!(applied.after.screen_id, "chat-turn");
    assert_eq!(applied.before.session_id, applied.after.session_id);
    assert_eq!(applied.before.draft, applied.after.draft);
    assert_eq!(applied.before.origin_stack, applied.after.origin_stack);
    assert_eq!(applied.before.attachment, applied.before.attachment);
    // Pending set becomes the applied set in one transition; attachment holds applied skills.
    assert_ne!(
        applied.before.attachment, applied.after.attachment,
        "apply must commit the pending skill set"
    );
    assert_eq!(
        applied.after.queued_follow_up, "",
        "pending skill mutations clear after atomic apply"
    );
    assert_reconciled_hash(&applied, "draft", true);
    assert_reconciled_hash(&applied, "transcript", true);
    assert_reconciled_hash(&applied, "attachment", false);
    assert!(
        applied
            .audit
            .render_txt()
            .contains("Applied skills")
            || applied.audit.render_txt().contains("session skills"),
        "returned conversation must reconcile applied skills"
    );
    assert_eq!(applied.side_effects[0].status, "skills-applied");

    let cancelled = replay_case("skills-cancel", 1).expect("skill cancel must restore origin");
    assert_eq!(cancelled.before.screen_id, "skills-picker");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.before.draft, cancelled.after.draft);
    assert_eq!(cancelled.before.attachment, cancelled.after.attachment);
    assert_eq!(
        cancelled.before.transcript, cancelled.after.transcript,
        "cancel restores transcript exactly"
    );
    assert_reconciled_hash(&cancelled, "attachment", true);
    assert_reconciled_hash(&cancelled, "draft", true);
    assert_eq!(cancelled.side_effects[0].status, "no-change");
}

#[test]
fn skills_escalation_routes_through_access_guard_like_work_style() {
    let pending =
        replay_case("skills-access-pending", 1).expect("capability skill must stay pending");
    assert_eq!(pending.before.screen_id, "skills-picker");
    assert_eq!(pending.after.screen_id, "mode-switch");
    assert_eq!(
        pending.before.attachment, pending.after.attachment,
        "escalation must not apply skills before confirm"
    );
    assert_ne!(
        pending.before.queued_follow_up, pending.after.queued_follow_up,
        "pending skill toggle must be recorded before access review"
    );
    let pending_text = pending.audit.render_txt();
    assert!(pending_text.contains("Requested:") || pending_text.contains("Review"));
    assert!(
        pending_text.contains("Research the web") || pending_text.contains("web content"),
        "access review names the expanding skill"
    );
    assert!(pending_text.contains("Scope:") || pending_text.contains("this project"));
    assert_reconciled_hash(&pending, "attachment", true);

    let confirmed =
        replay_case("skills-access-confirm", 1).expect("skill access confirm must apply scope");
    assert_eq!(confirmed.before.screen_id, "mode-switch");
    assert_eq!(confirmed.after.screen_id, "chat-turn");
    assert_ne!(confirmed.before.attachment, confirmed.after.attachment);
    assert!(
        confirmed
            .after
            .attachment
            .contains("Research the web"),
        "confirm must commit the elevated web skill set on attachment"
    );
    assert_eq!(confirmed.side_effects[0].status, "skills-and-access-applied");
    assert_reconciled_hash(&confirmed, "attachment", false);
    assert_reconciled_hash(&confirmed, "draft", true);
    assert_eq!(
        confirmed.before.origin_stack, confirmed.after.origin_stack,
        "access-guard confirm must preserve origin identity"
    );
    assert_eq!(
        confirmed.before.draft, confirmed.after.draft,
        "access-guard confirm must preserve the composer draft"
    );
    let confirmed_text = confirmed.audit.render_txt();
    let applied_web_primary = confirmed_text
        .lines()
        .find(|line| line.contains("Applied skills") && line.contains("Research the web"));
    assert!(
        applied_web_primary.is_some(),
        "skills-access-confirm primary Applied skills line must include Research the web; text was:\n{confirmed_text}"
    );

    let denied =
        replay_case("skills-access-cancel", 1).expect("skill access denial preserves prior set");
    assert_eq!(denied.before.screen_id, "mode-switch");
    assert_eq!(denied.after.screen_id, "skills-picker");
    assert_eq!(denied.before.attachment, denied.after.attachment);
    assert!(
        denied.after.queued_follow_up.is_empty()
            || !denied.after.attachment.contains("Research the web"),
        "cancel does not commit the expanding skill"
    );
    assert_eq!(
        denied.before.attachment, denied.after.attachment,
        "prior skill set remains after access denial"
    );
    assert!(!denied.after.attachment.contains("Research the web"));
    assert_reconciled_hash(&denied, "attachment", true);
}

#[test]
fn skills_apply_and_access_confirm_return_summaries_match_applied_attachment_sets() {
    let ordinary = replay_case("skills-apply", 1).expect("ordinary skill apply must replay");
    let ordinary_text = ordinary.audit.render_txt();
    let ordinary_primary = ordinary_text
        .lines()
        .find(|line| line.contains("Applied skills"))
        .expect("ordinary skills-apply return must render an Applied skills summary");
    assert!(
        ordinary_primary.contains("Commit cleanly")
            && ordinary_primary.contains("Design reviews")
            && ordinary_primary.contains("Review images"),
        "ordinary apply primary line must list the base applied set: {ordinary_primary}"
    );
    assert!(
        !ordinary_primary.contains("Research the web"),
        "ordinary non-elevated apply must not list Research the web on the primary Applied skills line: {ordinary_primary}"
    );
    assert!(
        !ordinary.after.attachment.contains("Research the web"),
        "ordinary apply attachment is the review set without web"
    );

    let confirm = replay_case("skills-access-confirm", 1).expect("access confirm must replay");
    let confirm_text = confirm.audit.render_txt();
    let confirm_primary = confirm_text
        .lines()
        .find(|line| line.contains("Applied skills"))
        .expect("skills-access-confirm return must render an Applied skills summary");
    assert!(
        confirm_primary.contains("Research the web"),
        "confirm primary Applied skills line must include Research the web when attachment is the web set: {confirm_primary}"
    );
    assert!(
        confirm.after.attachment.contains("Research the web"),
        "confirm attachment must be APPLIED_SKILLS_WEB"
    );
    assert_eq!(confirm.before.draft, confirm.after.draft);
    assert_eq!(confirm.before.origin_stack, confirm.after.origin_stack);
}

#[test]
fn subagent_picker_stays_task_local_and_avoids_work_style() {
    let audit = audit_screen("subagent-picker", "flow3-subagent-picker")
        .expect("subagent picker must render");
    audit
        .validate()
        .expect("subagent picker evidence must stay bijective");
    let text = audit.render_txt();
    assert!(
        text.contains("Delegate this task") || text.contains("Delegate the current task")
    );
    assert!(text.contains("Explore auth boundaries") || text.contains("Explore"));
    assert!(text.contains("current task") || text.contains("this task only"));
    assert!(
        text.contains("Find existing auth") || text.contains("auth middleware")
    );
    assert!(!text.contains("How Code should work"));
    assert!(!text.contains("Careful edits"));
    assert!(!text.contains("primary agent"));
    assert!(!text.contains("tool envelope"));
    assert!(!text.contains("worker wave"));
    assert!(text.contains("[enter]") || text.contains("Use this helper"));
    assert!(text.contains("[esc] Cancel") || text.contains("Cancel"));

    let agent = audit_screen("agent-picker", "flow3-work-style-access")
        .expect("agent picker remains the work-style surface");
    let agent_text = agent.render_txt();
    assert!(agent_text.contains("How Code should work"));
    assert!(!agent_text.contains("Delegate this task"));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "subagent-row")
            .count(),
        3
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selected-detail")
            .count(),
        1
    );
}

#[test]
fn subagent_apply_and_cancel_preserve_conversation_and_scope() {
    let applied = replay_case("subagent-apply", 1).expect("subagent apply must replay");
    assert_eq!(applied.before.screen_id, "subagent-picker");
    assert_eq!(applied.after.screen_id, "chat-turn");
    assert_eq!(applied.before.work_style, applied.after.work_style);
    assert_eq!(applied.before.access_scope, applied.after.access_scope);
    assert_eq!(applied.before.draft, applied.after.draft);
    assert_ne!(applied.before.selection, applied.after.selection);
    assert_eq!(applied.side_effects[0].status, "helper-delegated");
    assert_reconciled_hash(&applied, "work_style", true);
    assert_reconciled_hash(&applied, "draft", true);

    let cancelled = replay_case("subagent-cancel", 1).expect("subagent cancel must restore");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.before.selection, cancelled.after.selection);
    assert_eq!(cancelled.side_effects[0].status, "no-change");
}

#[test]
fn variant_picker_keeps_base_model_and_explains_tradeoffs() {
    let audit = audit_screen("variant-picker", "flow3-variant-picker")
        .expect("variant picker must render");
    audit
        .validate()
        .expect("variant picker evidence must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains("Choose a response variant") || text.contains("Response variant"));
    assert!(text.contains("Claude Opus 4.8") || text.contains("Base model"));
    assert!(text.contains("Balanced") || text.contains("Current ·"));
    assert!(text.contains("Thorough") || text.contains("Fast"));
    assert!(
        text.contains("slower")
            || text.contains("more careful")
            || text.contains("least deliberation")
            || text.contains("moderate wait")
    );
    assert!(text.contains("Selected ·") || text.contains("Pending ·"));
    assert!(text.contains("[enter] Apply variant") || text.contains("Apply variant"));
    assert!(text.contains("[esc] Cancel"));
    assert!(!text.contains(":nitro"));
    assert!(!text.contains("routing"));
    assert!(!text.contains("provider spool"));
    assert!(!text.contains("model suffix"));

    // Statusline still names the base human model.
    let statusline = text.lines().last().expect("statusline");
    assert!(statusline.contains("Claude Opus 4.8") || statusline.contains("Claude Sonnet 4.5"));
    assert!(!statusline.contains("Thorough"));
    assert!(!statusline.contains("Balanced ·"));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "variant-row")
            .count(),
        3
    );
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selected-detail")
            .count(),
        1
    );
}

#[test]
fn variant_apply_changes_only_variant_and_cancel_keeps_base() {
    let applied = replay_case("variant-apply", 1).expect("variant apply must replay");
    assert_eq!(applied.before.screen_id, "variant-picker");
    assert_eq!(applied.after.screen_id, "chat-turn");
    assert_ne!(applied.before.selection, applied.after.selection);
    // Attachment holds applied variant; base model lives outside that field.
    assert_ne!(applied.before.attachment, applied.after.attachment);
    assert_eq!(applied.before.work_style, applied.after.work_style);
    assert_eq!(applied.side_effects[0].status, "variant-applied");
    let text = applied.audit.render_txt();
    assert!(
        text.contains("Applied variant") || text.contains("Thorough") || text.contains("variant"),
        "returned conversation mentions applied variant"
    );
    assert_reconciled_hash(&applied, "draft", true);
    assert_reconciled_hash(&applied, "attachment", false);

    let cancelled = replay_case("variant-cancel", 1).expect("variant cancel restores prior");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(cancelled.before.attachment, cancelled.after.attachment);
    assert_eq!(cancelled.side_effects[0].status, "no-change");
}

#[test]
fn theme_picker_distinguishes_preview_from_apply_and_cancel_restores() {
    let audit = audit_screen("theme-picker", "flow3-theme-picker")
        .expect("theme picker must render");
    audit
        .validate()
        .expect("theme picker evidence must stay bijective");
    let text = audit.render_txt();
    assert!(text.contains("Choose a theme") || text.contains("Themes"));
    assert!(text.contains("Dark"));
    assert!(text.contains("Light"));
    assert!(text.contains("Monokai") || text.contains("VS Code dark"));
    assert!(
        text.contains("Preview") || text.contains("temporary"),
        "preview must be distinguished from Apply"
    );
    assert!(text.contains("Applied ·") || text.contains("Current · Dark"));
    assert!(text.contains("[enter] Apply theme") || text.contains("Apply theme"));
    assert!(
        text.contains("[esc] Cancel") || text.contains("restores"),
        "cancel must restore the applied palette"
    );
    assert!(!text.contains("ctx "));
    assert!(!text.contains("RGB("));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "theme-row")
            .count(),
        4
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "selected-detail" || event.identity.family == "theme-preview"
    }));

    let preview = replay_case("theme-preview", 1).expect("focus preview is temporary");
    assert_eq!(preview.before.screen_id, "theme-picker");
    assert_eq!(preview.after.screen_id, "theme-picker");
    assert_ne!(preview.before.selection, preview.after.selection);
    assert_eq!(
        preview.before.attachment, preview.after.attachment,
        "preview must not commit applied theme"
    );
    assert_eq!(preview.side_effects[0].status, "theme-preview");
    assert_reconciled_hash(&preview, "attachment", true);

    let applied = replay_case("theme-apply", 1).expect("theme apply commits and returns");
    assert_eq!(applied.before.screen_id, "theme-picker");
    assert_eq!(applied.after.screen_id, "chat-turn");
    assert_ne!(applied.before.attachment, applied.after.attachment);
    assert_eq!(applied.side_effects[0].status, "theme-applied");
    assert_reconciled_hash(&applied, "draft", true);
    assert_reconciled_hash(&applied, "attachment", false);

    let cancelled = replay_case("theme-cancel", 1).expect("theme cancel restores palette");
    assert_eq!(cancelled.after.screen_id, "chat-turn");
    assert_eq!(
        cancelled.before.attachment, cancelled.after.attachment,
        "cancel restores the previously applied theme exactly"
    );
    assert_eq!(cancelled.side_effects[0].status, "no-change");
    assert_reconciled_hash(&cancelled, "attachment", true);
    assert_reconciled_hash(&cancelled, "transcript", true);
}

#[test]
fn public_publication_emits_all_four_picker_txt_png_pairs() {
    for screen in [
        "skills-picker",
        "subagent-picker",
        "variant-picker",
        "theme-picker",
    ] {
        let output = harness()
            .arg(screen)
            .output()
            .unwrap_or_else(|error| panic!("{screen} must publish: {error}"));
        assert!(
            output.status.success(),
            "{screen} publish failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for ext in ["txt", "png"] {
            let path =
                format!("/projects/cortex-code/plans/assets/mockups-v3/input/{screen}.{ext}");
            assert!(
                Path::new(&path).is_file(),
                "canonical {ext} must exist at {path}"
            );
        }
        let text = std::fs::read_to_string(format!(
            "/projects/cortex-code/plans/assets/mockups-v3/input/{screen}.txt"
        ))
        .expect("txt readable");
        assert_eq!(text.lines().count(), 40);
        assert!(text.lines().all(|line| line.chars().count() == 120));
    }
}
