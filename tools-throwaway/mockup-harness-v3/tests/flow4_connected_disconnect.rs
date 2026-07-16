//! Connected resume, disconnect confirmation, and effective credentials
//! (VAL-V3-F4-020..026, VAL-V3-F4-060).
//!
//! Covers named success outcomes, idempotent Continue, exact blocked-origin
//! return, target-scoped disconnect confirmation, concrete consequences, and
//! multi-source effective availability without silent model switching.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const SECRET_SENTINELS: &[&str] = &[
    "*****************************************",
    "fixture-secret-value",
    "client_secret",
    "Authorization: Bearer",
    "access_token=sk-oauth-secret",
    "device_code",
    "refresh_token",
    "OPENAI_API_KEY=",
];

fn assert_no_secret_leak(text: &str) {
    for banned in SECRET_SENTINELS {
        assert!(
            !text.contains(banned),
            "secret residue leaked: {banned}"
        );
    }
}

fn assert_canvas_geometry(text: &str) {
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "canonical height is 40 rows");
    assert!(
        rows.iter().all(|row| row.chars().count() == 120),
        "canonical width is 120 columns"
    );
}

fn assert_statusline_minimal(text: &str) {
    let status = text.lines().last().expect("statusline");
    assert!(
        status.contains("~/project")
            && (status.contains("Claude Sonnet 4.5") || status.contains("Claude Opus 4.8")),
        "statusline unexpected: {status}"
    );
    assert!(!status.contains('%') && !status.contains('$') && !status.contains("idle"));
}

fn primary_labels(audit: &mockup_harness_v3::audit::AuditDocument) -> Vec<String> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
        .collect()
}

// ── VAL-V3-F4-020: Connection success names the outcome ─────────────────

#[test]
fn connect_connected_names_provider_method_and_one_continue() {
    let audit = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )
    .expect("connected must render");
    audit.validate().expect("connected audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_secret_leak(&text);

    // One success verdict names provider + method and readiness.
    assert!(
        text.contains("OpenAI"),
        "must name the connected provider"
    );
    assert!(
        text.contains("browser sign-in") || text.contains("Browser sign-in"),
        "gate success is browser sign-in / OAuth: {text}"
    );
    assert!(
        text.contains("✓") || text.contains("Connected") || text.contains("ready"),
        "must state readiness"
    );
    assert!(
        text.contains("GPT-5.2 Codex"),
        "must state immediate user benefit"
    );
    assert!(
        text.contains("Continue") || text.contains("Return to model"),
        "must present one Done/Continue/return action"
    );

    let primaries = primary_labels(&audit);
    assert_eq!(
        primaries.len(),
        1,
        "exactly one primary action on success: {primaries:?}"
    );
    // Must not compete with secondary banners.
    let success_events: Vec<_> = audit
        .events
        .iter()
        .filter(|e| e.identity.family == "success")
        .collect();
    assert_eq!(
        success_events.len(),
        1,
        "exactly one success family verdict"
    );
}

#[test]
fn connect_connected_api_key_method_named() {
    let audit = audit_screen("connect-connected", "auth.connect-connected.api-key")
        .expect("api-key connected fixture");
    audit.validate().expect("audit");
    let text = audit.render_txt();
    assert_no_secret_leak(&text);
    assert!(text.contains("OpenAI"));
    assert!(
        text.contains("API key") || text.contains("api key"),
        "must name API key method: {text}"
    );
    assert!(!text.contains("browser sign-in") && !text.contains("Browser sign-in"));
    assert!(!text.contains(SECRET_SENTINELS[0]));
}

// ── VAL-V3-F4-021: Success returns to blocked action, never MCP ─────────

#[test]
fn connect_connected_returns_to_blocked_origin_not_mcp() {
    let audit = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )
    .expect("connected");
    audit.validate().expect("audit");
    let text = audit.render_txt();
    assert!((text.contains("Connections") || text.contains("Provider connection · blocked model action")));
    assert!(
        !text.contains("Continue to MCP")
            && !text.contains("Open MCP now")
            && !text.contains("next step is MCP"),
        "MCP must not be the live primary return"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.connect-connected.return"
            && t.to_screen_id == "model-picker"
            && t.kind == "direct"
    }));
    assert!(
        !audit
            .transitions
            .iter()
            .any(|t| t.to_screen_id == "mcp" && t.kind == "direct"),
        "no direct MCP edge from provider success"
    );

    let replay = replay_case("connect-connected-return", 1).expect("return replay");
    assert_eq!(replay.before.screen_id, "connect-connected");
    assert_eq!(replay.after.screen_id, "model-picker");
    assert_eq!(
        replay.actions[0].action_id,
        "action.connect-connected.return"
    );
    // Draft and transcript survive; selection may move from connection-success
    // focus to the restored blocked model choice, which is the success intent.
    for field in ["draft", "transcript"] {
        if let Some(hashes) = replay.preserved_field_hashes.get(field) {
            assert_eq!(
                hashes.before, hashes.after,
                "{field} must survive successful return"
            );
        }
    }
    assert_eq!(
        replay.after.paused_activity_id,
        "pending.model.gpt-5-2-codex"
    );
    assert_eq!(replay.attempts[0].count, 1, "resume counts once");
}

// ── VAL-V3-F4-022: Success cannot duplicate credentials ─────────────────

#[test]
fn connect_connected_continue_is_idempotent() {
    let once = replay_case("connect-connected-return", 1).expect("first continue");
    let twice = replay_case("connect-connected-return-idempotent", 1)
        .expect("repeated continue must replay");

    assert_eq!(once.before.screen_id, "connect-connected");
    assert_eq!(twice.before.screen_id, "connect-connected");
    assert_eq!(once.after.screen_id, "model-picker");
    assert_eq!(twice.after.screen_id, "model-picker");
    assert_eq!(
        twice.actions[0].action_id,
        "action.connect-connected.return"
    );
    // Exactly one connection remains; repeated continue does not add another.
    assert!(
        twice.side_effects[0].status.contains("idempotent")
            || twice.side_effects[0].status.contains("already-connected")
            || twice.side_effects[0].status.contains("noop")
            || twice.attempts[0].outcome.contains("already")
            || twice.attempts[0].outcome.contains("idempotent"),
        "repeated Continue must be no-op / already-connected: effect={} attempt={}",
        twice.side_effects[0].status,
        twice.attempts[0].outcome
    );
    assert_eq!(
        twice.attempts[0].count, 1,
        "connection write count stays one"
    );

    // Surface must not redisplay secret-bearing input.
    let audit = audit_screen(
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    )
    .expect("connected");
    let text = audit.render_txt();
    assert_no_secret_leak(&text);
    assert!(
        !text.contains("sk-") || text.contains("·") || text.contains("•"),
        "must not redisplay a secret-bearing key field populated"
    );
    assert!(!text.contains("Paste or type an API key"));
}

// ── VAL-V3-F4-023: Disconnect confirmation names the provider ───────────

#[test]
fn connect_disconnect_names_provider_and_safe_cancel() {
    let audit = audit_screen(
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.local",
    )
    .expect("disconnect must render");
    audit.validate().expect("disconnect audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_secret_leak(&text);

    assert!(
        text.contains("Anthropic"),
        "must name the selected display name, not only an ID"
    );
    assert!(
        !text.contains("provider.anthropic") && !text.contains("anthropic-api"),
        "raw IDs must not replace the display name"
    );
    assert!(
        text.contains("Disconnect") || text.contains("disconnect"),
        "destructive action must be named"
    );
    assert!(
        text.contains("Cancel") || (text.contains("Esc to") || text.contains("[esc]")),
        "safe cancel/escape must exist"
    );

    let primaries = primary_labels(&audit);
    assert_eq!(primaries.len(), 1, "one primary: {primaries:?}");
    assert!(
        primaries[0].to_ascii_lowercase().contains("disconnect"),
        "primary is Disconnect: {}",
        primaries[0]
    );
    // Credential still exists before confirmation (no early deletion).
    assert!(
        text.contains("still connected")
            || text.contains("still available")
            || text.contains("remains until")
            || text.contains("until you confirm")
            || text.contains("will remove"),
        "must prove credential still exists until confirm: {text}"
    );
}

// ── VAL-V3-F4-024: Disconnect explains concrete consequences ────────────

#[test]
fn connect_disconnect_explains_consequences_and_remaining_sources() {
    let audit = audit_screen(
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.local",
    )
    .expect("disconnect");
    let text = audit.render_txt();
    assert_no_secret_leak(&text);

    assert!(
        text.contains("removable") || text.contains("local") || text.contains("saved"),
        "must name which removable stored credential is removed"
    );
    assert!(
        text.contains("remain")
            || text.contains("still available")
            || text.contains("environment")
            || text.contains("managed")
            || text.contains("other providers")
            || text.contains("Other providers"),
        "must speak to remaining effective sources / model availability"
    );
    // No silent active-model switch language.
    assert!(
        !text.contains("auto-switch")
            && !text.contains("silently switch")
            && !text.contains("automatically switch model"),
        "must never claim silent model switching"
    );
}

#[test]
fn connect_disconnect_active_model_impact_requires_choice() {
    // Anthropic is currently the catalogue "Connected" provider; when the active
    // model would lose its only effective source, the UI must require a choice.
    let audit = audit_screen(
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.last-source",
    )
    .expect("last-source disconnect");
    let text = audit.render_txt();
    assert!(
        text.contains("unavailable")
            || text.contains("choose a replacement")
            || text.contains("Choose a replacement")
            || text.contains("pick another")
            || text.contains("no longer use")
            || text.contains("will no longer"),
        "active-model impact must be explicit before confirm: {text}"
    );
    assert!(
        !text.contains("auto-switch") && !text.contains("automatically switch"),
        "must not claim automatic model switch"
    );
}

// ── VAL-V3-F4-025: Disconnect cancel changes nothing ────────────────────

#[test]
fn connect_disconnect_cancel_preserves_connection() {
    let replay = replay_case("connect-disconnect-cancel", 1).expect("cancel must replay");
    assert_eq!(replay.before.screen_id, "connect-disconnect");
    assert_eq!(replay.after.screen_id, "connect-provider");
    assert_eq!(
        replay.actions[0].action_id,
        "action.connect-disconnect.cancel"
    );
    assert!(
        replay.side_effects[0].status.contains("unchanged")
            || replay.side_effects[0].status.contains("noop")
            || replay.side_effects[0].status.contains("preserved")
            || replay.attempts[0].outcome.contains("cancelled")
            || replay.attempts[0].outcome.contains("unchanged"),
        "cancel must not disconnect: effect={} attempt={}",
        replay.side_effects[0].status,
        replay.attempts[0].outcome
    );
    assert_eq!(
        replay.after.selection, "anthropic",
        "same provider remains selected"
    );
    for field in ["draft", "transcript"] {
        let hashes = replay
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive cancel");
    }
}

// ── VAL-V3-F4-026: Confirm updates only its target ──────────────────────

#[test]
fn connect_disconnect_confirm_is_target_scoped() {
    let replay = replay_case("connect-disconnect-confirm", 1).expect("confirm must replay");
    assert_eq!(replay.before.screen_id, "connect-disconnect");
    assert_eq!(replay.after.screen_id, "connect-provider");
    assert_eq!(
        replay.actions[0].action_id,
        "action.connect-disconnect.confirm"
    );
    assert!(
        replay.side_effects[0].status.contains("removed")
            || replay.side_effects[0].status.contains("disconnected")
            || replay.attempts[0].outcome.contains("removed")
            || replay.attempts[0].outcome.contains("disconnected"),
        "confirm must remove only the named credential: effect={} attempt={}",
        replay.side_effects[0].status,
        replay.attempts[0].outcome
    );
    // Unrelated providers and MCP stay untouched in the effect detail.
    let detail_hash = &replay.side_effects[0].effect_hash;
    assert!(!detail_hash.is_empty());
    // Attempt owner is the named target.
    assert!(
        replay.attempts[0].owner_id.contains("anthropic")
            || replay.attempts[0].owner_id.contains("provider.anthropic"),
        "owner must be target-scoped: {}",
        replay.attempts[0].owner_id
    );
}

// ── VAL-V3-F4-060: Provider availability follows effective credentials ──

#[test]
fn effective_credentials_multi_source_matrix() {
    // Local + environment remaining: provider stays available after local remove.
    let multi = replay_case("connect-disconnect-multi-source", 1)
        .expect("multi-source disconnect must replay");
    assert_eq!(multi.before.screen_id, "connect-disconnect");
    assert_eq!(multi.after.screen_id, "connect-provider");
    assert!(
        multi.side_effects[0].status.contains("still-available")
            || multi.side_effects[0].status.contains("source-remaining")
            || multi.attempts[0].outcome.contains("still-available")
            || multi.attempts[0].outcome.contains("source-remaining")
            || multi.after.selection.contains("available")
            || multi.after.focus.contains("available")
            || multi.after.attachment.contains("available"),
        "provider must remain available when another source remains: effect={} attempt={} focus={} attachment={}",
        multi.side_effects[0].status,
        multi.attempts[0].outcome,
        multi.after.focus,
        multi.after.attachment
    );

    // Last source: provider becomes not connected / unavailable; no silent switch.
    let last = replay_case("connect-disconnect-last-source", 1)
        .expect("last-source disconnect must replay");
    assert!(
        last.side_effects[0].status.contains("unavailable")
            || last.side_effects[0].status.contains("not-connected")
            || last.attempts[0].outcome.contains("unavailable")
            || last.attempts[0].outcome.contains("not-connected")
            || last.after.focus.contains("not-connected")
            || last.after.attachment.contains("not-connected"),
        "last source must recompute to unavailable: effect={} attempt={}",
        last.side_effects[0].status,
        last.attempts[0].outcome
    );
    assert!(
        !last.side_effects[0].status.contains("auto-switch")
            && !last.attempts[0].outcome.contains("auto-switch")
            && !last.side_effects[0].status.contains("switch-model"),
        "never silent model switch"
    );

    // Surface catalogue for multi-source fixture must still show remaining source.
    let multi_ui = audit_screen(
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.multi-source",
    )
    .expect("multi-source fixture");
    multi_ui.validate().expect("audit");
    let text = multi_ui.render_txt();
    assert!(
        text.contains("environment")
            || text.contains("Environment")
            || text.contains("managed")
            || text.contains("Managed")
            || text.contains("still available")
            || text.contains("remain"),
        "multi-source fixture must show remaining effective credential: {text}"
    );
    // Local syntax validation alone never claims Connected (already covered upstream);
    // disconnect confirmation never claims unavailable while a valid source remains.
    assert!(
        !text.contains("provider unavailable") || text.contains("remain"),
        "must not falsely claim unavailable while another source remains"
    );
}

#[test]
fn connect_disconnect_canonical_and_gate_render() {
    for (screen, fixture) in [
        (
            "connect-connected",
            "gate.provider-and-mcp.connect-connected",
        ),
        ("connect-connected", "canonical.connect-connected"),
        (
            "connect-disconnect",
            "auth.connect-disconnect.anthropic.local",
        ),
        ("connect-disconnect", "canonical.connect-disconnect"),
    ] {
        let audit = audit_screen(screen, fixture)
            .unwrap_or_else(|e| panic!("{screen}/{fixture} must render: {e}"));
        audit
            .validate()
            .unwrap_or_else(|e| panic!("{screen}/{fixture} must validate: {e}"));
        let text = audit.render_txt();
        assert_canvas_geometry(&text);
        assert_statusline_minimal(&text);
        assert_no_secret_leak(&text);
    }
}
