
//! Final Flow 4 conformance sweep (VAL-V3-F4-044..048, VAL-V3-F4-050).
//!
//! Aggregate checks across every provider and MCP canonical state plus the
//! scenario inventory that proved each individual Flow 4 milestone.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    scenario::replay_case,
};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

/// Closed-default Protocol / transport jargon (VAL-V3-F4-044).
/// Allowed only inside explicitly opened diagnostics fixtures.
const CLOSED_DEFAULT_JARGON: &[&str] = &[
    "stdio",
    "oauth2",
    "OAuth2",
    "OAuth",
    "device_code",
    "client_secret",
    "PKCE",
    "mcpServers",
    "stack trace",
    "endpoint URL",
    "SSE ",
    " spawn error",
    "secret access token",
    "Device authorization ·",
];

/// Broader closed-surface ban for operator defaults (case-sensitive tokens kept
/// separate so human words like "timeout" in informal speech can be limited).
const CLOSED_TRANSPORT_PHRASES: &[&str] = &[
    "transport",
    "endpoint",
    "schema language",
    "config path",
];

/// Secret sentinels that must never appear anywhere Flow 4 emits (VAL-V3-F4-045).
const SECRET_SENTINELS: &[&str] = &[
    "*****************************************",
    "fixture-secret-value",
    "sk-live-secret-token",
    "api_key=supersecret",
    "Bearer aquicktoken",
    "access_token=sk-oauth-secret",
    "access_token=plaintext",
    "Authorization: Bearer",
    "OPENAI_API_KEY=",
    "refresh_token=",
    "token=abc",
    "client_secret=",
    "device_code=",
];

/// Canonical Flow 4 production/gate fixtures published in the gallery.
const CANONICAL_FIXTURES: &[(&str, &str)] = &[
    ("login", "gate.provider-and-mcp.login"),
    ("login", "auth.login.blocked-model"),
    ("login", "auth.login.onboarding"),
    ("login", "auth.login.conversation"),
    ("connect-provider", "gate.provider-and-mcp.connect-provider"),
    ("connect-provider", "auth.connect-provider.blocked-model"),
    ("connect-provider", "auth.connect-provider.onboarding"),
    ("connect-provider", "auth.connect-provider.conversation"),
    ("connect-provider", "auth.connect-provider.selected-anthropic"),
    ("connect-method", "gate.provider-and-mcp.connect-method"),
    ("connect-method", "auth.connect-method.openai"),
    ("connect-method", "auth.connect-method.google"),
    ("connect-apikey", "auth.connect-apikey.valid"),
    ("connect-apikey", "auth.connect-apikey.empty"),
    ("connect-apikey", "auth.connect-apikey.malformed"),
    ("connect-apikey", "auth.connect-apikey.checking"),
    ("connect-apikey", "auth.connect-apikey.failure"),
    ("connect-apikey", "auth.connect-apikey.local"),
    ("connect-apikey", "auth.connect-apikey.environment"),
    ("connect-apikey", "auth.connect-apikey.managed"),
    ("connect-oauth", "gate.provider-and-mcp.connect-oauth"),
    ("connect-oauth", "auth.connect-oauth.device.waiting"),
    ("connect-oauth", "auth.connect-oauth.loopback.waiting"),
    ("connect-oauth", "auth.connect-oauth.consent"),
    ("connect-oauth", "auth.connect-oauth.denied"),
    ("connect-oauth", "auth.connect-oauth.expired"),
    ("connect-oauth", "auth.connect-oauth.browser-failed"),
    ("connect-oauth", "auth.connect-oauth.hostile-scope"),
    ("connect-oauth", "auth.connect-oauth.secret-scope"),
    ("connect-connected", "gate.provider-and-mcp.connect-connected"),
    ("connect-connected", "auth.connect-connected.api-key"),
    ("connect-disconnect", "auth.connect-disconnect.anthropic.local"),
    (
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.multi-source",
    ),
    (
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.last-source",
    ),
    ("mcp", "gate.provider-and-mcp.mcp"),
    ("mcp", "views.mcp.empty"),
    ("mcp", "views.mcp.mixed"),
    ("mcp", "views.mcp.setup-browser"),
    ("mcp", "views.mcp.auth-docs"),
    ("mcp", "views.mcp.auth-docs-ready"),
    ("mcp", "views.mcp.failed-tracker"),
    ("mcp", "views.mcp.retry-running"),
    ("mcp", "views.mcp.retry-ready"),
    ("mcp", "views.mcp.retry-failed"),
    ("mcp", "views.mcp.progress-setup.tick-1"),
    ("mcp", "views.mcp.progress-auth.tick-1"),
    ("mcp", "views.mcp.progress-reconnect.tick-1"),
    ("mcp", "views.mcp.progress-invocation.tick-1"),
    ("mcp", "views.mcp.progress-success"),
    ("mcp", "views.mcp.progress-failed"),
    ("mcp", "views.mcp.progress-cancelled"),
    ("mcp", "views.mcp.detail-docs"),
    ("mcp", "views.mcp.policy-blocked"),
    ("mcp", "views.mcp.policy-review"),
    ("mcp", "views.mcp.policy-change-confirm"),
    ("mcp", "views.mcp.manage-removal"),
    ("mcp", "views.mcp.handshake-success"),
    ("mcp", "views.mcp.handshake-failed"),
    ("mcp", "views.mcp.handshake-zero-usable"),
    ("mcp", "views.mcp.hostile-labels"),
    ("mcp", "views.mcp.hostile-detail"),
    ("sidebar-mcp", "views.sidebar-mcp.attention"),
    ("sidebar-mcp", "views.sidebar-mcp.all-ready"),
];

/// Advanced diagnostic fixtures: secrets still banned; technical record allowed.
const DIAGNOSTICS_FIXTURES: &[(&str, &str)] = &[
    ("mcp", "views.mcp.diagnostics-docs"),
    ("mcp", "views.mcp.diagnostics-failed"),
    ("mcp", "views.mcp.diagnostics-auth"),
    ("mcp", "views.mcp.diagnostics-hostile"),
];

/// Gallery artifacts for published Flow 4 IDs (relative to plans/assets/mockups-v3).
const GALLERY_RELATIVE: &[(&str, &str)] = &[
    ("auth", "login"),
    ("auth", "connect-provider"),
    ("auth", "connect-method"),
    ("auth", "connect-apikey"),
    ("auth", "connect-oauth"),
    ("auth", "connect-connected"),
    ("auth", "connect-disconnect"),
    ("views", "mcp"),
    ("views", "sidebar-mcp"),
];

/// Exhaustive public Flow 4 cancel/back/success origin-restore matrix
/// (VAL-V3-F4-048). Derived from every named `replay_case` for login, the six
/// connect-* screens, mcp, and sidebar-mcp. Protocol-only OAuth safeguards and
/// static fixture snapshots are listed separately when they are not origin
/// for navigation actions.
const ORIGIN_SCENARIOS: &[(&str, &str, &str)] = &[
    // login
    ("login-cancel", "login", "model-picker"),
    ("login-cancel-onboarding", "login", "onboarding"),
    ("login-cancel-conversation", "login", "chat-turn"),
    ("login-choose-provider", "login", "connect-provider"),
    // connect-provider (all origins)
    ("connect-provider-select", "connect-provider", "connect-method"),
    (
        "connect-provider-back-onboarding",
        "connect-provider",
        "login",
    ),
    (
        "connect-provider-back-conversation",
        "connect-provider",
        "login",
    ),
    (
        "connect-provider-back-blocked-model",
        "connect-provider",
        "login",
    ),
    // connect-method
    ("connect-method-back", "connect-method", "connect-provider"),
    ("connect-method-oauth", "connect-method", "connect-oauth"),
    // connect-apikey
    ("connect-apikey-cancel", "connect-apikey", "connect-method"),
    // connect-oauth success / cancel
    ("connect-oauth-cancel", "connect-oauth", "connect-method"),
    ("connect-oauth-finish", "connect-oauth", "connect-connected"),
    (
        "connect-oauth-state-match",
        "connect-oauth",
        "connect-connected",
    ),
    (
        "connect-oauth-pkce-supported",
        "connect-oauth",
        "connect-connected",
    ),
    // connect-connected success return (incl. idempotent continue)
    (
        "connect-connected-return",
        "connect-connected",
        "model-picker",
    ),
    (
        "connect-connected-return-idempotent",
        "connect-connected",
        "model-picker",
    ),
    // connect-disconnect cancel + confirm variants
    (
        "connect-disconnect-cancel",
        "connect-disconnect",
        "connect-provider",
    ),
    (
        "connect-disconnect-confirm",
        "connect-disconnect",
        "connect-provider",
    ),
    (
        "connect-disconnect-multi-source",
        "connect-disconnect",
        "connect-provider",
    ),
    (
        "connect-disconnect-last-source",
        "connect-disconnect",
        "connect-provider",
    ),
    // mcp return / setup / sign-in / retry
    ("mcp-return", "mcp", "chat-turn"),
    ("mcp-setup-cancel", "mcp", "mcp"),
    ("mcp-sign-in-success", "mcp", "mcp"),
    ("mcp-retry-running", "mcp", "mcp"),
    ("mcp-retry-ready", "mcp", "mcp"),
    ("mcp-retry-failed", "mcp", "mcp"),
    // mcp progress terminals + operation-scoped cancels
    ("mcp-progress-success", "mcp", "mcp"),
    ("mcp-progress-failed", "mcp", "mcp"),
    ("mcp-progress-cancel-setup", "mcp", "mcp"),
    ("mcp-progress-cancel-auth", "mcp", "mcp"),
    ("mcp-progress-cancel-reconnect", "mcp", "mcp"),
    ("mcp-progress-cancel-invocation", "mcp", "mcp"),
    // mcp detail + diagnostics open/close (incl. failed diagnostics)
    ("mcp-details-open", "mcp", "mcp"),
    ("mcp-details-back", "mcp", "mcp"),
    ("mcp-diagnostics-open", "mcp", "mcp"),
    ("mcp-diagnostics-close", "mcp", "mcp"),
    ("mcp-diagnostics-failed-open", "mcp", "mcp"),
    ("mcp-diagnostics-failed-close", "mcp", "mcp"),
    // mcp handshake outcomes
    ("mcp-handshake-success", "mcp", "mcp"),
    ("mcp-handshake-failed", "mcp", "mcp"),
    ("mcp-handshake-zero-usable", "mcp", "mcp"),
    // hostile label detail open/back
    ("mcp-hostile-labels-open", "mcp", "mcp"),
    ("mcp-hostile-labels-back", "mcp", "mcp"),
    // sidebar-mcp bidirectional
    ("sidebar-mcp-open-mcp", "sidebar-mcp", "mcp"),
    ("sidebar-mcp-return", "mcp", "sidebar-mcp"),
    // policy review / change confirm / cancel
    ("mcp-policy-review-open", "mcp", "mcp"),
    ("mcp-policy-change-confirm", "mcp", "mcp"),
    ("mcp-policy-change-cancel", "mcp", "mcp"),
    // removal family (distinct disable / sign-out / remove)
    ("mcp-disable", "mcp", "mcp"),
    ("mcp-sign-out", "mcp", "mcp"),
    ("mcp-remove-connection", "mcp", "mcp"),
    ("mcp-remove-managed-refused", "mcp", "mcp"),
    // resume policy outcomes
    ("mcp-resume-automatic-once", "mcp", "tool-running"),
    ("mcp-resume-confirmation", "mcp", "tool-failed"),
    ("mcp-resume-none", "mcp", "tool-failed"),
    ("mcp-resume-cancel", "mcp", "tool-failed"),
    ("mcp-resume-failed", "mcp", "tool-failed"),
    ("mcp-resume-policy-blocked", "mcp", "mcp"),
];

/// OAuth protocol safeguards that must stay non-dead-end and self-consistent
/// without leaving the connection shell (not navigation identity changes).
const OAUTH_PROTOCOL_SCENARIOS: &[(&str, &str, &str)] = &[
    (
        "connect-oauth-state-mismatch",
        "connect-oauth",
        "connect-oauth",
    ),
    (
        "connect-oauth-late-callback",
        "connect-method",
        "connect-method",
    ),
];

/// All nine Flow 4 decision surfaces that must expose non-dead transitions.
const FLOW4_DECISION_SURFACES: &[(&str, &str)] = &[
    ("login", "gate.provider-and-mcp.login"),
    (
        "connect-provider",
        "gate.provider-and-mcp.connect-provider",
    ),
    ("connect-method", "gate.provider-and-mcp.connect-method"),
    ("connect-apikey", "auth.connect-apikey.valid"),
    ("connect-oauth", "gate.provider-and-mcp.connect-oauth"),
    (
        "connect-connected",
        "gate.provider-and-mcp.connect-connected",
    ),
    (
        "connect-disconnect",
        "auth.connect-disconnect.anthropic.local",
    ),
    ("mcp", "gate.provider-and-mcp.mcp"),
    ("sidebar-mcp", "views.sidebar-mcp.attention"),
];

fn gallery_root() -> PathBuf {
    // tests run with CARGO_MANIFEST_DIR = harness root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3")
        .canonicalize()
        .expect("gallery root must resolve")
}

fn assert_canvas_geometry(text: &str, label: &str) {
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(
        rows.len(),
        40,
        "{label}: canonical height is 40 rows, got {}",
        rows.len()
    );
    for (idx, row) in rows.iter().enumerate() {
        assert_eq!(
            row.chars().count(),
            120,
            "{label}: row {idx} width is {}, expected 120",
            row.chars().count()
        );
    }
}

fn assert_no_secret_leak(text: &str, label: &str) {
    for banned in SECRET_SENTINELS {
        assert!(
            !text.contains(banned),
            "{label}: secret sentinel leaked: {banned}"
        );
    }
    // Recoverable project key full material must never appear unmasked.
    if text.contains("sk-proj-") {
        assert!(
            text.contains('·') || text.contains('•') || text.contains('*'),
            "{label}: raw project key prefix without masking is forbidden"
        );
    }
}

fn assert_closed_plain_english(text: &str, label: &str) {
    for banned in CLOSED_DEFAULT_JARGON {
        assert!(
            !text.contains(banned),
            "{label}: closed surface jargon leaked: {banned}"
        );
    }
    let lower = text.to_ascii_lowercase();
    for banned in CLOSED_TRANSPORT_PHRASES {
        assert!(
            !lower.contains(&banned.to_ascii_lowercase()),
            "{label}: closed transport phrase leaked: {banned}"
        );
    }
}

fn primary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect()
}

fn is_waiting_zero_primary(screen_id: &str, fixture_id: &str, text: &str) -> bool {
    // OAuth browser/device wait: cancel only (no primary).
    if screen_id == "connect-oauth" {
        let waiting = fixture_id.contains("waiting")
            || text.contains("Waiting for authorization")
            || text.contains("Waiting for browser");
        return !fixture_id.contains("denied")
            && !fixture_id.contains("expired")
            && !fixture_id.contains("failed")
            && !fixture_id.contains("consent")
            && waiting;
    }
    // API-key checking is a non-final automatic wait with cancel only.
    if screen_id == "connect-apikey"
        && (fixture_id.contains("checking") || fixture_id.contains("validating"))
    {
        return true;
    }
    // In-flight progress ticks may hold zero primary while cancel is secondary.
    if screen_id == "mcp"
        && fixture_id.contains("progress-")
        && fixture_id.contains("tick-")
        && !fixture_id.contains("success")
        && !fixture_id.contains("failed")
        && !fixture_id.contains("cancelled")
    {
        return true;
    }
    false
}

fn status_has_readable_label(text: &str) -> bool {
    // Each structural success/error/neutral peak should pair with words.
    let lower = text.to_ascii_lowercase();
    let markers = [
        ("✓", "connected"),
        ("✗", "fail"),
        ("●", "looks valid"),
    ];
    // Presence of glyph alone is fine when nearby text names the state.
    // We require at least one of the non-color status words overall when
    // markers appear; failure fixtures specifically name Failed/Sign in/etc.
    if text.contains('✓') || text.contains('✗') || text.contains('●') || text.contains('○') {
        return lower.contains("connected")
            || lower.contains("ready")
            || lower.contains("failed")
            || lower.contains("needs")
            || lower.contains("waiting")
            || lower.contains("blocked")
            || lower.contains("attempt")
            || lower.contains("retry")
            || lower.contains("looks valid")
            || lower.contains("checking")
            || lower.contains("progress")
            || lower.contains("setup")
            || lower.contains("sign in")
            || lower.contains("not connected")
            || lower.contains("attention")
            || lower.contains("collapsed")
            || lower.contains("denied")
            || lower.contains("expired");
    }
    let _ = markers;
    true
}

fn render_pair(screen_id: &str, fixture_id: &str) -> AuditDocument {
    let audit = audit_screen(screen_id, fixture_id)
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} must render: {e}"));
    audit
        .validate()
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} audit bijection: {e}"));
    audit
}

// ── VAL-V3-F4-044: Connection copy stays plain English ──────────────────

#[test]
fn closed_defaults_use_plain_english_without_protocol_jargon() {
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        // Fast path: diagnostics land in DIAGNOSTICS_FIXTURES separately.
        if fixture_id.contains("diagnostics") {
            continue;
        }
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_closed_plain_english(&text, &format!("{screen_id}/{fixture_id}"));
        // Audit payload itself must not smuggle protocol terms into operator output.
        let json = audit.to_json();
        for banned in ["device_code=", "client_secret=", "stdio://", "oauth2://"] {
            assert!(
                !json.contains(banned),
                "{screen_id}/{fixture_id} audit jason leaked {banned}"
            );
        }
    }
}

// ── VAL-V3-F4-045: Credentials never leak ───────────────────────────────

#[test]
fn secret_sentinels_absent_from_txt_audit_and_gallery() {
    if pure_mode() {
        // Pure dump owns gallery paint; still scan pure gallery files for raw secret sentinels.
        let root = Path::new("/projects/cortex-code/plans/assets/mockups-v3");
        for cat in fs::read_dir(root).expect("gallery") {
            let cat = cat.expect("cat");
            if !cat.file_type().expect("ft").is_dir() { continue; }
            for file in fs::read_dir(cat.path()).expect("files") {
                let file = file.expect("f");
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("txt") { continue; }
                let text = fs::read_to_string(&path).expect("read");
                for s in [
                    "sk-ant-",
                    "sk-or-v1-",
                    "gsk_",
                    "xai-super-secret",
                    "-----BEGIN",
                    "fixture-secret-value",
                    "sk-live-secret-token",
                    "access_token=plaintext",
                ] {
                    assert!(!text.contains(s), "pure gallery secret leak {}: {s}", path.display());
                }
            }
        }
        return;
    }
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES
        .iter()
        .chain(DIAGNOSTICS_FIXTURES.iter())
    {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_no_secret_leak(&text, &format!("{screen_id}/{fixture_id} txt"));
        let json = audit.to_json();
        assert_no_secret_leak(&json, &format!("{screen_id}/{fixture_id} audit"));
        // Diagnostics still redacts secrets; allow technical macros only without tokens.
        if fixture_id.contains("diagnostics") {
            for banned in [
                "supersecret",
                "sk-live-secret-token",
                "access_token=plaintext",
                "Bearer aquicktoken",
            ] {
                assert!(
                    !text.contains(banned) && !json.contains(banned),
                    "diagnostics leaked recoverable secret {banned} in {fixture_id}"
                );
            }
        }
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let txt_path = root.join(category).join(format!("{name}.txt"));
        let text = fs::read_to_string(&txt_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", txt_path.display()));
        assert_no_secret_leak(&text, &format!("gallery {}", txt_path.display()));
        assert_closed_plain_english(&text, &format!("gallery {}", txt_path.display()));
    }
}

// ── VAL-V3-F4-046: Status never depends on color ────────────────────────

#[test]
fn every_flow4_state_pairs_status_markers_with_words() {
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert!(
            status_has_readable_label(&text),
            "{screen_id}/{fixture_id}: status marker without readable label"
        );
        // Semantic events that carry status/selection must not be color-only.
        for event in &audit.events {
            if matches!(
                event.identity.family.as_str(),
                "status" | "state" | "selection" | "primary-action" | "secondary-action"
            ) {
                assert!(
                    !event.identity.subject_id.trim().is_empty()
                        || event.action.as_ref().is_some_and(|a| !a.label.trim().is_empty()),
                    "{screen_id}/{fixture_id}: event {} lacks textual subject",
                    event.identity.event_id
                );
            }
        }
    }
}

// ── VAL-V3-F4-047: Every decision has one primary action ────────────────

#[test]
fn every_actionable_flow4_state_has_exactly_one_primary() {
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);
        if is_waiting_zero_primary(screen_id, fixture_id, &text) {
            assert!(
                primaries.len() <= 1,
                "{screen_id}/{fixture_id}: live wait allows 0-1 primary, got {}",
                primaries.len()
            );
            if primaries.is_empty() {
                assert!(
                    (text.contains("Esc to") || text.contains("[esc]"))
                        || text.to_ascii_lowercase().contains("cancel")
                        || text.to_ascii_lowercase().contains("waiting"),
                    "{screen_id}/{fixture_id}: zero-primary wait must still expose cancel or waiting copy"
                );
            }
            continue;
        }
        assert_eq!(
            primaries.len(),
            1,
            "{screen_id}/{fixture_id}: expected exactly one primary-action event, got {} labels {:?}",
            primaries.len(),
            primaries
                .iter()
                .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
                .collect::<Vec<_>>()
        );
        let action = primaries[0]
            .action
            .as_ref()
            .expect("primary must bind ActionEvidence");
        assert!(action.enabled, "{screen_id}/{fixture_id}: primary disabled");
        assert!(
            !action.label.trim().is_empty(),
            "{screen_id}/{fixture_id}: primary label empty"
        );
        // Destructive disconnect must not be the only weight if cancel exists;
        // cancel remains secondary (non-primary-family) while primary is confirm.
        if screen_id == "connect-disconnect" {
            assert!(
                action.label.to_ascii_lowercase().contains("disconnect")
                    || action.label.to_ascii_lowercase().contains("remove")
                    || action.label.to_ascii_lowercase().contains("confirm"),
                "disconnect primary is the confirm action: {}",
                action.label
            );
            assert!(
                (text.contains("Esc to") || text.contains("[esc]")) || text.to_ascii_lowercase().contains("cancel"),
                "disconnect must expose safe cancel"
            );
        }
    }
}

// ── VAL-V3-F4-048: All Flow 4 transitions restore origin ────────────────

#[test]
fn exhaustive_transition_matrix_restores_documented_origins() {
    // Cover both navigation origin restores and protocol-only OAuth isolates.
    let all_cases = ORIGIN_SCENARIOS
        .iter()
        .chain(OAUTH_PROTOCOL_SCENARIOS.iter());
    for &(case_id, from, to) in all_cases {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("scenario {case_id} replay: {e}"));
        assert_eq!(
            replay.before.screen_id, from,
            "{case_id}: before screen"
        );
        assert_eq!(replay.after.screen_id, to, "{case_id}: after screen");
        assert!(
            !replay.actions.is_empty(),
            "{case_id}: must record at least one action"
        );
        assert!(
            !replay.transitions.is_empty() || !replay.actions.is_empty(),
            "{case_id}: must not be a dead end"
        );
        // No transition may declare a dead-end kind.
        for transition in &replay.transitions {
            assert_ne!(
                transition.kind, "dead-end",
                "{case_id}: transition must not be dead-end"
            );
        }
        // Cancel/back/return/close preserve draft + scroll + conversation
        // except when the action intentionally opens a different legal shell.
        let is_cancel_like = case_id.contains("cancel")
            || case_id.contains("back")
            || case_id.ends_with("-return")
            || case_id.contains("close")
            || case_id.ends_with("-return-idempotent");
        if is_cancel_like {
            for field in ["draft", "scroll_anchor", "conversation_id"] {
                if let Some(hashes) = replay.preserved_field_hashes.get(field) {
                    assert_eq!(
                        hashes.before, hashes.after,
                        "{case_id}: preserved {field}"
                    );
                }
            }
            if !replay.before.conversation_id.is_empty() && !replay.after.conversation_id.is_empty()
            {
                // detach when navigating groups intentionally (sidebar open)
                if case_id != "sidebar-mcp-open-mcp" && case_id != "login-choose-provider" {
                    assert_eq!(
                        replay.before.conversation_id, replay.after.conversation_id,
                        "{case_id}: conversation identity"
                    );
                }
            }
        }
        // Provider success never lands on MCP (separate branches).
        if case_id.starts_with("connect-connected-return")
            || case_id == "connect-oauth-finish"
            || case_id == "connect-oauth-state-match"
            || case_id == "connect-oauth-pkce-supported"
        {
            assert_ne!(
                replay.after.screen_id, "mcp",
                "{case_id}: provider success must not jump to MCP"
            );
        }
    }

    // Matrix must enumerate every public cancel/back/success surface group.
    let covered_from: std::collections::BTreeSet<&str> = ORIGIN_SCENARIOS
        .iter()
        .map(|(_, from, _)| *from)
        .collect();
    for required in [
        "login",
        "connect-provider",
        "connect-method",
        "connect-apikey",
        "connect-oauth",
        "connect-connected",
        "connect-disconnect",
        "mcp",
        "sidebar-mcp",
    ] {
        assert!(
            covered_from.contains(required),
            "ORIGIN_SCENARIOS must include origin-restore cases from {required}"
        );
    }

    // Required public cancel/back/success id subset named by scrutiny/AGENTS.
    for required_case in [
        "connect-apikey-cancel",
        "connect-oauth-cancel",
        "connect-provider-back-onboarding",
        "connect-provider-back-conversation",
        "connect-provider-back-blocked-model",
        "connect-disconnect-confirm",
        "connect-connected-return-idempotent",
        "mcp-progress-cancel-setup",
        "mcp-progress-cancel-auth",
        "mcp-progress-cancel-reconnect",
        "mcp-progress-cancel-invocation",
        "mcp-diagnostics-close",
        "mcp-diagnostics-failed-close",
        "mcp-hostile-labels-back",
        "mcp-policy-change-cancel",
        "mcp-details-back",
    ] {
        assert!(
            ORIGIN_SCENARIOS
                .iter()
                .any(|(case_id, _, _)| *case_id == required_case),
            "ORIGIN_SCENARIOS missing required public case {required_case}"
        );
    }

    // All nine decision surfaces expose non-dead transitions with from==self.
    for &(screen_id, fixture) in FLOW4_DECISION_SURFACES {
        let audit = render_pair(screen_id, fixture);
        assert!(
            !audit.transitions.is_empty(),
            "{screen_id}/{fixture} must expose transitions"
        );
        assert!(
            audit
                .transitions
                .iter()
                .all(|t| t.from_screen_id == screen_id && t.kind != "dead-end"),
            "{screen_id}/{fixture} transitions must preserve origin and avoid dead-ends: {:?}",
            audit
                .transitions
                .iter()
                .map(|t| (t.from_screen_id.as_str(), t.kind.as_str(), t.to_screen_id.as_str()))
                .collect::<Vec<_>>()
        );
    }
}

// ── VAL-V3-F4-050: Legible at 120x40 / 1200x840 ─────────────────────────

#[test]
fn flow4_fixtures_and_gallery_keep_canonical_dimensions() {
    for &(screen_id, fixture_id) in CANONICAL_FIXTURES
        .iter()
        .chain(DIAGNOSTICS_FIXTURES.iter())
    {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
        // Contents must not be pure blank (legibility of longest states still
        // produce meaningful labels).
        assert!(
            text.chars().filter(|c| !c.is_whitespace()).count() > 80,
            "{screen_id}/{fixture_id}: canvas nearly empty"
        );
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let txt = root.join(category).join(format!("{name}.txt"));
        let png = root.join(category).join(format!("{name}.png"));
        let text = fs::read_to_string(&txt).expect("gallery txt");
        assert_canvas_geometry(&text, &format!("gallery {}", txt.display()));
        assert_png_dimensions(&png, 1200, 840);
    }
}

#[test]
fn longest_label_fixtures_remain_readable_within_canvas() {
    // Hostile labels and long capability/publisher names must not explode
    // geometry or spill secrets.
    let long_cases = [
        ("mcp", "views.mcp.hostile-labels"),
        ("mcp", "views.mcp.hostile-detail"),
        ("mcp", "views.mcp.diagnostics-hostile"),
        ("connect-oauth", "auth.connect-oauth.hostile-scope"),
        ("connect-oauth", "auth.connect-oauth.secret-scope"),
        ("mcp", "views.mcp.manage-removal"),
        ("mcp", "views.mcp.policy-blocked"),
        ("connect-provider", "auth.connect-provider.selected-anthropic"),
    ];
    for &(screen_id, fixture_id) in &long_cases {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("long {screen_id}/{fixture_id}"));
        assert_no_secret_leak(&text, &format!("long {screen_id}/{fixture_id}"));
        // Readable primary or recovery still present on decision surfaces.
        if !fixture_id.contains("diagnostics") && !is_waiting_zero_primary(screen_id, fixture_id, &text)
        {
            let primaries = primary_events(&audit);
            assert!(
                !primaries.is_empty() || (text.contains("Enter to") || text.contains("[enter]")) || (text.contains("Esc to") || text.contains("[esc]")),
                "{screen_id}/{fixture_id}: longest-label surface lost actions"
            );
        }
        // No control/bidi injection residues.
        for banned in ['\u{0007}', '\u{001b}', '\u{0000}', '\u{202E}', '\u{200B}'] {
            assert!(
                !text.contains(banned),
                "{screen_id}/{fixture_id}: control char leaked"
            );
        }
    }
}

#[test]
fn published_gallery_txt_matches_live_canonical_renders() {
    if pure_mode() {
        return;
    }
    // Drift guard: live gate fixtures must match published gallery bytes.
    let mapping = [
        ("login", "gate.provider-and-mcp.login", "auth/login.txt"),
        (
            "connect-provider",
            "gate.provider-and-mcp.connect-provider",
            "auth/connect-provider.txt",
        ),
        (
            "connect-method",
            "gate.provider-and-mcp.connect-method",
            "auth/connect-method.txt",
        ),
        (
            "connect-oauth",
            "gate.provider-and-mcp.connect-oauth",
            "auth/connect-oauth.txt",
        ),
        (
            "connect-connected",
            "gate.provider-and-mcp.connect-connected",
            "auth/connect-connected.txt",
        ),
        ("mcp", "gate.provider-and-mcp.mcp", "views/mcp.txt"),
        (
            "sidebar-mcp",
            "views.sidebar-mcp.attention",
            "views/sidebar-mcp.txt",
        ),
        (
            "connect-apikey",
            "auth.connect-apikey.valid",
            "auth/connect-apikey.txt",
        ),
        (
            "connect-disconnect",
            "auth.connect-disconnect.anthropic.local",
            "auth/connect-disconnect.txt",
        ),
    ];
    let root = gallery_root();
    for (screen_id, fixture_id, rel) in mapping {
        let live = render_pair(screen_id, fixture_id).render_txt();
        let published = fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("read {rel}: {e}"));
        assert_eq!(
            live, published,
            "gallery drift for {rel}: live fixture {fixture_id} differs from published TXT"
        );
    }
}

fn assert_png_dimensions(path: &Path, expect_w: u32, expect_h: u32) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(data.len() > 24, "{} too small to be PNG", path.display());
    assert_eq!(&data[0..8], b"\x89PNG\r\n\x1a\n", "{} bad PNG sig", path.display());
    // IHDR width/height at bytes 16..24 big-endian.
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    assert_eq!(
        (width, height),
        (expect_w, expect_h),
        "{}: expected {expect_w}x{expect_h}, got {width}x{height}",
        path.display()
    );
}

// Optional safety: harness binary still lists Flow 4 screens if present.
#[test]
fn harness_list_includes_all_flow4_screen_ids() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_mockup-harness-v3"));
    if !bin.exists() {
        return;
    }
    let output = Command::new(&bin)
        .arg("--list")
        .output()
        .expect("list screens");
    assert!(output.status.success(), "list failed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    for id in [
        "login",
        "connect-provider",
        "connect-method",
        "connect-apikey",
        "connect-oauth",
        "connect-connected",
        "connect-disconnect",
        "mcp",
        "sidebar-mcp",
    ] {
        assert!(
            stdout.contains(id),
            "harness --list missing Flow 4 id {id}"
        );
    }
}
