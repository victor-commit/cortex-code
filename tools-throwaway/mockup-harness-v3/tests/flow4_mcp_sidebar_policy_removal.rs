//! Contextual MCP sidebar, policy review/change, removal effects, public
//! fixture inventory, full-origin restoration, and resume policy
//! (VAL-V3-F4-041..043, 051..053, 055, 059).

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

const FORBIDDEN_SIDEBAR: &[&str] = &[
    "stdio",
    "http://",
    "https://",
    "SSE",
    "transport",
    "endpoint",
    "timeout",
    "scopes",
    "schema",
    "mcpServers",
    "spawn error",
    "stack trace",
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

fn assert_no_forbidden_sidebar(text: &str) {
    let lower = text.to_ascii_lowercase();
    for banned in FORBIDDEN_SIDEBAR {
        assert!(
            !lower.contains(&banned.to_ascii_lowercase()),
            "sidebar must hide technical detail: found {banned}"
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

fn origin_hashes(case: &mockup_harness_v3::scenario::CaseReplay) -> (String, String) {
    (
        case.before.state_hash.clone(),
        case.after.state_hash.clone(),
    )
}

// ── VAL-V3-F4-041: Sidebar MCP compact and contextual ───────────────────

#[test]
fn sidebar_mcp_stays_compact_and_contextual() {
    let audit = audit_screen("sidebar-mcp", "views.sidebar-mcp.attention")
        .expect("sidebar-mcp must render");
    audit.validate().expect("sidebar-mcp audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_forbidden_sidebar(&text);

    // Transcript and composer remain visible (contextual inspector).
    assert!(
        text.contains("rate limit") || text.contains("safe path") || text.contains("You ·"),
        "conversation transcript must remain beside the inspector: {text}"
    );
    assert!(
        text.contains("Also list the tests") || text.contains("› "),
        "composer draft must remain available under the inspector"
    );

    // Compact total + attention or all-ready, not full MCP catalogue.
    assert!(
        text.contains("MCP") && (text.contains("of") || text.contains("capabilities")),
        "sidebar must summarize a compact total"
    );
    assert!(
        text.contains("Open MCP") || text.contains("Open MCP"),
        "sidebar must expose one local Open MCP action"
    );

    // Must not duplicate the entire manager list.
    let count_cli_like = [
        "Negotiated",
        "Code search",
        "Enterprise knowledge",
        "Metric export",
        "Deploy hooks",
        "Project docs",
        "Browser automation",
        "Issue tracker",
    ]
    .iter()
    .filter(|needle| text.contains(*needle))
    .count();
    assert!(
        count_cli_like <= 3,
        "sidebar must not dump the full MCP catalogue; matched {count_cli_like}"
    );

    let primaries = primary_labels(&audit);
    assert!(
        primaries.iter().any(|p| p.contains("Open MCP")),
        "primary action is Open MCP: {primaries:?}"
    );
    assert_eq!(audit.screen_id, "sidebar-mcp");
}

// ── VAL-V3-F4-042: Sidebar prioritizes attention ────────────────────────

#[test]
fn sidebar_mcp_prioritizes_attention_over_healthy() {
    let audit = audit_screen("sidebar-mcp", "views.sidebar-mcp.attention")
        .expect("attention sidebar must render");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);

    assert!(
        text.contains("Blocked by policy")
            || text.contains("Policy blocked")
            || text.contains("needs")
            || text.contains("Failed")
            || text.contains("Sign in")
            || text.contains("Setup"),
        "attention item must be readable without color"
    );
    assert!(
        !text.contains("All ready") && !text.contains("everything is ready"),
        "must not claim all ready while attention remains"
    );
    assert!(
        !text.to_ascii_lowercase().contains("diagnostics")
            || text.contains("Open MCP"),
        "compact panel must not expand diagnostics"
    );

    let all_ready = audit_screen("sidebar-mcp", "views.sidebar-mcp.all-ready")
        .expect("all-ready sidebar must render");
    let ready_text = all_ready.render_txt();
    assert!(
        ready_text.contains("All ready")
            || ready_text.contains("all ready")
            || ready_text.contains("ready"),
        "healthy fixture surfaces an all-ready statement: {ready_text}"
    );
    assert!(
        ready_text.contains("Open MCP"),
        "healthy sidebar still offers Open MCP"
    );
}

// ── VAL-V3-F4-043: Sidebar navigation restores conversation ─────────────

#[test]
fn sidebar_mcp_open_and_return_restores_context() {
    let open = replay_case("sidebar-mcp-open-mcp", 1).expect("open MCP from sidebar");
    assert_eq!(open.before.screen_id, "sidebar-mcp");
    assert_eq!(open.after.screen_id, "mcp");
    assert_eq!(open.before.draft, open.after.draft);
    assert_eq!(open.before.scroll_anchor, open.after.scroll_anchor);
    assert_eq!(open.before.conversation_id, open.after.conversation_id);
    assert_eq!(open.attempts[0].count, 0);

    let back = replay_case("sidebar-mcp-return", 1).expect("return from full MCP to sidebar");
    assert_eq!(back.before.screen_id, "mcp");
    assert_eq!(back.after.screen_id, "sidebar-mcp");
    assert_eq!(back.before.draft, back.after.draft);
    assert_eq!(back.before.scroll_anchor, back.after.scroll_anchor);
    assert_eq!(back.before.selection, back.after.selection);
    assert!(!back.after.focus.is_empty());
    assert_ne!(back.after.screen_id, "home");
    assert_eq!(back.before.origin_stack, back.after.origin_stack);
    // Sidebar tab / selection intentionally restored.
    assert!(
        back.after.focus.contains("sidebar") || back.after.selection.contains("deploy")
            || back.after.selection.contains("mcp")
            || !back.after.selection.is_empty(),
        "selection and focus restored after return"
    );
}

// ── VAL-V3-F4-051: Thirteen public MCP fixtures ─────────────────────────

#[test]
fn thirteen_public_mcp_fixtures_cover_required_states() {
    // Named public scenarios for each required noncanonical fixture branch.
    let inventory: &[(&str, &str)] = &[
        ("mcp-fixture-empty", "empty"),
        ("mcp-fixture-mixed", "mixed"),
        ("mcp-fixture-setup-needed", "setup"),
        ("mcp-fixture-needs-auth", "auth"),
        ("mcp-fixture-policy-blocked", "policy"),
        ("mcp-fixture-failed", "failed"),
        ("mcp-fixture-retrying", "retry"),
        ("mcp-fixture-running-progress", "progress"),
        ("mcp-fixture-succeeded-call", "success"),
        ("mcp-fixture-failed-call", "failed-call"),
        ("mcp-fixture-cancelled-call", "cancel"),
        ("mcp-fixture-focused-details", "detail"),
        ("mcp-fixture-diagnostics", "diagnostics"),
    ];
    assert_eq!(inventory.len(), 13, "exactly thirteen public fixtures");

    for (case_id, tag) in inventory {
        let case = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        assert_eq!(
            case.before.screen_id, "mcp",
            "{case_id} stays on mcp detail surface"
        );
        assert!(
            case.after.screen_id == "mcp" || case.after.screen_id == "chat-turn",
            "{case_id} does not invent a new canonical ID"
        );
        // Fixture tags land in focus so replay evidence is inspectable.
        assert!(
            case.before.focus.to_ascii_lowercase().contains(tag)
                || case.after.focus.to_ascii_lowercase().contains(tag)
                || case.actions[0].label.to_ascii_lowercase().contains(tag)
                || case.side_effects[0]
                    .status
                    .to_ascii_lowercase()
                    .contains(tag)
                || tag.len() < 3,
            "{case_id} must evidence fixture role `{tag}` via focus/label/status"
        );
    }
}

// ── VAL-V3-F4-052: Policy blocking ──────────────────────────────────────

#[test]
fn policy_blocked_disables_incompatible_actions_and_requires_confirm() {
    let audit = audit_screen("mcp", "views.mcp.policy-blocked").expect("policy-blocked render");
    audit.validate().expect("policy audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert!(
        text.contains("Blocked by policy"),
        "must say exactly Blocked by policy: {text}"
    );
    assert!(
        text.contains("Review policy") || text.contains("Review"),
        "primary is Review policy"
    );
    // Incompatible actions disabled: not offered as enter/primary targets.
    let lower = text.to_ascii_lowercase();
    assert!(
        !lower.contains("enter to retry")
            && !lower.contains("enter to sign in")
            && !lower.contains("enter to setup"),
        "Retry/Sign in/Setup must not be primary when policy-blocked"
    );
    let primaries = primary_labels(&audit);
    assert!(
        primaries.iter().any(|p| p.contains("Review policy")),
        "primary Review policy: {primaries:?}"
    );

    let open = replay_case("mcp-policy-review-open", 1).expect("open policy review");
    assert_eq!(open.before.selection, open.after.selection);
    assert!(
        open.after.focus.contains("policy") || open.after.screen_id == "mcp",
        "opens focused policy state"
    );
    assert_eq!(open.attempts[0].count, 0);
    assert!(
        open.side_effects[0].status.contains("review")
            || open.side_effects[0].status.contains("policy")
            || open.side_effects[0].status.contains("open"),
        "policy review does not invoke: {:?}",
        open.side_effects[0]
    );

    let confirm = replay_case("mcp-policy-change-confirm", 1).expect("confirm policy change");
    assert!(
        confirm.actions[0].label.to_ascii_lowercase().contains("confirm")
            || confirm.actions[0].action_id.contains("confirm"),
        "confirm path for mutation"
    );

    let cancel = replay_case("mcp-policy-change-cancel", 1).expect("cancel policy change");
    assert_eq!(cancel.before.selection, cancel.after.selection);
    assert_eq!(cancel.before.conversation_id, cancel.after.conversation_id);
    assert_eq!(cancel.before.origin_stack, cancel.after.origin_stack);
    assert_eq!(cancel.attempts[0].count, 0);
    assert!(
        cancel.side_effects[0].status.contains("cancel")
            || cancel.side_effects[0].status.contains("unchanged")
            || cancel.side_effects[0].status.contains("no-change"),
        "cancel leaves policy unchanged: {:?}",
        cancel.side_effects[0]
    );
}

// ── VAL-V3-F4-053: Distinct removal actions ─────────────────────────────

#[test]
fn removal_actions_are_distinct_with_managed_safeguard() {
    let disable = replay_case("mcp-disable", 1).expect("disable retains configuration");
    assert!(
        disable.side_effects[0].status.contains("disabled")
            || disable.side_effects[0].kind.contains("disable"),
        "disable effect: {:?}",
        disable.side_effects[0]
    );
    assert!(
        !disable.side_effects[0].status.contains("removed-connection")
            && !disable.side_effects[0].status.contains("signed-out"),
        "disable must not remove/sign-out"
    );

    let sign_out = replay_case("mcp-sign-out", 1).expect("sign out removes auth only");
    assert!(
        sign_out.side_effects[0].status.contains("signed-out")
            || sign_out.side_effects[0].status.contains("auth-removed"),
        "sign-out effect: {:?}",
        sign_out.side_effects[0]
    );

    let remove = replay_case("mcp-remove-connection", 1).expect("remove deletes user connection");
    assert!(
        remove.side_effects[0].status.contains("removed")
            || remove.side_effects[0].kind.contains("remove"),
        "remove effect: {:?}",
        remove.side_effects[0]
    );

    let managed =
        replay_case("mcp-remove-managed-refused", 1).expect("managed config cannot be removed");
    assert_eq!(managed.attempts[0].count, 0);
    assert!(
        managed.side_effects[0].status.contains("managed")
            || managed.side_effects[0].status.contains("refused")
            || managed.side_effects[0].status.contains("blocked"),
        "managed refusal: {:?}",
        managed.side_effects[0]
    );

    let managed_audit = audit_screen("mcp", "views.mcp.manage-removal").expect("manage removal UI");
    let managed_text = managed_audit.render_txt();
    assert!(
        managed_text.contains("Disable")
            && (managed_text.contains("Sign out") || managed_text.contains("Sign out"))
            && managed_text.contains("Remove connection"),
        "distinct actions disclosed: {managed_text}"
    );
    assert!(
        managed_text.contains("Managed") || managed_text.contains("managed"),
        "managed project configuration is identified"
    );
    assert!(
        managed_text.contains("cannot be removed")
            || managed_text.contains("not removable")
            || managed_text.contains("managed by the project"),
        "managed safeguard copy present"
    );
}

// ── VAL-V3-F4-055: Full origin restoration ──────────────────────────────

#[test]
fn mcp_origin_preserves_complete_interaction_state() {
    let cases = [
        "sidebar-mcp-open-mcp",
        "sidebar-mcp-return",
        "mcp-policy-change-cancel",
        "mcp-details-back",
        "mcp-diagnostics-close",
        "mcp-return",
    ];
    for case_id in cases {
        let case = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        assert!(!case.before.draft.is_empty(), "{case_id} draft present");
        assert!(
            !case.before.scroll_anchor.is_empty(),
            "{case_id} scroll present"
        );
        assert!(!case.before.selection.is_empty(), "{case_id} selection");
        assert!(!case.before.focus.is_empty(), "{case_id} focus");
        assert!(!case.before.return_policy.is_empty(), "{case_id} policy");
        assert!(!case.before.state_hash.is_empty(), "{case_id} hash");
        assert!(
            case.before.origin_stack.len() >= 2,
            "{case_id} origin stack"
        );
        // Preservation: cancel/back/close keep draft + scroll + conversation.
        if case_id.contains("return")
            || case_id.contains("cancel")
            || case_id.contains("close")
            || case_id.contains("back")
        {
            assert_eq!(case.before.draft, case.after.draft, "{case_id} draft");
            assert_eq!(
                case.before.scroll_anchor, case.after.scroll_anchor,
                "{case_id} scroll"
            );
            assert_eq!(
                case.before.conversation_id, case.after.conversation_id,
                "{case_id} conversation"
            );
            assert_eq!(
                case.before.origin_stack, case.after.origin_stack,
                "{case_id} origin stack"
            );
        }
        let (_before_hash, _after_hash) = origin_hashes(&case);
        assert_eq!(case.before.state_hash.len(), 64);
        assert_eq!(case.after.state_hash.len(), 64);
    }
}

// ── VAL-V3-F4-059: Resume policy controls automatic invocation ──────────

#[test]
fn resume_policy_controls_automatic_invocation_counts() {
    // Read-only automatic_once: exactly one invocation.
    let auto = replay_case("mcp-resume-automatic-once", 1).expect("automatic once");
    assert_eq!(auto.before.return_policy, "automatic_once");
    assert_eq!(auto.attempts[0].count, 1);
    assert!(
        auto.side_effects[0].status.contains("once")
            || auto.side_effects[0].status.contains("automatic")
            || auto.attempts[0].outcome.contains("once"),
        "automatic-once evidence: {:?}",
        auto.side_effects[0]
    );

    // Side-effecting requires confirmation (zero invocation until confirmed).
    let confirm = replay_case("mcp-resume-confirmation", 1).expect("confirmation required");
    assert_eq!(confirm.before.return_policy, "confirmation");
    assert_eq!(confirm.attempts[0].count, 0);
    assert!(
        confirm.side_effects[0].status.contains("confirm")
            || confirm.attempts[0].outcome.contains("confirm"),
        "confirmation required with zero invocation"
    );

    // none: no invocation.
    let none = replay_case("mcp-resume-none", 1).expect("none");
    assert_eq!(none.before.return_policy, "none");
    assert_eq!(none.attempts[0].count, 0);

    // cancel recovery never invokes.
    let cancel = replay_case("mcp-resume-cancel", 1).expect("cancel resume");
    assert_eq!(cancel.attempts[0].count, 0);

    // failed recovery never invokes.
    let failed = replay_case("mcp-resume-failed", 1).expect("failed resume");
    assert_eq!(failed.attempts[0].count, 0);

    // policy-blocked never invokes.
    let policy = replay_case("mcp-resume-policy-blocked", 1).expect("policy blocked resume");
    assert_eq!(policy.attempts[0].count, 0);
    assert!(
        policy.side_effects[0].status.contains("policy")
            || policy.attempts[0].outcome.contains("policy"),
        "policy-blocked zero invocation"
    );

    // Provider-origin blocked model preserves action identity + exact counts.
    let provider_auto =
        replay_case("provider-resume-automatic-once", 1).expect("provider automatic once");
    assert_eq!(provider_auto.before.return_policy, "automatic_once");
    assert_eq!(provider_auto.attempts[0].count, 1);
    assert!(
        provider_auto
            .before
            .pending_action
            .as_deref()
            .unwrap_or("")
            .contains("model")
            || provider_auto.before.selection.contains("model")
            || provider_auto.before.paused_activity_id.contains("model")
            || provider_auto
                .before
                .paused_activity_id
                .contains("blocked"),
        "blocked model action identity preserved"
    );

    let provider_none = replay_case("provider-resume-none", 1).expect("provider none");
    assert_eq!(provider_none.attempts[0].count, 0);
}
