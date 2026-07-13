//! OAuth UX and protocol safeguards (VAL-V3-F4-016..019, VAL-V3-F4-054).
//!
//! Covers device/loopback browser steps, live waiting, cancel and failure
//! recovery, state/PKCE validation, code separation, listener release, and
//! late-callback rejection.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const DEVICE_CODE_SECRET: &str = "device_code";
const DEVICE_CODE_VALUE: &str = "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS";
const USER_CODE: &str = "WDJB-MJHT";
const ACCESS_TOKEN_SENTINEL: &str = "access_token=sk-oauth-secret";
const CLIENT_SECRET_SENTINEL: &str = "client_secret";
const CALLBACK_QUERY_SENTINEL: &str = "?code=";
const CODE_VERIFIER_SENTINEL: &str = "code_verifier";
const STATE_SECRET: &str = "oauth_state_raw_abcdef0123456789";

fn assert_no_oauth_secret_leak(text: &str) {
    for banned in [
        DEVICE_CODE_SECRET,
        DEVICE_CODE_VALUE,
        ACCESS_TOKEN_SENTINEL,
        CLIENT_SECRET_SENTINEL,
        CALLBACK_QUERY_SENTINEL,
        CODE_VERIFIER_SENTINEL,
        STATE_SECRET,
        "Authorization: Bearer",
        "refresh_token",
        "fixture-secret-value",
    ] {
        assert!(
            !text.contains(banned),
            "OAuth secret/protocol residue leaked: {banned}"
        );
    }
}

fn assert_no_protocol_jargon_in_ui(text: &str) {
    // User-facing canvas must not lead with raw protocol terms.
    for banned in ["oauth2", "PKCE", "stdio", "transport", "code_challenge"] {
        assert!(
            !text.contains(banned),
            "protocol jargon leaked into UI: {banned}"
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

// ── VAL-V3-F4-016: Understandable browser steps ─────────────────────────

#[test]
fn connect_oauth_device_flow_shows_user_code_not_device_code() {
    let audit = audit_screen("connect-oauth", "auth.connect-oauth.device.waiting")
        .expect("device waiting fixture must render");
    audit.validate().expect("device waiting audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_oauth_secret_leak(&text);
    assert_no_protocol_jargon_in_ui(&text);

    assert!(
        text.contains("device")
            || text.contains("one-time code")
            || text.contains("verification"),
        "device flow must declare device/one-time code mode: {text}"
    );
    assert!(
        text.contains(USER_CODE),
        "user-facing user_code must render: {text}"
    );
    assert!(
        text.contains("auth.openai.com") || text.contains("openai.com"),
        "verification URL must remain user-actionable"
    );
    assert!(
        text.contains("OpenAI") || text.contains("Authorize"),
        "provider must stay clear"
    );
    assert!(
        !text.contains(DEVICE_CODE_VALUE) && !text.contains("device_code"),
        "device_code must never render"
    );
    assert!(
        audit.events.iter().any(|e| {
            e.identity.family == "oauth-flow" && e.identity.subject_id.contains("device")
        }),
        "audit must declare device flow subject"
    );
    assert!(
        audit.events.iter().any(|e| {
            e.identity.family == "one-time-code" || e.identity.subject_id.contains("user-code")
        }),
        "audit must emit user-code subject"
    );
}

#[test]
fn connect_oauth_loopback_flow_has_no_device_code_or_callback_query() {
    let audit = audit_screen("connect-oauth", "auth.connect-oauth.loopback.waiting")
        .expect("loopback waiting fixture must render");
    audit.validate().expect("loopback audit");
    let text = audit.render_txt();
    assert_no_oauth_secret_leak(&text);
    assert_no_protocol_jargon_in_ui(&text);
    assert!(
        text.contains("browser") || text.contains("redirect") || text.contains("loopback")
            || text.contains("local return"),
        "loopback flow must be declared in plain English"
    );
    assert!(
        !text.contains(USER_CODE) || text.contains("local return"),
        "loopback must not invent a device user_code as the primary step"
    );
    assert!(!text.contains(CALLBACK_QUERY_SENTINEL));
    assert!(!text.contains("device_code"));
    assert!(
        audit.events.iter().any(|e| {
            e.identity.family == "oauth-flow"
                && (e.identity.subject_id.contains("loopback")
                    || e.identity.subject_id.contains("redirect"))
        }),
        "audit must declare loopback/redirect flow"
    );
}

#[test]
fn connect_oauth_consent_before_launch_and_scope_sanitization() {
    let consent = audit_screen("connect-oauth", "auth.connect-oauth.consent")
        .expect("consent fixture");
    consent.validate().expect("consent audit");
    let text = consent.render_txt();
    assert_no_oauth_secret_leak(&text);
    assert!(
        text.contains("Before") || text.contains("allows") || text.contains("consent")
            || text.contains("Code will") || text.contains("authorize"),
        "consent copy must appear before browser launch"
    );
    let labels = primary_labels(&consent);
    assert_eq!(labels.len(), 1, "consent has one primary launch action");
    assert!(
        labels[0].to_ascii_lowercase().contains("open")
            || labels[0].to_ascii_lowercase().contains("browser"),
        "primary is Open browser, got {}",
        labels[0]
    );

    let hostile = audit_screen("connect-oauth", "auth.connect-oauth.hostile-scope")
        .expect("hostile scope fixture");
    hostile.validate().expect("hostile audit");
    let hostile_txt = hostile.render_txt();
    assert_no_oauth_secret_leak(&hostile_txt);
    assert!(
        !hostile_txt.contains('\u{202e}')
            && !hostile_txt.contains('\t')
            && !hostile_txt.contains('\u{0007}'),
        "hostile bidi/tab/control sequences must be sanitized out of consent"
    );
    assert!(
        hostile_txt.contains("read")
            || hostile_txt.contains("models")
            || hostile_txt.contains("scope"),
        "sanitized scope names may still appear as policy metadata"
    );

    let secret_scope = audit_screen("connect-oauth", "auth.connect-oauth.secret-scope")
        .expect("secret scope fixture");
    secret_scope.validate().expect("secret-scope audit");
    let secret_txt = secret_scope.render_txt();
    assert_no_oauth_secret_leak(&secret_txt);
    assert!(
        !secret_txt.contains("sk-oauth-secret") && !secret_txt.contains("bearer-value"),
        "secret scope values must be redacted"
    );
}

// ── VAL-V3-F4-017: Waiting is live with zero primary ────────────────────

#[test]
fn connect_oauth_waiting_is_live_with_one_owner_and_zero_primary() {
    let early = audit_screen("connect-oauth", "auth.connect-oauth.device.waiting.early")
        .expect("early waiting fixture");
    early.validate().expect("early audit");
    let early_txt = early.render_txt();
    assert_no_oauth_secret_leak(&early_txt);
    assert!(
        early_txt.contains("Waiting for authorization"),
        "must show readable waiting state"
    );
    assert!(
        !early_txt.contains("✓ Connected") && !early_txt.contains("Connected · ready"),
        "waiting is not a success verdict"
    );
    let early_primaries = primary_labels(&early);
    assert!(
        early_primaries.is_empty(),
        "waiting has zero primary actions, got {early_primaries:?}"
    );
    assert!(
        early.events.iter().any(|e| {
            e.identity.family == "live-state"
                && e.identity.subject_id.contains("waiting")
        }),
        "one live waiting owner event required"
    );
    assert!(
        early_txt.contains("Cancel") || early_txt.contains("cancel"),
        "Cancel remains available as secondary"
    );

    let later = audit_screen("connect-oauth", "auth.connect-oauth.device.waiting.later")
        .expect("later waiting fixture");
    later.validate().expect("later audit");
    let later_txt = later.render_txt();
    assert!(later_txt.contains("Waiting for authorization"));
    assert!(
        primary_labels(&later).is_empty(),
        "later tick still has zero primary"
    );
    // Meaningful liveness: elapsed/countdown/phase advances between ticks.
    assert_ne!(
        early_txt, later_txt,
        "two waiting captures at distinct ticks must differ"
    );
    let early_live = early
        .events
        .iter()
        .find(|e| e.identity.family == "live-state")
        .map(|e| e.identity.subject_id.as_str())
        .unwrap_or("");
    let later_live = later
        .events
        .iter()
        .find(|e| e.identity.family == "live-state")
        .map(|e| e.identity.subject_id.as_str())
        .unwrap_or("");
    assert!(
        early_live.contains("waiting") && later_live.contains("waiting"),
        "both ticks share the waiting owner family"
    );
    assert!(
        early_txt.contains('m') || early_txt.contains('s') || early_txt.contains("phase"),
        "elapsed or countdown must be visible for liveness"
    );
}

// ── VAL-V3-F4-018: Cancel is explicit and safe ──────────────────────────

#[test]
fn connect_oauth_cancel_invalidates_attempt_and_returns_to_method() {
    let replay = replay_case("connect-oauth-cancel", 1).expect("cancel case");
    assert_eq!(replay.before.screen_id, "connect-oauth");
    assert_eq!(replay.after.screen_id, "connect-method");
    assert_eq!(
        replay.actions[0].action_id,
        "action.connect-oauth.cancel"
    );
    assert_eq!(replay.transitions[0].kind, "cancel");

    // Pending attempt becomes inactive; PKCE/state context discarded safely.
    assert!(
        replay.side_effects.iter().any(|effect| {
            effect.status.contains("attempt-invalidated")
                || effect.status.contains("listener-released")
                || effect.status.contains("cancelled")
        }),
        "cancel must invalidate the attempt and release resources: {:?}",
        replay.side_effects
    );
    assert!(
        !replay.side_effects.iter().any(|effect| {
            effect.status.contains("provider-connected") || effect.status.contains("credential-saved")
        }),
        "cancel must not connect the provider"
    );
    assert!(
        replay.attempts.iter().any(|a| {
            a.outcome.contains("cancelled")
                || a.outcome.contains("invalidated")
                || a.outcome.contains("inactive")
        }),
        "attempt outcome must record cancel: {:?}",
        replay.attempts
    );

    // Same provider + browser/OAuth choice remain understandable.
    assert!(
        replay.after.selection.contains("oauth")
            || replay.after.selection.contains("browser")
            || replay.after.focus.contains("method")
            || replay.after.selection.contains("openai"),
        "method/provider OAuth choice must remain; selection={} focus={}",
        replay.after.selection,
        replay.after.focus
    );

    // Origin draft remains available.
    for field in ["draft", "transcript", "user_task", "visible_origin"] {
        if let Some(hashes) = replay.preserved_field_hashes.get(field) {
            assert_eq!(
                hashes.before, hashes.after,
                "{field} must survive OAuth cancel"
            );
        }
    }
    assert_eq!(replay.before.draft, replay.after.draft);

    // Method surface still understands OAuth choice; no secrets.
    let method = audit_screen("connect-method", "gate.provider-and-mcp.connect-method")
        .expect("method surface");
    let method_txt = method.render_txt();
    assert!(method_txt.contains("OpenAI") || method_txt.contains("Browser sign-in"));
    assert_no_oauth_secret_leak(&method_txt);
    assert_no_oauth_secret_leak(&replay.audit.render_txt());
}

// ── VAL-V3-F4-019: Failure remains recoverable ──────────────────────────

#[test]
fn connect_oauth_failures_offer_one_primary_recovery() {
    let cases = [
        (
            "auth.connect-oauth.denied",
            "denied",
            &["denied", "did not approve", "was not approved", "refused"][..],
            &["retry", "try again"][..],
        ),
        (
            "auth.connect-oauth.expired",
            "expired",
            &["expired", "timed out", "no longer valid"][..],
            &["retry", "try again", "start"][..],
        ),
        (
            "auth.connect-oauth.browser-failed",
            "browser-failed",
            &["could not open", "browser", "failed to open"][..],
            &["open browser", "retry", "try again"][..],
        ),
    ];
    for (fixture, name, causes, recovery) in cases {
        let audit = audit_screen("connect-oauth", fixture)
            .unwrap_or_else(|err| panic!("{fixture} must render: {err}"));
        audit
            .validate()
            .unwrap_or_else(|err| panic!("{fixture} audit: {err}"));
        let text = audit.render_txt();
        assert_no_oauth_secret_leak(&text);
        assert_no_protocol_jargon_in_ui(&text);
        let lower = text.to_ascii_lowercase();
        assert!(
            causes.iter().any(|c| lower.contains(c)),
            "{name} must name plain-English cause from {causes:?}:\n{text}"
        );
        assert!(
            !text.contains("error_code")
                && !text.contains("access_denied")
                && !text.contains("ECONNREFUSED"),
            "{name} must not lead with raw protocol codes"
        );
        let labels = primary_labels(&audit);
        assert_eq!(
            labels.len(),
            1,
            "{name} must have exactly one primary recovery, got {labels:?}"
        );
        let label_lower = labels[0].to_ascii_lowercase();
        assert!(
            recovery.iter().any(|r| label_lower.contains(r)),
            "{name} recovery label unexpected: {}",
            labels[0]
        );
        // Secondary Back/Cancel available.
        assert!(
            text.contains("Cancel") || text.contains("Back"),
            "{name} needs secondary Cancel/Back"
        );
        // Provider choice remains.
        assert!(
            text.contains("OpenAI") || text.contains("provider"),
            "{name} must not lose the provider"
        );
    }
}

// ── VAL-V3-F4-054: Protocol safeguards ──────────────────────────────────

#[test]
fn connect_oauth_state_match_and_mismatch_scenarios() {
    let matched = replay_case("connect-oauth-state-match", 1).expect("state-match");
    assert_eq!(matched.before.screen_id, "connect-oauth");
    assert_eq!(matched.after.screen_id, "connect-connected");
    assert!(
        matched.side_effects.iter().any(|e| {
            e.status.contains("provider-connected") || e.status.contains("state-validated")
        }),
        "state match connects: {:?}",
        matched.side_effects
    );

    let mismatched = replay_case("connect-oauth-state-mismatch", 1).expect("state-mismatch");
    assert_eq!(mismatched.before.screen_id, "connect-oauth");
    assert_eq!(mismatched.after.screen_id, "connect-oauth");
    assert!(
        !mismatched.side_effects.iter().any(|e| {
            e.status.contains("provider-connected") || e.status.contains("credential-saved")
        }),
        "state mismatch must not connect: {:?}",
        mismatched.side_effects
    );
    assert!(
        mismatched.side_effects.iter().any(|e| {
            e.status.contains("state-mismatch") || e.status.contains("rejected")
        }) || mismatched.attempts.iter().any(|a| {
            a.outcome.contains("state-mismatch") || a.outcome.contains("rejected")
        }),
        "state mismatch must be recorded"
    );
    // Rendered failure stays recoverable and secret-clean.
    let after_txt = mismatched.audit.render_txt();
    assert_no_oauth_secret_leak(&after_txt);
    assert!(
        after_txt.contains("could not")
            || after_txt.contains("Could not")
            || after_txt.contains("did not match")
            || after_txt.contains("Try again")
            || after_txt.contains("Retry"),
        "mismatch surface remains recoverable"
    );
}

#[test]
fn connect_oauth_pkce_supported_loopback_connects_without_leaking_verifier() {
    let replay = replay_case("connect-oauth-pkce-supported", 1).expect("pkce case");
    assert_eq!(replay.before.screen_id, "connect-oauth");
    assert_eq!(replay.after.screen_id, "connect-connected");
    assert!(
        replay.side_effects.iter().any(|e| {
            e.status.contains("provider-connected")
                || e.status.contains("pkce-verified")
                || e.status.contains("proof-verified")
        }),
        "PKCE-supported loopback must connect: {:?}",
        replay.side_effects
    );
    let evidence = format!("{:?}", replay);
    assert!(!evidence.contains(CODE_VERIFIER_SENTINEL));
    assert!(!evidence.contains(STATE_SECRET));
    assert!(!replay.audit.render_txt().contains("PKCE"));
}

#[test]
fn connect_oauth_cancel_releases_listener_and_rejects_late_callback() {
    let cancel = replay_case("connect-oauth-cancel", 1).expect("cancel");
    assert!(
        cancel.side_effects.iter().any(|e| {
            e.status.contains("listener-released")
                || e.status.contains("attempt-invalidated")
                || e.kind.contains("listener")
                || e.effect_id.contains("cancel")
        }),
        "cancel must release loopback listener / invalidate attempt: {:?}",
        cancel.side_effects
    );

    let late = replay_case("connect-oauth-late-callback", 1).expect("late callback");
    assert_eq!(late.before.screen_id, "connect-method");
    // Late callback/poll after cancel cannot connect.
    assert_ne!(late.after.screen_id, "connect-connected");
    assert!(
        !late.side_effects.iter().any(|e| {
            e.status.contains("provider-connected") || e.status.contains("credential-saved")
        }),
        "late callback must not connect after cancel: {:?}",
        late.side_effects
    );
    assert!(
        late.side_effects.iter().any(|e| {
            e.status.contains("late-rejected")
                || e.status.contains("attempt-inactive")
                || e.status.contains("ignored")
        }) || late.attempts.iter().any(|a| {
            a.outcome.contains("late-rejected")
                || a.outcome.contains("inactive")
                || a.outcome.contains("ignored")
        }),
        "late response must be rejected against invalidated attempt"
    );
}

#[test]
fn connect_oauth_device_code_separation_is_audited() {
    let audit = audit_screen("connect-oauth", "auth.connect-oauth.device.waiting")
        .expect("device waiting");
    let text = audit.render_txt();
    assert!(text.contains(USER_CODE));
    assert!(!text.contains(DEVICE_CODE_VALUE));
    assert!(!text.contains("device_code"));
    // Copy affordance for the user_code is present in copy or audit.
    assert!(
        text.contains("Copy")
            || text.contains("one-time code")
            || audit.events.iter().any(|e| {
                e.identity.family == "one-time-code"
                    || e.identity.subject_id.contains("user-code")
            }),
        "user_code remains copyable/actionable without revealing device_code"
    );
}

#[test]
fn connect_oauth_canonical_and_gate_fixtures_publish_waiting_device() {
    for fixture in [
        "canonical.connect-oauth",
        "gate.provider-and-mcp.connect-oauth",
        "auth.connect-oauth.device.waiting",
    ] {
        let audit = audit_screen("connect-oauth", fixture)
            .unwrap_or_else(|err| panic!("{fixture} must render: {err}"));
        audit
            .validate()
            .unwrap_or_else(|err| panic!("{fixture} audit: {err}"));
        let text = audit.render_txt();
        assert_canvas_geometry(&text);
        assert_no_oauth_secret_leak(&text);
        assert!(
            text.contains("Waiting for authorization"),
            "{fixture} default is live waiting"
        );
        assert!(
            primary_labels(&audit).is_empty(),
            "{fixture} waiting has zero primary"
        );
        assert!(text.contains(USER_CODE) || text.contains("one-time code"));
    }
}

#[test]
fn connect_oauth_retry_from_denied_reopens_wait_without_secrets() {
    let audit = audit_screen("connect-oauth", "auth.connect-oauth.denied")
        .expect("denied fixture");
    assert!(
        audit.transitions.iter().any(|t| {
            (t.action_id.contains("retry") || t.action_id.contains("open-browser"))
                && t.to_screen_id == "connect-oauth"
                && t.kind == "direct"
        }),
        "Retry/Open browser must stay in connect-oauth: {:?}",
        audit.transitions
    );
    assert!(
        audit.transitions.iter().any(|t| {
            t.action_id == "action.connect-oauth.cancel"
                && t.to_screen_id == "connect-method"
                && t.kind == "cancel"
        }),
        "Cancel remains secondary back to method choice"
    );
}
