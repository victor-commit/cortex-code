//! Secret-safe API-key connection (VAL-V3-F4-012..015).
//!
//! Covers masked entry, source/removability copy, validation states with one
//! primary next action, and cancel that discards only the secret input.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

/// Sentinel that must never appear in rendered or audit-visible surfaces.
const SECRET_SENTINEL: &str = "sk-proj-SECRET_SENTINEL_NEVER_ECHO_9f3a2c";

fn assert_no_secret_leak(text: &str) {
    assert!(
        !text.contains(SECRET_SENTINEL),
        "secret sentinel leaked into operator surface"
    );
    assert!(
        !text.contains("fixture-secret-value"),
        "fixture secret value leaked"
    );
    // Reusable full key material must never print literally.
    assert!(
        !text.contains("sk-proj-") || text.contains("·") || text.contains("•") || text.contains('*'),
        "raw project key prefix without masking is forbidden"
    );
    for banned in [
        "client_secret",
        "Authorization: Bearer",
        "OPENAI_API_KEY=",
        "device_code",
    ] {
        assert!(!text.contains(banned), "secret/protocol residue: {banned}");
    }
}

fn assert_no_protocol_jargon(text: &str) {
    for banned in ["oauth2", "PKCE", "stdio", "endpoint URL", "transport"] {
        assert!(
            !text.contains(banned),
            "protocol jargon leaked: {banned}"
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

// ── VAL-V3-F4-012: Masked credential input ──────────────────────────────

#[test]
fn connect_apikey_masks_credential_and_never_echoes_secret() {
    let audit = audit_screen("connect-apikey", "auth.connect-apikey.valid")
        .expect("valid apikey fixture must render");
    audit.validate().expect("apikey audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_secret_leak(&text);
    assert_no_protocol_jargon(&text);

    // Mask characters are present; full reusable secret is not.
    assert!(
        text.contains('·') || text.contains('•') || text.contains('*'),
        "credential field must show mask characters: {text}"
    );
    assert!(
        !text.contains(SECRET_SENTINEL),
        "sentinel must not leak into valid fixture"
    );
    // Cursor may appear, but never next to full secret material.
    assert!(
        text.contains("Add an API key")
            || text.contains("API key")
            || text.contains("Paste"),
        "must identify the task"
    );
    // Audit JSON path without secret: none of the events/actions may quote it.
    let json = audit.to_json();
    assert!(!json.contains(SECRET_SENTINEL));
    assert!(!json.contains("fixture-secret-value"));
}

// ── VAL-V3-F4-013: Storage / source copy ────────────────────────────────

#[test]
fn connect_apikey_source_copy_matches_fixture() {
    let cases = [
        (
            "auth.connect-apikey.local",
            "local",
            true,
            &["local", "this machine", "removable"][..],
        ),
        (
            "auth.connect-apikey.environment",
            "environment",
            false,
            &["environment", "env", "not removable", "cannot remove"][..],
        ),
        (
            "auth.connect-apikey.managed",
            "managed",
            false,
            &["managed", "organization", "not removable", "cannot remove"][..],
        ),
    ];
    for (fixture, source, removable, needles) in cases {
        let audit = audit_screen("connect-apikey", fixture)
            .unwrap_or_else(|err| panic!("{fixture} must render: {err}"));
        audit
            .validate()
            .unwrap_or_else(|err| panic!("{fixture} audit: {err}"));
        let text = audit.render_txt().to_ascii_lowercase();
        assert_no_secret_leak(&audit.render_txt());
        let hit = needles.iter().any(|n| text.contains(n));
        assert!(
            hit,
            "{fixture} must describe source={source} removability={removable}; got:\n{}",
            audit.render_txt()
        );
        assert!(
            text.contains("never") && (text.contains("echo") || text.contains("shown") || text.contains("display")),
            "{fixture} must say the credential is never echoed"
        );
        // Never claim remote upload as storage path for local key entry.
        assert!(
            !text.contains("uploaded to our servers") && !text.contains("sent to factory"),
            "{fixture} must not invent remote upload promises"
        );
        // Semantic subject for source should match fixture.
        assert!(
            audit.events.iter().any(|e| {
                e.identity.family == "credential-source"
                    && (e.identity.subject_id.contains(source)
                        || e.identity.subject_id == source
                        || e.identity.subject_id.contains(&source.replace('-', "")))
            }),
            "{fixture} must emit credential-source semantic subject for {source}"
        );
    }
}

// ── VAL-V3-F4-014: Validation states and one next action ────────────────

#[test]
fn connect_apikey_validation_states_are_distinct() {
    // empty
    let empty = audit_screen("connect-apikey", "auth.connect-apikey.empty")
        .expect("empty fixture");
    empty.validate().expect("empty audit");
    let empty_text = empty.render_txt();
    assert_no_secret_leak(&empty_text);
    assert!(
        empty_text.contains("Paste")
            || empty_text.contains("Enter an API key")
            || empty_text.contains("Type an API key")
            || empty_text.contains("empty"),
        "empty state needs readable empty guidance"
    );
    // Connect must not be an enabled primary when empty.
    let empty_primary_enabled = empty.events.iter().any(|e| {
        e.identity.family == "primary-action"
            && e.action
                .as_ref()
                .map(|a| a.enabled && a.label.to_ascii_lowercase().contains("connect"))
                .unwrap_or(false)
    });
    assert!(
        !empty_primary_enabled,
        "empty must not enable Connect as primary"
    );

    // malformed
    let bad = audit_screen("connect-apikey", "auth.connect-apikey.malformed")
        .expect("malformed fixture");
    bad.validate().expect("malformed audit");
    let bad_text = bad.render_txt();
    assert_no_secret_leak(&bad_text);
    assert!(
        bad_text.contains("malformed")
            || bad_text.contains("incomplete")
            || bad_text.contains("invalid")
            || bad_text.contains("Check the key"),
        "malformed needs readable diagnosis"
    );
    let bad_primary_enabled = bad.events.iter().any(|e| {
        e.identity.family == "primary-action"
            && e.action
                .as_ref()
                .map(|a| a.enabled && a.label.to_ascii_lowercase().contains("connect"))
                .unwrap_or(false)
    });
    assert!(
        !bad_primary_enabled,
        "malformed must not enable Connect"
    );

    // checking (non-final)
    let checking = audit_screen("connect-apikey", "auth.connect-apikey.checking")
        .expect("checking fixture");
    checking.validate().expect("checking audit");
    let checking_text = checking.render_txt();
    assert_no_secret_leak(&checking_text);
    assert!(
        checking_text.contains("Checking") || checking_text.contains("checking"),
        "checking state must be explicit and non-final"
    );
    assert!(
        !checking_text.contains("✓ Connected") && !checking_text.contains("Connected ·"),
        "checking must not claim Connected"
    );
    // checking is not a success verdict; Connect may be busy/disabled.
    let checking_success = checking.events.iter().any(|e| e.identity.family == "success");
    assert!(!checking_success, "checking is not a success family");

    // valid-looking → one Connect primary
    let valid = audit_screen("connect-apikey", "auth.connect-apikey.valid")
        .expect("valid fixture");
    valid.validate().expect("valid audit");
    let valid_text = valid.render_txt();
    assert_no_secret_leak(&valid_text);
    assert!(
        valid_text.contains("Looks valid"),
        "local syntax validation may only claim Looks valid, not Connected"
    );
    assert!(
        !valid_text.contains("✓ Connected"),
        "Connected is reserved for provider success after connect"
    );
    let connect_actions: Vec<_> = valid
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect();
    assert_eq!(
        connect_actions.len(),
        1,
        "exactly one primary action when ready"
    );
    let connect = connect_actions[0]
        .action
        .as_ref()
        .expect("primary must carry action evidence");
    assert!(connect.enabled, "valid-looking enables Connect");
    assert!(
        connect.label.to_ascii_lowercase().contains("connect"),
        "primary label is Connect-ish: {}",
        connect.label
    );
    // Message must not quote the key.
    assert!(!valid_text.contains(SECRET_SENTINEL));
}

#[test]
fn connect_apikey_failure_offers_one_recovery() {
    let audit = audit_screen("connect-apikey", "auth.connect-apikey.failure")
        .expect("failure fixture");
    audit.validate().expect("failure audit");
    let text = audit.render_txt();
    assert_no_secret_leak(&text);
    assert!(
        text.contains("could not")
            || text.contains("Could not")
            || text.contains("failed")
            || text.contains("Failed")
            || text.contains("not accept"),
        "failure must show plain-English cause"
    );
    let primaries: Vec<_> = audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect();
    assert_eq!(primaries.len(), 1, "exactly one recovery primary");
    let primary = primaries[0].action.as_ref().expect("action");
    assert!(primary.enabled);
    let label = primary.label.to_ascii_lowercase();
    assert!(
        label.contains("try") || label.contains("retry") || label.contains("check"),
        "recovery label unexpected: {}",
        primary.label
    );
}

// ── VAL-V3-F4-015: Cancel discards only secret ──────────────────────────

#[test]
fn connect_apikey_cancel_discards_secret_and_returns_to_method() {
    let replay = replay_case("connect-apikey-cancel", 1).expect("cancel case");
    assert_eq!(replay.before.screen_id, "connect-apikey");
    assert_eq!(replay.after.screen_id, "connect-method");
    assert_eq!(
        replay.actions[0].action_id,
        "action.connect-apikey.cancel"
    );
    assert_eq!(replay.transitions[0].kind, "cancel");

    // Provider / method selection preserved in scenario state.
    assert!(
        replay.after.selection.contains("api-key")
            || replay.after.selection.contains("openai")
            || replay.after.selection == "api-key"
            || replay.after.focus.contains("method"),
        "method/provider choice must remain after cancel; selection={} focus={}",
        replay.after.selection,
        replay.after.focus
    );

    // Draft / origin non-secret state preserved.
    for field in ["draft", "transcript", "user_task", "visible_origin"] {
        if let Some(hashes) = replay.preserved_field_hashes.get(field) {
            assert_eq!(
                hashes.before, hashes.after,
                "{field} must survive secret-only cancel"
            );
        }
    }
    assert_eq!(replay.before.draft, replay.after.draft);
    assert_eq!(replay.before.transcript, replay.after.transcript);

    // No credential write side effect; secret discarded.
    assert!(
        replay.side_effects.iter().any(|effect| {
            effect.status.contains("secret-discarded")
                || effect.status.contains("no-change")
                || effect.status.contains("discard")
        }),
        "cancel must discard secret without writing credentials: {:?}",
        replay.side_effects
    );
    assert!(
        !replay.side_effects.iter().any(|effect| {
            effect.status.contains("credential-saved") || effect.status.contains("connected")
        }),
        "cancel must not save/connect"
    );

    // Rendered after state (method screen) and case JSON must not contain sentinel.
    let after_txt = replay.audit.render_txt();
    assert!(!after_txt.contains(SECRET_SENTINEL));
    assert!(!format!("{:?}", replay).contains(SECRET_SENTINEL));

    // Method screen still understands the same provider + API key choice.
    let method = audit_screen("connect-method", "auth.connect-method.openai.api-key")
        .or_else(|_| audit_screen("connect-method", "auth.connect-method.api-key"))
        .or_else(|_| audit_screen("connect-method", "auth.connect-method.openai"))
        .expect("method return fixture");
    let method_txt = method.render_txt();
    assert!(
        method_txt.contains("OpenAI") || method_txt.contains("API key"),
        "return surface keeps provider/method context"
    );
    assert!(!method_txt.contains(SECRET_SENTINEL));
}

#[test]
fn connect_apikey_valid_connect_transition_targets_connected() {
    let audit = audit_screen("connect-apikey", "auth.connect-apikey.valid")
        .expect("valid apikey");
    assert!(
        audit.transitions.iter().any(|t| {
            t.action_id == "action.connect-apikey.connect"
                && t.to_screen_id == "connect-connected"
                && t.kind == "direct"
        }),
        "Connect must advance to connect-connected"
    );
    assert!(
        audit.transitions.iter().any(|t| {
            t.action_id == "action.connect-apikey.cancel"
                && t.to_screen_id == "connect-method"
                && t.kind == "cancel"
        }),
        "Cancel must return to connect-method"
    );
}

#[test]
fn connect_apikey_canonical_default_is_publishable() {
    // Gallery publishes via canonical.connect-apikey when outside first3 flows.
    let audit = audit_screen("connect-apikey", "canonical.connect-apikey")
        .expect("canonical fixture must render");
    audit.validate().expect("canonical audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_secret_leak(&text);
    assert!(text.contains("Looks valid") || text.contains("API key"));
    assert!(
        text.contains("local")
            || text.contains("this machine")
            || text.contains("removable"),
        "default source is removable local storage"
    );
}
