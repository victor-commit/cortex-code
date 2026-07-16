//! Theme galleries, status detail, empty origin (VAL-V3-F3-038..040, 042..044).

use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

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
        "anthropic/",
        "claude-opus-4-8",
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
        status.contains("~/project")
            && (status.contains("Claude Opus 4.8") || status.contains("Claude Sonnet 4.5")),
        "minimal statusline required: {status}"
    );
    assert!(
        !status.contains('%') && !status.contains('$') && !status.contains("idle"),
        "statusline must omit cost/context/idle"
    );
}

fn normalize_gallery(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dark_theme_gallery_is_readable_comparison() {
    let audit = audit_screen("theme-gallery-dark", "views.theme-gallery.dark")
        .expect("theme-gallery-dark must produce typed evidence");
    audit.validate().expect("dark gallery audit");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);

    assert!(text.contains("Theme gallery") || text.contains("theme gallery"));
    assert!(text.contains("Dark") && text.contains("Light"));
    assert!(text.contains("VS Code dark") || text.contains("vscode"));
    assert!(text.contains("Monokai") && text.contains("Ocean"));
    assert!(text.contains("cortex default") && text.contains("daytime"));
    assert!(text.contains("active") || text.contains("Selected"));
    assert!(text.contains("███"), "swatch samples required");
    assert!((text.contains("Esc to") || text.contains("[esc]")) || text.contains("Return"));
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "theme-row"),
        "theme rows as events"
    );
    assert!(
        audit.transitions.iter().any(|transition| {
            transition.action_id.contains("return")
                && transition.to_screen_id == "theme-picker"
        }),
        "return to theme picker required"
    );
}

#[test]
fn light_and_dark_galleries_have_content_parity() {
    let dark = audit_screen("theme-gallery-dark", "views.theme-gallery.selected")
        .expect("dark gallery");
    let light = audit_screen("theme-gallery-light", "views.theme-gallery.selected")
        .expect("light gallery");
    dark.validate().expect("dark valid");
    light.validate().expect("light valid");

    let dark_txt = normalize_gallery(&dark.render_txt());
    let light_txt = normalize_gallery(&light.render_txt());
    assert_eq!(
        dark_txt, light_txt,
        "normalized light/dark gallery text must match exactly"
    );

    // Geometry: candidate rows mirror across schemes.
    assert_eq!(dark.candidates.len(), light.candidates.len());
    for (left, right) in dark.candidates.iter().zip(light.candidates.iter()) {
        assert_eq!(left.identity.family, right.identity.family);
        assert_eq!(left.identity.subject_id, right.identity.subject_id);
        assert_eq!(
            left.identity.render_refs, right.identity.render_refs,
            "layout geometry must match"
        );
    }

    // Light canvas cells report light absolute backgrounds.
    let light_ground = light
        .styled_cells
        .iter()
        .filter(|cell| cell.glyph == ' ')
        .filter_map(|cell| cell.absolute_bg)
        .next();
    assert_eq!(
        light_ground,
        Some((0xFA, 0xF7, 0xF2)),
        "light gallery canvas must use the approved light ground"
    );
    let dark_ground = dark
        .styled_cells
        .iter()
        .filter(|cell| cell.glyph == ' ')
        .find_map(|cell| cell.absolute_bg);
    assert!(
        dark_ground.is_none(),
        "dark gallery avoids absolute light bleed"
    );
}

#[test]
fn theme_gallery_returns_without_losing_context() {
    for (open_case, return_case, gallery) in [
        (
            "theme-gallery-open-dark",
            "theme-gallery-return-dark",
            "theme-gallery-dark",
        ),
        (
            "theme-gallery-open-light",
            "theme-gallery-return-light",
            "theme-gallery-light",
        ),
    ] {
        let open = replay_case(open_case, 1).expect(open_case);
        open.validate().expect("open valid");
        assert_eq!(open.before.screen_id, "theme-picker");
        assert_eq!(open.after.screen_id, gallery);
        assert_eq!(open.before.selection, "light");
        assert_eq!(open.after.selection, "light");
        assert_eq!(
            open.before.pending_action.as_deref(),
            Some("pending.theme.light")
        );
        assert_eq!(
            open.after.pending_action.as_deref(),
            Some("pending.theme.light")
        );
        assert!(
            open.preserved_field_hashes
                .get("composer_draft")
                .map(|hash| hash.before == hash.after)
                .unwrap_or(false)
                || open.before.draft == open.after.draft,
            "draft and pending must survive gallery open"
        );

        let back = replay_case(return_case, 1).expect(return_case);
        back.validate().expect("return valid");
        assert_eq!(back.before.screen_id, gallery);
        assert_eq!(back.after.screen_id, "theme-picker");
        assert_eq!(back.after.selection, "light");
        assert_eq!(
            back.after.pending_action.as_deref(),
            Some("pending.theme.light")
        );
        assert!(
            back.after.focus.contains("theme") || back.after.focus.contains("light"),
            "pending focus restored on the picker"
        );
        // Subsequent apply/cancel still follow theme-picker semantics.
        let apply = replay_case("theme-apply", 1).expect("theme apply after gallery");
        assert_eq!(apply.before.screen_id, "theme-picker");
        assert_eq!(apply.after.screen_id, "chat-turn");
        let cancel = replay_case("theme-cancel", 1).expect("theme cancel after gallery");
        assert_eq!(cancel.before.screen_id, "theme-picker");
        assert_eq!(cancel.after.screen_id, "chat-turn");
    }
}

#[test]
fn status_detail_answers_configuration_questions() {
    let audit = audit_screen("status", "views.status.default").expect("status");
    audit.validate().expect("status audit");
    let text = audit.render_txt();
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);
    assert!(text.contains("Session status") || text.contains("status"));
    assert!(text.contains("Claude Opus 4.8"));
    assert!(text.contains("How Code should work") || text.contains("Careful edits"));
    assert!(text.contains("~/project") || text.contains("Project"));
    assert!(
        text.contains("Needs attention") || text.contains("Needs sign-in"),
        "actionable issues required"
    );
    assert!(text.contains("Return") || (text.contains("Esc to") || text.contains("[esc]")));
    assert!(
        text.contains("Advanced raw metadata stays hidden")
            || text.contains("open Details")
            || text.contains("d for Details"),
        "advanced disclosure behind default"
    );
    assert!(
        !text.contains("wf-") && !text.contains("token"),
        "default must not lead with IDs or token metrics"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "model-summary"),
        "readable model event required"
    );
    assert!(
        audit
            .events
            .iter()
            .any(|event| event.identity.family == "work-style-summary"),
        "unified work-style event required"
    );

    let advanced = audit_screen("status", "views.status.advanced").expect("advanced status");
    let advanced_text = advanced.render_txt();
    assert!(
        advanced_text.contains("Advanced detail") || advanced_text.contains("Provider"),
        "advanced detail must add useful content"
    );
    assert!(
        advanced
            .events
            .iter()
            .any(|event| event.identity.family == "advanced-detail"),
        "advanced event required"
    );
}

#[test]
fn empty_state_invites_one_clear_start() {
    let audit = audit_screen("empty", "views.empty.origin").expect("empty");
    audit.validate().expect("empty audit");
    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));
    assert_no_private_traces(&text);
    assert_statusline_minimal(&text);

    assert!(text.contains("No messages yet") || text.contains("Start a new conversation"));
    assert!(
        text.contains("Ask Code to start") || text.contains("›"),
        "dominant composer start required"
    );
    assert!(
        text.contains("resume") || text.contains("Resume") || text.contains("/resume"),
        "subordinate resume entry"
    );
    assert!(
        text.contains("/model") || text.contains("work style") || text.contains("/theme"),
        "subordinate configuration entry"
    );
    assert!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count()
            == 1
            || text.matches("›").count() >= 1,
        "one dominant primary action"
    );
}

#[test]
fn empty_configuration_returns_to_empty() {
    let open_model = replay_case("empty-open-model", 1).expect("open model");
    open_model.validate().expect("open model valid");
    assert_eq!(open_model.before.screen_id, "empty");
    assert_eq!(open_model.after.screen_id, "model-picker");
    assert!(open_model.after.origin_stack.contains(&"empty".to_owned()));
    assert_eq!(open_model.before.draft, open_model.after.draft);

    let cancel = replay_case("empty-model-cancel", 1).expect("model cancel");
    cancel.validate().expect("cancel valid");
    assert_eq!(cancel.after.screen_id, "empty");
    assert_eq!(cancel.after.draft, "Ask Code to start a task");
    assert!(cancel.after.transcript.is_empty());

    let apply = replay_case("empty-model-apply", 1).expect("model apply");
    apply.validate().expect("apply valid");
    assert_eq!(apply.after.screen_id, "empty");
    assert!(
        apply.after.attachment.contains("Sonnet") || apply.after.attachment.contains("model="),
        "only the selected setting changed"
    );
    assert!(apply.after.transcript.is_empty());
    // VAL-V3-F3-046: after paint, statusline, model_choice.applied, and the
    // model-applied side effect must share one applied Sonnet truth — never the
    // stock empty Opus fixture.
    let apply_text = apply.audit.render_txt();
    let apply_status = apply_text.lines().last().expect("statusline");
    assert!(
        apply_text.contains("Ready · Claude Sonnet 4.5"),
        "after-apply Ready model must paint Claude Sonnet 4.5, got:\n{apply_text}"
    );
    assert!(
        apply_status.contains("Claude Sonnet 4.5"),
        "after-apply statusline must paint Claude Sonnet 4.5: {apply_status}"
    );
    assert!(
        !apply_status.contains("Claude Opus 4.8"),
        "after-apply statusline must not keep stock empty Opus: {apply_status}"
    );
    let applied = apply
        .audit
        .model_choice
        .as_ref()
        .map(|choice| choice.applied.as_str())
        .expect("empty-model-apply after audit.model_choice.applied");
    assert_eq!(
        applied, "Claude Sonnet 4.5",
        "audit.model_choice.applied must agree with apply attachment/side_effect"
    );
    assert!(
        apply.side_effects.iter().any(|effect| {
            effect.status == "model-applied"
                && (effect.effect_id.contains("model-apply")
                    || effect.effect_id.contains("model.apply"))
        }),
        "side_effect must remain model-applied for empty-model-apply"
    );
    assert!(
        apply.audit.fixture_id.contains("after-apply")
            && apply.audit.fixture_id.to_ascii_lowercase().contains("sonnet"),
        "empty-model-apply after fixture must be the Sonnet after-apply bind, got {}",
        apply.audit.fixture_id
    );

    let open_session = replay_case("empty-open-session", 1).expect("open session");
    open_session.validate().expect("open session valid");
    assert_eq!(open_session.after.screen_id, "session-picker");
    assert!(open_session
        .after
        .origin_stack
        .contains(&"empty".to_owned()));

    let session_cancel = replay_case("empty-session-cancel", 1).expect("session cancel");
    session_cancel.validate().expect("session cancel valid");
    assert_eq!(session_cancel.after.screen_id, "empty");
    assert!(session_cancel.after.transcript.is_empty());

    let resume = replay_case("empty-session-resume", 1).expect("session resume");
    resume.validate().expect("resume valid");
    assert_eq!(resume.after.screen_id, "chat-turn");
    assert_ne!(
        resume.after.conversation_id, resume.before.conversation_id,
        "resume intentionally opens the chosen session"
    );
}

#[test]
fn status_return_preserves_conversation_draft() {
    let open = replay_case("status-open", 1).expect("status open");
    open.validate().expect("open valid");
    assert_eq!(open.after.screen_id, "status");
    let back = replay_case("status-return", 1).expect("status return");
    back.validate().expect("return valid");
    assert_eq!(back.after.screen_id, "chat-turn");
    assert_eq!(
        back.before.draft, back.after.draft,
        "draft must survive status"
    );
}

#[test]
fn published_screens_render_through_public_cli() {
    for screen in [
        "theme-gallery-dark",
        "theme-gallery-light",
        "status",
        "empty",
    ] {
        let output = harness().args([screen]).output().expect("render");
        assert!(
            output.status.success(),
            "{screen}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/{}/{}.txt",
                if screen == "empty" { "states" } else { "views" },
                screen
            ))
            .is_file(),
            "{screen} TXT must publish"
        );
    }
}
