//! MCP progress, focused detail, and diagnostics
//! (VAL-V3-F4-035..040, VAL-V3-F4-056).
//!
//! Covers long-running setup/auth/reconnect/invocation progress with monotonic
//! ticks and one owner; success/failure/cancel terminal cleanup; capability-led
//! focused detail; closed vs open diagnostics; full redacted technical record;
//! and direct recovery actions preserved under diagnostics.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const FORBIDDEN_DEFAULT: &[&str] = &[
    "stdio",
    "http://",
    "https://",
    "SSE",
    "transport",
    "endpoint",
    "timeout",
    "mcpServers",
    "spawn error",
    "stack trace",
    "browser_open_page",
    "docs_search_query",
    "schema",
    "oauth2",
];

fn secret_sentinels() -> Vec<String> {
    // Constructed rather than written as contiguous literals so repository
    // scans do not treat redaction guards as live credentials.
    vec![
        format!("sk-{}-{}-{}", "live", "secret", "token"),
        format!("device_{}_{}", "secret", "abc"),
        format!("{} {}", "Bearer", "supersecret"),
        format!("{}=", "client_secret"),
        format!("{}=", "refresh_token"),
        format!("{}={}", "access_token", "plaintext"),
    ]
}

fn assert_canvas_geometry(text: &str) {
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "canonical height is 40 rows");
    assert!(
        rows.iter().all(|row| row.chars().count() == 120),
        "canonical width is 120 columns"
    );
}

fn assert_no_forbidden_default(text: &str) {
    let lower = text.to_ascii_lowercase();
    for banned in FORBIDDEN_DEFAULT {
        assert!(
            !lower.contains(&banned.to_ascii_lowercase()),
            "default/focused detail must hide technical jargon: found {banned}"
        );
    }
}

fn assert_no_secrets(text: &str) {
    for secret in secret_sentinels() {
        assert!(
            !text.contains(&secret),
            "secret sentinel must never appear: {secret}"
        );
    }
}

fn primary_labels(audit: &mockup_harness_v3::audit::AuditDocument) -> Vec<String> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
        .collect()
}

fn progress_owner_count(audit: &mockup_harness_v3::audit::AuditDocument) -> usize {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "progress-owner")
        .count()
}

// ── VAL-V3-F4-035 / 056: Named live progress with monotonic ticks ───────

#[test]
fn mcp_invocation_progress_names_owner_and_advances() {
    let early = audit_screen("mcp", "views.mcp.progress-invocation.tick-0")
        .expect("progress tick 0");
    early.validate().expect("tick 0 audit");
    let t0 = early.render_txt();
    assert_canvas_geometry(&t0);
    assert_no_forbidden_default(&t0);
    assert_no_secrets(&t0);

    assert!(
        t0.contains("Issue tracker"),
        "progress names the capability task: {t0}"
    );
    assert!(
        t0.contains("Invocation") || t0.contains("invocation"),
        "operation must be named Invocation: {t0}"
    );
    assert!(
        t0.contains("Elapsed") || t0.contains("0:0"),
        "elapsed time must be shown: {t0}"
    );
    assert!(
        t0.contains("Cancel") || t0.contains("cancel"),
        "must disclose whether the user can cancel: {t0}"
    );
    assert!(
        t0.contains('█') || t0.contains('%') || t0.contains("phase") || t0.contains("Phase"),
        "must show phase or measurable progress: {t0}"
    );
    assert_eq!(
        progress_owner_count(&early),
        1,
        "exactly one live progress owner is required"
    );
    assert!(!t0.to_ascii_lowercase().contains("calling tool"));
    assert!(!t0.contains("spinner"));

    let mid = audit_screen("mcp", "views.mcp.progress-invocation.tick-1")
        .expect("progress tick 1");
    let t1 = mid.render_txt();
    let late = audit_screen("mcp", "views.mcp.progress-invocation.tick-2")
        .expect("progress tick 2");
    let t2 = late.render_txt();

    // Parse percent (or filled-bar widths) to prove monotonic advance.
    let pct = |text: &str| -> i32 {
        for line in text.lines() {
            if let Some(idx) = line.find('%') {
                let prefix = &line[..idx];
                let digits: String = prefix
                    .chars()
                    .rev()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                if let Ok(value) = digits.parse::<i32>() {
                    return value;
                }
            }
            let bars = line.chars().filter(|c| *c == '█').count() as i32;
            if bars > 0 {
                return bars;
            }
        }
        panic!("progress metric present: {text}");
    };
    let p0 = pct(&t0);
    let p1 = pct(&t1);
    let p2 = pct(&t2);
    assert!(p0 < p1 && p1 < p2, "progress must advance: {p0} < {p1} < {p2}");

    let elapsed = |text: &str| -> String {
        text.lines()
            .find(|l| l.contains("Elapsed") || l.contains("0:"))
            .map(|l| l.trim().to_owned())
            .expect("elapsed line")
    };
    assert_ne!(
        elapsed(&t0),
        elapsed(&t2),
        "elapsed time must advance across ticks"
    );
    assert_eq!(progress_owner_count(&mid), 1);
    assert_eq!(progress_owner_count(&late), 1);

    // Mid tick must still look live; sibling Ready may remain but owner is in progress.
    assert!(
        t1.contains("in progress") || t1.contains("In progress") || t1.contains("Working"),
        "middle tick must still look live, not complete: {t1}"
    );
    assert!(
        t1.contains('%') || t1.contains('█'),
        "middle tick still shows a live progress metric: {t1}"
    );

    let primaries = primary_labels(&early);
    assert_eq!(primaries.len(), 1, "one primary while progress is live: {primaries:?}");
}

#[test]
fn mcp_progress_operations_are_named_and_cancelable() {
    for (fixture, operation, cancel_hint) in [
        (
            "views.mcp.progress-setup.tick-1",
            "Setup",
            "provisional",
        ),
        (
            "views.mcp.progress-auth.tick-1",
            "Authentication",
            "stop",
        ),
        (
            "views.mcp.progress-reconnect.tick-1",
            "Reconnect",
            "restore",
        ),
        (
            "views.mcp.progress-invocation.tick-1",
            "Invocation",
            "invocation",
        ),
    ] {
        let audit = audit_screen("mcp", fixture).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        audit
            .validate()
            .unwrap_or_else(|e| panic!("{fixture} audit: {e}"));
        let text = audit.render_txt();
        assert_canvas_geometry(&text);
        assert_no_forbidden_default(&text);
        assert!(
            text.contains(operation),
            "{fixture} must name operation {operation}: {text}"
        );
        assert!(
            text.to_ascii_lowercase().contains(cancel_hint)
                || text.contains("Cancel"),
            "{fixture} must disclose cancel consequence involving {cancel_hint}"
        );
        assert_eq!(
            progress_owner_count(&audit),
            1,
            "{fixture} has one progress owner"
        );
    }
}

// ── VAL-V3-F4-036 / 056: Terminal cleanup and operation cancel ──────────

#[test]
fn mcp_progress_success_collapses_and_removes_live_state() {
    let ready = audit_screen("mcp", "views.mcp.progress-success").expect("progress success");
    ready.validate().expect("success audit");
    let text = ready.render_txt();
    assert_no_forbidden_default(&text);
    assert!(text.contains("Issue tracker"));
    assert!(text.contains("Ready"));
    assert!(!text.contains("In progress") && !text.contains("Working"));
    assert!(!text.contains("█"), "no live progress bar after success");
    assert_eq!(progress_owner_count(&ready), 0);

    let replay = replay_case("mcp-progress-success", 1).expect("success transition");
    assert_eq!(replay.before.selection, "issue-tracker");
    assert_eq!(replay.after.selection, "issue-tracker");
    assert!(
        replay.after.focus.contains("ready")
            || replay.side_effects[0].status.contains("ready")
            || replay.attempts[0].outcome.contains("ready"),
        "success lands ready"
    );
    assert!(
        !replay.after.focus.contains("progress") && !replay.after.focus.contains("running"),
        "live progress focus must clear"
    );
}

#[test]
fn mcp_progress_failure_becomes_attention_retry_row() {
    let failed = audit_screen("mcp", "views.mcp.progress-failed").expect("progress failed");
    failed.validate().expect("failed audit");
    let text = failed.render_txt();
    assert_no_forbidden_default(&text);
    assert!(text.contains("Issue tracker"));
    assert!(text.contains("Failed"));
    assert!(text.contains("Retry"));
    assert!(!text.contains("█") || text.contains("Failed"));
    assert_eq!(progress_owner_count(&failed), 0);

    let replay = replay_case("mcp-progress-failed", 1).expect("failed transition");
    assert_eq!(replay.after.selection, "issue-tracker");
    assert!(
        replay.side_effects[0].status.contains("failed")
            || replay.attempts[0].outcome.contains("failed")
            || replay.after.focus.contains("failed"),
        "failure retains row-scoped failure"
    );
}

#[test]
fn mcp_progress_cancel_rolls_back_by_operation() {
    let setup = replay_case("mcp-progress-cancel-setup", 1).expect("cancel setup");
    assert_eq!(setup.actions[0].action_id, "action.mcp.cancel-setup");
    assert_eq!(setup.after.selection, "browser-automation");
    assert!(
        setup.side_effects[0].status.contains("rolled-back")
            || setup.attempts[0].outcome.contains("rolled-back")
            || setup.side_effects[0].status.contains("setup-rolled-back"),
        "setup cancel rolls provisional setup back: {:?}",
        setup.side_effects[0]
    );

    let auth = replay_case("mcp-progress-cancel-auth", 1).expect("cancel auth");
    assert_eq!(auth.actions[0].action_id, "action.mcp.cancel-auth");
    assert_eq!(auth.after.selection, "project-docs");
    assert!(
        auth.side_effects[0].status.contains("auth-stopped")
            || auth.attempts[0].outcome.contains("auth-stopped")
            || auth.after.focus.contains("auth"),
        "auth cancel stops authentication: {:?}",
        auth.side_effects[0]
    );

    let reconnect = replay_case("mcp-progress-cancel-reconnect", 1).expect("cancel reconnect");
    assert_eq!(
        reconnect.actions[0].action_id,
        "action.mcp.cancel-reconnect"
    );
    assert_eq!(reconnect.after.selection, "issue-tracker");
    assert!(
        reconnect.side_effects[0].status.contains("restored")
            || reconnect.attempts[0].outcome.contains("restored")
            || reconnect.side_effects[0].status.contains("pre-reconnect"),
        "reconnect cancel restores prior state: {:?}",
        reconnect.side_effects[0]
    );

    let invocation = replay_case("mcp-progress-cancel-invocation", 1).expect("cancel invocation");
    assert_eq!(
        invocation.actions[0].action_id,
        "action.mcp.cancel-invocation"
    );
    assert_eq!(invocation.after.selection, "issue-tracker");
    assert!(
        invocation.side_effects[0].status.contains("invocation-cancelled")
            || invocation.attempts[0]
                .outcome
                .contains("invocation-cancelled")
            || invocation.side_effects[0].status.contains("connection-kept"),
        "invocation cancel keeps connection, cancels only the call: {:?}",
        invocation.side_effects[0]
    );
    // Connection preserved while call stops.
    assert!(
        invocation.after.focus.contains("ready")
            || invocation.side_effects[0].status.contains("connection-kept")
            || invocation.side_effects[0]
                .kind
                .to_ascii_lowercase()
                .contains("mcp"),
        "invocation cancel must keep the connection usable"
    );

    let cancelled = audit_screen("mcp", "views.mcp.progress-cancelled")
        .expect("cancelled terminal render");
    let text = cancelled.render_txt();
    assert!(!text.contains("█"), "no stale progress bar after cancel");
    assert_eq!(progress_owner_count(&cancelled), 0);
    assert!(
        text.contains("Ready") || text.contains("Needs setup") || text.contains("Sign in needed"),
        "cancel returns to a usable prior state"
    );
}

// ── VAL-V3-F4-037: Focused capability detail ────────────────────────────

#[test]
fn mcp_focused_detail_is_capability_led_without_raw_tools() {
    let detail = audit_screen("mcp", "views.mcp.detail-docs").expect("focused detail");
    detail.validate().expect("detail audit");
    let text = detail.render_txt();
    assert_canvas_geometry(&text);
    assert_no_forbidden_default(&text);
    assert_no_secrets(&text);

    // Heading is the sanitized full capability name.
    assert!(
        text.contains("Project docs"),
        "heading uses capability name: {text}"
    );
    // Available user actions and recent readable activity first.
    assert!(
        text.contains("What you can do") || text.contains("Available actions"),
        "explains available actions first"
    );
    assert!(
        text.contains("Recent activity") || text.contains("Recent"),
        "shows recent readable activity"
    );
    // May list human-purpose tools/resources/prompts.
    assert!(
        text.contains("Search") || text.contains("Open document") || text.contains("document"),
        "lists readable tools by purpose"
    );
    // No raw protocol IDs / schemas.
    assert!(!text.contains("docs_search_query"));
    assert!(!text.contains("resource://"));
    assert!(!text.contains("prompt_id"));
    assert!(!text.contains('{') && !text.contains('}'));

    let primaries = primary_labels(&detail);
    assert_eq!(primaries.len(), 1, "one primary in detail: {primaries:?}");
    assert!(
        primaries.iter().any(|p| p.contains("Back") || p.contains("Use")),
        "primary is Back or Use: {primaries:?}"
    );

    let open = replay_case("mcp-details-open", 1).expect("open details");
    assert_eq!(open.before.selection, "project-docs");
    assert_eq!(open.after.selection, "project-docs");
    assert!(
        open.after.focus.contains("detail") || open.after.focus.contains("focused"),
        "opens focused detail for selected row"
    );

    let back = replay_case("mcp-details-back", 1).expect("detail back");
    assert_eq!(back.before.selection, "project-docs");
    assert_eq!(back.after.selection, "project-docs");
    assert!(
        !back.after.focus.contains("detail") || back.after.focus.contains("list"),
        "Back restores the selected list row"
    );
    for field in ["draft", "transcript", "session_id", "scroll_anchor"] {
        let hashes = back
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved on Back"));
        assert_eq!(hashes.before, hashes.after, "{field} restored");
    }
}

// ── VAL-V3-F4-038 / 039: Diagnostics closed vs full technical record ────

#[test]
fn mcp_raw_tools_require_explicit_diagnostics_disclosure() {
    let closed = audit_screen("mcp", "views.mcp.detail-docs").expect("closed detail");
    let closed_txt = closed.render_txt();
    assert_no_forbidden_default(&closed_txt);
    assert!(
        closed_txt.contains("diagnostics")
            || closed_txt.contains("Diagnostics")
            || closed_txt.contains("[d]"),
        "diagnostics remains a disclosure affordance"
    );

    let open = audit_screen("mcp", "views.mcp.diagnostics-docs").expect("diagnostics open");
    open.validate().expect("diagnostics audit");
    let open_txt = open.render_txt();
    assert_canvas_geometry(&open_txt);
    assert_no_secrets(&open_txt);

    // Technical record only when open.
    assert!(
        open_txt.contains("Transport") || open_txt.contains("transport"),
        "open diagnostics shows Transport"
    );
    assert!(
        open_txt.contains("Endpoint") || open_txt.contains("endpoint"),
        "open diagnostics shows Endpoint"
    );
    assert!(
        open_txt.contains("Timeout") || open_txt.contains("timeout"),
        "open diagnostics shows Timeout"
    );
    assert!(
        open_txt.contains("Config source") || open_txt.contains("config source"),
        "open diagnostics shows config source"
    );
    assert!(
        open_txt.contains("docs_search_query")
            || open_txt.contains("raw tools")
            || open_txt.contains("Raw tools"),
        "open diagnostics may list raw tool identifiers"
    );

    let open_case = replay_case("mcp-diagnostics-open", 1).expect("open diagnostics");
    assert_eq!(open_case.before.selection, "project-docs");
    assert_eq!(open_case.after.selection, "project-docs");
    assert!(
        open_case.after.focus.contains("diagnostics")
            || open_case.side_effects[0].status.contains("diagnostics-open"),
        "opens diagnostics on the selected capability"
    );

    let close_case = replay_case("mcp-diagnostics-close", 1).expect("close diagnostics");
    assert_eq!(close_case.after.selection, "project-docs");
    assert!(
        close_case.after.focus.contains("detail")
            || close_case.side_effects[0].status.contains("detail")
            || !close_case.after.focus.contains("diagnostics-open"),
        "closing Diagnostics restores focused detail"
    );
}

#[test]
fn mcp_advanced_diagnostics_show_full_redacted_record() {
    let open = audit_screen("mcp", "views.mcp.diagnostics-docs").expect("full diagnostics");
    let text = open.render_txt();
    assert_no_secrets(&text);

    for field in [
        "Transport",
        "Endpoint",
        "Timeout",
        "Config source",
        "Raw tools",
        "Resources",
        "Prompts",
        "Schema",
        "Scopes",
    ] {
        assert!(
            text.contains(field),
            "diagnostics must label field {field}: {text}"
        );
    }
    // Unavailable fields say Not available rather than inventing values.
    assert!(
        text.contains("Not available") || text.contains("redacted") || text.contains("sanitized"),
        "unavailable or sensitive values are disclosed as such"
    );
    // Hostile/sensitive scopes are sanitized.
    let hostile = audit_screen("mcp", "views.mcp.diagnostics-hostile")
        .expect("hostile diagnostics");
    let hostile_txt = hostile.render_txt();
    assert_no_secrets(&hostile_txt);
    assert!(
        hostile_txt.contains("sanitized")
            || hostile_txt.contains("redacted")
            || hostile_txt.contains("Not available")
            || hostile_txt.contains("untrusted"),
        "hostile-scope values are sanitized with provenance"
    );
    assert!(!hostile_txt.contains(&format!("sk-{}-{}-{}", "live", "secret", "token")));
    assert!(!hostile_txt.contains(&format!("{} {}", "Bearer", "supersecret")));
}

// ── VAL-V3-F4-040: Diagnostics keep recovery reachable ──────────────────

#[test]
fn mcp_diagnostics_preserve_direct_recovery_action() {
    let failed = audit_screen("mcp", "views.mcp.diagnostics-failed")
        .expect("failed diagnostics");
    failed.validate().expect("failed diagnostics audit");
    let text = failed.render_txt();
    assert_canvas_geometry(&text);
    assert_no_secrets(&text);

    assert!(text.contains("Issue tracker") || text.contains("Failed"));
    assert!(
        text.contains("Retry") || text.contains("Retry"),
        "recovery Retry remains immediately reachable"
    );
    assert!(
        text.contains("Could not") || text.contains("unavailable") || text.contains("Failed"),
        "readable cause remains visible"
    );
    // Technical evidence is present but secondary to recovery.
    assert!(
        text.contains("Transport") || text.contains("Endpoint") || text.contains("diagnostics"),
        "technical evidence is available under diagnostics"
    );
    let primaries = primary_labels(&failed);
    assert_eq!(primaries.len(), 1, "one primary under failed diagnostics: {primaries:?}");
    assert!(
        primaries.iter().any(|p| p.contains("Retry")),
        "primary stays Retry: {primaries:?}"
    );

    let auth = audit_screen("mcp", "views.mcp.diagnostics-auth").expect("auth diagnostics");
    let auth_txt = auth.render_txt();
    assert!(auth_txt.contains("Sign in"));
    let auth_primaries = primary_labels(&auth);
    assert_eq!(auth_primaries.len(), 1);
    assert!(
        auth_primaries.iter().any(|p| p.contains("Sign in")),
        "primary stays Sign in under diagnostics: {auth_primaries:?}"
    );

    let open_failed = replay_case("mcp-diagnostics-failed-open", 1).expect("diagnostics open failed");
    assert_eq!(open_failed.before.selection, "issue-tracker");
    assert_eq!(open_failed.after.selection, "issue-tracker");
    // Primary action identity retained across open.
    assert!(
        open_failed.actions[0].action_id == "action.mcp.diagnostics"
            || open_failed.after.focus.contains("diagnostics"),
        "opens diagnostics without dropping recovery row identity"
    );

    let close_failed =
        replay_case("mcp-diagnostics-failed-close", 1).expect("diagnostics close failed");
    assert_eq!(close_failed.after.selection, "issue-tracker");
    assert!(
        !close_failed.after.focus.contains("diagnostics-open")
            || close_failed.after.focus.contains("failed"),
        "collapse returns to the same failed row"
    );
}
