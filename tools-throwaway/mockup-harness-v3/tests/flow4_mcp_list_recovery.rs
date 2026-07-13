//! Capability-led MCP listing and row recovery
//! (VAL-V3-F4-027..034).
//!
//! Covers empty and mixed MCP inventories, exception-first ordering, healthy
//! collapse, non-color states, guided setup with cancel rollback, sign-in,
//! and row-scoped retry that leaves sibling capabilities unchanged.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const FORBIDDEN_TRANSPORT: &[&str] = &[
    "stdio",
    "http://",
    "https://",
    "SSE",
    "transport",
    "endpoint",
    "timeout",
    "scopes",
    "schema",
    "config path",
    "mcpServers",
    "spawn error",
    "stack trace",
    "401",
    "OpenRouter",
    "Anthropic",
    "oauth2",
];

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

fn assert_no_forbidden_transport(text: &str) {
    let lower = text.to_ascii_lowercase();
    for banned in FORBIDDEN_TRANSPORT {
        assert!(
            !lower.contains(&banned.to_ascii_lowercase()),
            "default MCP must hide technical detail: found {banned}"
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

// ── VAL-V3-F4-027: Empty MCP state leads with setup ─────────────────────

#[test]
fn empty_mcp_leads_with_setup_and_stays_separate_from_providers() {
    let audit = audit_screen("mcp", "views.mcp.empty").expect("empty mcp must render");
    audit.validate().expect("empty mcp audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_forbidden_transport(&text);

    assert!(
        text.contains("Connect external tools"),
        "empty state must lead with Connect external tools: {text}"
    );
    assert!(
        text.contains("MCP")
            || text.contains("external tools")
            || text.contains("capabilities outside"),
        "must explain what MCP enables"
    );
    assert!(
        text.contains("Setup connection") || text.contains("[enter] Setup"),
        "must offer one Setup connection action"
    );
    assert!(
        !text.to_ascii_lowercase().contains("provider login")
            && !text.contains("Choose a provider")
            && !text.contains("OpenAI")
            && !text.contains("GPT-5.2"),
        "empty MCP must never route into provider login"
    );
    let primaries = primary_labels(&audit);
    assert_eq!(
        primaries.len(),
        1,
        "empty MCP has exactly one primary: {primaries:?}"
    );
    assert!(
        primaries.iter().any(|p| p.contains("Setup")),
        "primary must be Setup: {primaries:?}"
    );
    assert!(audit.transitions.iter().any(|t| {
        t.action_id == "action.mcp.setup-connection"
            && t.from_screen_id == "mcp"
            && t.kind == "direct"
    }));
}

// ── VAL-V3-F4-028 / 029 / 030 / 031: Default list shape ─────────────────

#[test]
fn default_mcp_list_is_capability_led_and_hides_transport() {
    let audit = audit_screen("mcp", "gate.provider-and-mcp.mcp").expect("default mcp");
    audit.validate().expect("default mcp audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_forbidden_transport(&text);

    // Each attention or ready row names a human capability.
    assert!(text.contains("Browser automation"));
    assert!(text.contains("Project docs"));
    assert!(text.contains("Code search"));

    // Readable states distinct from actions.
    assert!(text.contains("Needs setup") || text.contains("Setup needed"));
    assert!(text.contains("Sign in needed") || text.contains("Needs sign-in"));
    assert!(text.contains("Ready"));

    // At most one primary action region for the focused row.
    let primaries = primary_labels(&audit);
    assert_eq!(primaries.len(), 1, "one primary: {primaries:?}");

    // Diagnostics closed: no default transport/tool dump.
    assert!(
        text.contains("diagnostics") || text.contains("Diagnostics") || text.contains("[d]"),
        "diagnostics remains a closed disclosure affordance"
    );
    assert!(!text.contains("browser_open_page"));
    assert!(!text.contains("docs_search_query"));
}

#[test]
fn healthy_mcp_rows_collapse_to_one_line() {
    let audit = audit_screen("mcp", "gate.provider-and-mcp.mcp").expect("default mcp");
    let text = audit.render_txt();
    // Summary + one-line healthy row; no tool inventory under Ready.
    assert!(
        text.contains("ready connection collapsed") || text.contains("Ready"),
        "healthy summary or row must appear"
    );
    let ready_lines: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("Code search") && line.contains("Ready"))
        .collect();
    assert_eq!(
        ready_lines.len(),
        1,
        "healthy capability uses one compact line: {ready_lines:?}"
    );
    // No expanded tool/metadata under the healthy row by default.
    assert!(!text.contains("enabled tools"));
    assert!(!text.contains("config source"));
}

#[test]
fn mixed_mcp_sorts_attention_before_ready_deterministically() {
    let audit = audit_screen("mcp", "views.mcp.mixed").expect("mixed mcp must render");
    audit.validate().expect("mixed mcp audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_forbidden_transport(&text);

    // Priority: policy blocked → needs sign-in → setup needed → failed
    // → retrying/running → disabled → ready.
    let positions = |needle: &str| text.find(needle).expect(needle);
    let policy = positions("Blocked by policy");
    let signin = positions("Sign in needed");
    let setup = text
        .find("Setup needed")
        .or_else(|| text.find("Needs setup"))
        .expect("setup state");
    let failed = positions("Failed");
    let ready = positions("Ready");

    assert!(
        policy < signin && signin < setup && setup < failed && failed < ready,
        "attention order must be exception-first: policy={policy} signin={signin} setup={setup} failed={failed} ready={ready}"
    );

    // Non-color markers beside states.
    assert!(
        text.contains('●') || text.contains('○') || text.contains('✗'),
        "state markers must be non-color glyphs"
    );

    // Stable identities and reconciled summary counts.
    assert!(text.contains("Project docs") || text.contains("Workspace docs"));
    assert!(text.contains("Browser automation"));
    assert!(text.contains("Issue tracker") || text.contains("Deploy hooks"));
    assert!(text.contains("Code search"));

    // Re-render for determinism.
    let again = audit_screen("mcp", "views.mcp.mixed")
        .expect("repeated mixed render")
        .render_txt();
    assert_eq!(text, again, "mixed MCP render must be bit-stable");
}

#[test]
fn mcp_states_are_plain_english_and_distinct_from_actions() {
    let audit = audit_screen("mcp", "views.mcp.mixed").expect("mixed mcp");
    let text = audit.render_txt();

    // State phrase + action label must not collapse ("Retry" is action not state).
    assert!(
        text.contains("Failed"),
        "failed capability must show state Failed"
    );
    assert!(
        text.contains("Retry"),
        "failed capability must show action Retry"
    );
    // Ensure the state phrase itself is not named only Retry (action labels may say Retry).
    let state_named_retry: Vec<&str> = text
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed.contains(" · Retry")
                || trimmed.ends_with(" · Retry")
                || (trimmed.contains("Retry")
                    && !trimmed.contains('[')
                    && !trimmed.contains("Failed")
                    && !trimmed.contains("Retrying"))
        })
        .collect();
    assert!(
        state_named_retry.is_empty(),
        "state labels must not be named only Retry: {state_named_retry:?}"
    );

    assert!(text.contains("Sign in needed") || text.contains("Needs sign-in"));
    assert!(text.contains("[enter] Sign in") || text.contains("Sign in"));
    assert!(text.contains("Setup needed") || text.contains("Needs setup"));
    assert!(text.contains("Setup") || text.contains("Fix setup"));
}

// ── VAL-V3-F4-032: Guided setup ─────────────────────────────────────────

#[test]
fn mcp_setup_opens_guided_focus_with_connect_and_cancel_rollback() {
    let focused = audit_screen("mcp", "views.mcp.setup-browser").expect("setup step");
    focused.validate().expect("setup audit");
    let text = focused.render_txt();
    assert_canvas_geometry(&text);
    assert_no_forbidden_transport(&text);

    assert!(
        text.contains("Browser automation"),
        "setup focuses named capability"
    );
    assert!(
        text.contains("source") || text.contains("publisher") || text.contains("Browser Tools"),
        "shows source or publisher"
    );
    assert!(
        text.contains("access") || text.contains("scope") || text.contains("Requested"),
        "shows requested access/scope"
    );
    assert!(
        text.contains("Connect") && text.contains("Advanced setup"),
        "Connect primary and Advanced setup secondary"
    );
    assert!(
        text.contains("MCP · conversation capabilities") || text.contains("MCP origin"),
        "retains MCP origin"
    );

    let primaries = primary_labels(&focused);
    assert_eq!(primaries.len(), 1, "one Connect primary: {primaries:?}");
    assert!(primaries.iter().any(|p| p.contains("Connect")));

    let cancel = replay_case("mcp-setup-cancel", 1).expect("setup cancel replay");
    assert_eq!(cancel.before.screen_id, "mcp");
    assert_eq!(cancel.after.screen_id, "mcp");
    assert_eq!(cancel.actions[0].action_id, "action.mcp.setup-cancel");
    assert_eq!(
        cancel.before.selection, "browser-automation",
        "focus stays on the selected capability"
    );
    assert_eq!(
        cancel.after.selection, "browser-automation",
        "cancel restores the same selected row"
    );
    // Inventory unchanged: setup provisional state rolls back.
    assert_eq!(
        cancel.side_effects[0].status, "rolled-back",
        "cancel must not add a connection"
    );
    for field in ["draft", "transcript", "session_id"] {
        let hashes = cancel
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must be preserved"));
        assert_eq!(hashes.before, hashes.after, "{field} must survive setup cancel");
    }
}

// ── VAL-V3-F4-033: Sign-in action ───────────────────────────────────────

#[test]
fn mcp_sign_in_stays_in_mcp_flow_and_resolves_to_ready() {
    let auth = audit_screen("mcp", "views.mcp.auth-docs").expect("auth-needed row");
    auth.validate().expect("auth audit");
    let text = auth.render_txt();
    assert_no_forbidden_transport(&text);
    assert!(text.contains("Project docs") || text.contains("Workspace docs"));
    assert!(text.contains("Sign in needed") || text.contains("Needs sign-in"));
    assert!(text.contains("Sign in"));
    assert!(!text.contains("401"));
    assert!(!text.contains("Choose a provider"));
    assert!(!text.contains("scopes"));

    let replay = replay_case("mcp-sign-in-success", 1).expect("sign-in replay");
    assert_eq!(replay.before.screen_id, "mcp");
    assert_eq!(replay.after.screen_id, "mcp");
    assert_eq!(replay.actions[0].action_id, "action.mcp.sign-in");
    assert_eq!(replay.before.selection, "project-docs");
    assert_eq!(
        replay.after.selection, "project-docs",
        "sign-in is scoped to the authorized capability"
    );
    assert!(
        replay.after.focus.contains("ready")
            || replay.side_effects[0].status.contains("ready"),
        "success updates capability to ready"
    );

    let after = audit_screen("mcp", "views.mcp.auth-docs-ready").expect("post sign-in");
    let after_text = after.render_txt();
    assert!(
        after_text.contains("Project docs") && after_text.contains("Ready"),
        "authorized capability becomes Ready"
    );
    assert!(
        !after_text
            .lines()
            .any(|line| line.contains("Project docs") && line.contains("Sign in needed")),
        "auth-needed state must clear for that capability"
    );
}

// ── VAL-V3-F4-034: Row-scoped retry ─────────────────────────────────────

#[test]
fn mcp_retry_mutates_only_selected_capability() {
    let failed = audit_screen("mcp", "views.mcp.failed-tracker").expect("failed row");
    failed.validate().expect("failed audit");
    let text = failed.render_txt();
    assert_no_forbidden_transport(&text);
    assert!(text.contains("Failed"));
    assert!(text.contains("Retry"));
    assert!(
        text.contains("Issue tracker") || text.contains("Deploy hooks"),
        "names the failed capability"
    );
    // Concise cause, not raw spawn/stack.
    assert!(
        text.contains("could not")
            || text.contains("lost")
            || text.contains("unavailable")
            || text.contains("timed out")
            || text.contains("Could not"),
        "plain-English cause required"
    );
    assert!(!text.contains("spawn error") && !text.contains("stack"));

    let running = replay_case("mcp-retry-running", 1).expect("retry → running");
    assert_eq!(running.before.screen_id, "mcp");
    assert_eq!(running.after.screen_id, "mcp");
    assert_eq!(running.actions[0].action_id, "action.mcp.retry");
    assert_eq!(running.before.selection, "issue-tracker");
    assert_eq!(running.after.selection, "issue-tracker");
    assert!(
        running.after.focus.contains("retry")
            || running.side_effects[0].status.contains("running")
            || running.attempts[0].outcome.contains("retrying"),
        "retry moves the selected row into running/retrying"
    );
    // Sibling inventory identity stays intact.
    assert_eq!(
        running.preserved_field_hashes.get("transcript").map(|h| h.before.clone()),
        running.preserved_field_hashes.get("transcript").map(|h| h.after.clone()),
    );

    let ready = replay_case("mcp-retry-ready", 1).expect("retry → ready terminal");
    assert_eq!(ready.before.screen_id, "mcp");
    assert_eq!(ready.after.screen_id, "mcp");
    assert_eq!(ready.actions[0].action_id, "action.mcp.retry");
    assert_eq!(ready.after.selection, "issue-tracker");
    assert!(
        ready.side_effects[0].status.contains("ready")
            || ready.attempts[0].outcome.contains("ready")
            || ready.after.focus.contains("ready"),
        "successful retry lands Ready"
    );

    let still_failed = replay_case("mcp-retry-failed", 1).expect("retry → new failure");
    assert_eq!(still_failed.after.selection, "issue-tracker");
    assert!(
        still_failed.side_effects[0].status.contains("failed")
            || still_failed.attempts[0].outcome.contains("failed")
            || still_failed.after.focus.contains("failed"),
        "failed retry produces a new plain-English failure"
    );

    // Rendered siblings present and unchanged across ready terminal fixture.
    let terminal = audit_screen("mcp", "views.mcp.retry-ready").expect("retry ready fixture");
    let terminal_text = terminal.render_txt();
    assert!(terminal_text.contains("Issue tracker") || terminal_text.contains("Deploy hooks"));
    assert!(
        terminal_text.contains("Ready"),
        "selected capability becomes Ready after success"
    );
    // Unrelated ready capability still listed.
    assert!(
        terminal_text.contains("Code search") || terminal_text.contains("Browser automation"),
        "sibling capabilities remain"
    );
    assert_no_forbidden_transport(&terminal_text);
}

#[test]
fn mcp_row_actions_never_leak_provider_adjacency() {
    for fixture in [
        "views.mcp.empty",
        "views.mcp.mixed",
        "views.mcp.setup-browser",
        "views.mcp.auth-docs",
        "views.mcp.failed-tracker",
    ] {
        let audit = audit_screen("mcp", fixture).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        let text = audit.render_txt();
        assert!(
            !audit
                .transitions
                .iter()
                .any(|t| t.to_screen_id == "login" || t.to_screen_id == "connect-provider"),
            "{fixture} must not route into provider login"
        );
        assert!(
            !text.contains("Provider connection · blocked model action"),
            "{fixture} must keep MCP origin, not provider origin"
        );
    }
}
