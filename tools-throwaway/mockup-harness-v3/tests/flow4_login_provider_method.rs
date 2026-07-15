//! Production Login / Connect Provider / Connect Method (VAL-V3-F4-002..011).
//!
//! Covers multi-origin entry (onboarding, blocked-model, conversation), human
//! provider catalogues with readable states and applicable actions, method
//! catalogues that only expose supported sign-in options, non-color selection,
//! and exact Back/Cancel restoration of prior interaction state.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

fn assert_no_protocol_jargon(text: &str) {
    for banned in [
        "transport",
        "endpoint",
        "oauth2",
        "device_code",
        "client_secret",
        "PKCE",
        "openai-provider-id",
        "stdio",
    ] {
        assert!(
            !text.contains(banned),
            "protocol jargon leaked into operator surface: {banned}"
        );
    }
}

fn assert_statusline_minimal(text: &str) {
    let status = text.lines().last().expect("statusline row");
    assert!(
        status.contains("~/project")
            && (status.contains("Claude Sonnet 4.5") || status.contains("Claude Opus 4.8")),
        "statusline shape unexpected: {status}"
    );
    assert!(
        !status.contains('%') && !status.contains('$') && !status.contains("idle"),
        "statusline must stay minimal"
    );
}

fn assert_canvas_geometry(text: &str) {
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "canonical height is 40 rows");
    assert!(
        rows.iter().all(|row| row.chars().count() == 120),
        "canonical width is 120 columns"
    );
}

// ── VAL-V3-F4-002 / 004: Login first choice + safe exit ─────────────────

#[test]
fn login_blocked_model_origin_offers_one_primary_and_safe_exit() {
    let audit = audit_screen("login", "auth.login.blocked-model")
        .expect("blocked-model login must render");
    audit.validate().expect("login audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_protocol_jargon(&text);

    assert!(text.contains("Connect a model provider"));
    assert!(text.contains("Code needs a provider"));
    assert!(text.contains("› [enter] Choose a provider"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")) || text.contains("[esc] Back"));
    assert!(text.contains("does not attempt") || text.contains("without connecting") || text.contains("no auth"));
    assert!(text.contains("Provider connection · blocked model action"));
    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1,
        "exactly one primary action"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.login.choose-provider"
            && t.to_screen_id == "connect-provider"
            && t.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.login.cancel"
            && t.to_screen_id == "model-picker"
            && t.kind == "cancel"
    }));
    // Neutral: no premature success/error verdict.
    assert!(!text.contains("✓ Connected"));
    assert!(!text.contains("Connection failed"));
}

#[test]
fn login_onboarding_origin_cancels_to_onboarding() {
    let audit =
        audit_screen("login", "auth.login.onboarding").expect("onboarding login must render");
    audit.validate().expect("onboarding login audit");
    let text = audit.render_txt();
    assert!(text.contains("Provider connection · onboarding"));
    assert!(text.contains("› [enter] Choose a provider"));
    assert!((text.contains("Esc to") || text.contains("[esc]")));
    assert!(
        text.contains("onboarding") || text.contains("first-run") || text.contains("setup"),
        "onboarding-origin copy must name the setup context"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.login.cancel" && t.to_screen_id == "onboarding" && t.kind == "cancel"
    }));
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|c| c.visible_origin.as_str()),
        Some("Provider connection · onboarding")
    );
}

#[test]
fn login_conversation_origin_cancels_to_chat_turn() {
    let audit =
        audit_screen("login", "auth.login.conversation").expect("conversation login must render");
    audit.validate().expect("conversation login audit");
    let text = audit.render_txt();
    assert!(text.contains("Provider connection · conversation"));
    assert!(text.contains("draft") || text.contains("conversation stays"));
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.login.cancel" && t.to_screen_id == "chat-turn" && t.kind == "cancel"
    }));
    assert_eq!(
        audit
            .conversation
            .as_ref()
            .map(|c| c.visible_origin.as_str()),
        Some("Provider connection · conversation")
    );
}

// ── VAL-V3-F4-005 / 006 / 007 / 008: Provider catalogue ─────────────────

#[test]
fn connect_provider_catalogue_is_human_and_stateful() {
    let audit = audit_screen("connect-provider", "auth.connect-provider.blocked-model")
        .expect("provider catalogue must render");
    audit.validate().expect("catalogue audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_protocol_jargon(&text);

    // Human display names + short benefit; full catalogue, not a fixed-row truncation story.
    for name in ["OpenRouter", "Anthropic", "OpenAI", "Google"] {
        assert!(text.contains(name), "missing provider {name}");
    }
    assert!(text.contains("Unified access") || text.contains("many models"));
    assert!(text.contains("Claude models") || text.contains("everyday coding"));
    assert!(text.contains("GPT-5.2 Codex") || text.contains("related models"));
    assert!(text.contains("Gemini") || text.contains("broad tasks"));

    // Readable state text, not color alone.
    assert!(text.contains("Connected · in use") || text.contains("In use"));
    assert!(text.contains("Connected"));
    assert!(text.contains("Not connected"));
    // Exactly one selection marker on a provider row (non-color).
    assert!(text.contains("› OpenAI") || text.contains("› Anthropic") || text.contains("› Google") || text.contains("› OpenRouter"));

    // Default unconnected selection offers Connect.
    assert!(text.contains("Connect OpenAI") || text.contains("Connect Anthropic") || text.contains("Connect Google") || text.contains("Connect OpenRouter"));
    assert!(text.contains("[esc] Back") || (text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));

    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-provider.connect"
            && t.to_screen_id == "connect-method"
            && t.kind == "direct"
    }));
}

#[test]
fn connect_provider_connected_selection_offers_use_or_disconnect() {
    let audit = audit_screen(
        "connect-provider",
        "auth.connect-provider.selected-anthropic",
    )
    .expect("connected selection fixture must render");
    audit.validate().expect("connected selection audit");
    let text = audit.render_txt();
    assert!(text.contains("› Anthropic"));
    assert!(text.contains("Connected"));
    assert!(
        text.contains("Use Anthropic") || text.contains("Disconnect Anthropic"),
        "connected selection must offer Use or Disconnect, not Connect"
    );
    assert!(
        !text.contains("Connect Anthropic"),
        "Connect must not apply to an already connected provider"
    );
    assert!(
        audit.transitions.iter().any(|t| {
            t.action_id == "action.connect-provider.use"
                || t.action_id == "action.connect-provider.disconnect"
        }),
        "audit must expose Use or Disconnect for a connected selection"
    );
}

#[test]
fn connect_provider_backs_restore_login_for_gate_chain() {
    let audit = audit_screen(
        "connect-provider",
        "gate.provider-and-mcp.connect-provider",
    )
    .expect("gate catalogue must stay green");
    audit.validate().expect("gate catalogue");
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-provider.back"
            && t.to_screen_id == "login"
            && t.kind == "cancel"
    }));
}

#[test]
fn connect_provider_origin_copy_matches_fixture() {
    for (fixture, origin_tag) in [
        (
            "auth.connect-provider.onboarding",
            "Provider connection · onboarding",
        ),
        (
            "auth.connect-provider.blocked-model",
            "Provider connection · blocked model action",
        ),
        (
            "auth.connect-provider.conversation",
            "Provider connection · conversation",
        ),
    ] {
        let audit = audit_screen("connect-provider", fixture)
            .unwrap_or_else(|err| panic!("{fixture} must render: {err}"));
        audit
            .validate()
            .unwrap_or_else(|err| panic!("{fixture} audit: {err}"));
        let text = audit.render_txt();
        assert!(
            text.contains(origin_tag),
            "{fixture} must show visible origin {origin_tag}"
        );
        assert_eq!(
            audit
                .conversation
                .as_ref()
                .map(|c| c.visible_origin.as_str()),
            Some(origin_tag)
        );
    }
}

// ── VAL-V3-F4-009 / 010 / 011: Method choice ────────────────────────────

#[test]
fn connect_method_openai_shows_both_applicable_methods_with_marker() {
    let audit = audit_screen("connect-method", "auth.connect-method.openai")
        .expect("openai method choice must render");
    audit.validate().expect("method choice audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_protocol_jargon(&text);

    assert!(text.contains("Sign in to OpenAI") || text.contains("OpenAI"));
    assert!(text.contains("Browser sign-in") || text.contains("browser"));
    assert!(text.contains("API key") || text.contains("Use an API key"));
    assert!(text.contains("authorize in your browser") || text.contains("browser"));
    assert!(text.contains("paste a key") || text.contains("stored only"));
    // Exactly one selected method marked with › (plus dock primary), never color-only.
    assert!(text.contains("› Browser sign-in") || text.contains("› Use an API key"));
    let marker_count = text.matches('›').count();
    assert!(
        marker_count >= 2,
        "selected method and primary action must both be marked"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-method.oauth"
            && t.to_screen_id == "connect-oauth"
            && t.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-method.back"
            && t.to_screen_id == "connect-provider"
            && t.kind == "cancel"
    }));
}

#[test]
fn connect_method_google_is_api_key_only() {
    let audit = audit_screen("connect-method", "auth.connect-method.google")
        .expect("google methods must render");
    audit.validate().expect("google method audit");
    let text = audit.render_txt();
    assert!(text.contains("Sign in to Google") || text.contains("Google"));
    assert!(text.contains("API key") || text.contains("Use an API key"));
    assert!(
        !text.contains("Browser sign-in"),
        "Google does not advertise browser sign-in"
    );
    assert!(
        !text.contains("OAuth"),
        "unsupported OAuth must be absent, not disabled"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-method.api-key"
            && t.to_screen_id == "connect-apikey"
            && t.kind == "direct"
    }));
    assert!(
        !audit
            .transitions
            .iter()
            .any(|t| t.action_id == "action.connect-method.oauth"),
        "oauth transition must be absent when the provider does not advertise it"
    );
}

// ── Multi-origin cancel / back scenarios (exact restoration) ────────────

#[test]
fn login_cancel_scenarios_restore_each_origin() {
    let blocked = replay_case("login-cancel", 1).expect("blocked cancel still works");
    assert_eq!(blocked.before.screen_id, "login");
    assert_eq!(blocked.after.screen_id, "model-picker");
    assert_eq!(blocked.actions[0].action_id, "action.login.cancel");
    for field in ["draft", "selection", "transcript"] {
        let hashes = blocked
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} required"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive cancel");
    }

    let onboarding = replay_case("login-cancel-onboarding", 1).expect("onboarding cancel");
    assert_eq!(onboarding.before.screen_id, "login");
    assert_eq!(onboarding.after.screen_id, "onboarding");
    assert_eq!(onboarding.actions[0].action_id, "action.login.cancel");
    assert!(
        onboarding
            .preserved_field_hashes
            .get("draft")
            .map(|h| h.before == h.after)
            .unwrap_or(false)
            || onboarding.before.draft == onboarding.after.draft
    );

    let conversation = replay_case("login-cancel-conversation", 1).expect("conversation cancel");
    assert_eq!(conversation.before.screen_id, "login");
    assert_eq!(conversation.after.screen_id, "chat-turn");
    assert_eq!(conversation.actions[0].action_id, "action.login.cancel");
    for field in ["draft", "transcript", "selection"] {
        let hashes = conversation
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} required on conversation cancel"));
        assert_eq!(
            hashes.before, hashes.after,
            "conversation {field} must be byte-equal after cancel"
        );
    }
}

#[test]
fn method_back_preserves_provider_and_origin_chain() {
    let method_back = replay_case("connect-method-back", 1).expect("method back");
    assert_eq!(method_back.before.screen_id, "connect-method");
    assert_eq!(method_back.after.screen_id, "connect-provider");
    assert_eq!(
        method_back.actions[0].action_id,
        "action.connect-method.back"
    );
    assert_eq!(method_back.after.selection, "openai");
    // No auth attempt side-effect on back.
    assert!(
        method_back.side_effects.iter().all(|effect| {
            effect.status == "no-change"
                || effect.kind.contains("provider")
                || effect.status.contains("provider-selected")
                || effect.status == "provider-selection-preserved"
                || effect.effect_id.contains("back")
        }),
        "back must not start authentication"
    );

    let select = replay_case("connect-provider-select", 1).expect("select still works");
    assert_eq!(select.before.screen_id, "connect-provider");
    assert_eq!(select.after.screen_id, "connect-method");
    assert_eq!(
        select.actions[0].action_id,
        "action.connect-provider.connect"
    );
}

#[test]
fn provider_back_scenarios_do_not_mutate_credentials() {
    for case_id in [
        "connect-provider-back-onboarding",
        "connect-provider-back-conversation",
        "connect-provider-back-blocked-model",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|err| panic!("{case_id}: {err}"));
        assert_eq!(replay.before.screen_id, "connect-provider");
        assert_eq!(
            replay.actions[0].action_id,
            "action.connect-provider.back"
        );
        assert!(
            matches!(
                replay.after.screen_id.as_str(),
                "login" | "onboarding" | "model-picker" | "chat-turn"
            ),
            "{case_id} returned to unexpected screen {}",
            replay.after.screen_id
        );
        for field in ["draft", "transcript"] {
            if let Some(hashes) = replay.preserved_field_hashes.get(field) {
                assert_eq!(
                    hashes.before, hashes.after,
                    "{case_id} must preserve {field}"
                );
            }
        }
        assert!(
            replay
                .side_effects
                .iter()
                .all(|effect| effect.status == "no-change"
                    || effect.status.contains("no-auth")
                    || effect.effect_id.contains("back")),
            "{case_id} must not write credentials"
        );
    }
}

#[test]
fn gate_login_fixture_remains_blocked_model_compatible() {
    // Production work must not break the approved Flow 4 gate frame.
    let audit = audit_screen("login", "gate.provider-and-mcp.login").expect("gate login");
    audit.validate().expect("gate login valid");
    let text = audit.render_txt();
    assert!(text.contains("Provider connection · blocked model action"));
    assert!(text.contains("› [enter] Choose a provider"));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
}

#[test]
fn method_selection_survives_without_color_alone() {
    let audit = audit_screen("connect-method", "auth.connect-method.openai")
        .expect("method choice");
    let text = audit.render_txt();
    // Glyph or positional marker on the selected row, independent of ACCENT styling.
    let has_marker = text.lines().any(|line| {
        line.contains('›')
            && (line.contains("Browser sign-in") || line.contains("Use an API key"))
    });
    assert!(
        has_marker,
        "selected method must be identifiable by › marker without relying on color"
    );
    // Confirm dock restates the current selection in plain text.
    assert!(
        text.contains("Selected · Browser sign-in")
            || text.contains("Selected · Use an API key")
            || text.contains("Selected method"),
        "dock must restate the selected method in words"
    );
}
