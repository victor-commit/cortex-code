//! Usable negotiation and untrusted MCP label sanitization
//! (VAL-V3-F4-057, VAL-V3-F4-058).
//!
//! Covers Ready only after a usable policy-permitted capability exists,
//! reconciled negotiated/allowed/blocked/unavailable counts, and sanitization
//! of hostile capability/publisher/tool/resource/prompt/error/argument/response
//! labels across list, detail, audit, and diagnostics surfaces.

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};

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

fn parse_summary_counts(text: &str) -> Option<(u32, u32, u32, u32)> {
    // "Negotiated N · Allowed A · Blocked B · Unavailable U"
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("Negotiated") || !trimmed.contains("Allowed") {
            continue;
        }
        let mut negotiated = None;
        let mut allowed = None;
        let mut blocked = None;
        let mut unavailable = None;
        for part in trimmed.split('·') {
            let p = part.trim();
            if let Some(rest) = p.strip_prefix("Negotiated ") {
                negotiated = rest.split_whitespace().next().and_then(|s| s.parse().ok());
            } else if let Some(rest) = p.strip_prefix("Allowed ") {
                allowed = rest.split_whitespace().next().and_then(|s| s.parse().ok());
            } else if let Some(rest) = p.strip_prefix("Blocked ") {
                blocked = rest.split_whitespace().next().and_then(|s| s.parse().ok());
            } else if let Some(rest) = p.strip_prefix("Unavailable ") {
                unavailable = rest.split_whitespace().next().and_then(|s| s.parse().ok());
            }
        }
        if let (Some(n), Some(a), Some(b), Some(u)) = (negotiated, allowed, blocked, unavailable) {
            return Some((n, a, b, u));
        }
    }
    None
}

fn assert_no_control_injection(text: &str) {
    for banned in [
        "\u{0007}",
        "\u{001b}",
        "\u{0000}",
        "\u{202E}",
        "\u{202A}",
        "\u{200B}",
        "\u{200D}",
        "\u{200C}",
        "\u{2066}",
        "\u{FEFF}",
    ] {
        assert!(
            !text.contains(banned),
            "control/bidi/zero-width sequence leaked into MCP surface"
        );
    }
    // Literal escape sequences that would flip terminal styles must not appear.
    assert!(
        !text.contains("[31m") && !text.contains("\x1b"),
        "ANSI escape residue leaked"
    );
}

fn assert_no_secret_leak(text: &str) {
    for banned in [
        "sk-live-secret-token",
        "api_key=supersecret",
        "Bearer aquicktoken",
        "token=abc",
        "authorization: yes",
        "authorization:yes",
    ] {
        assert!(
            !text.to_ascii_lowercase().contains(&banned.to_ascii_lowercase()),
            "secret leaked into MCP surface: {banned}"
        );
    }
}

// ── VAL-V3-F4-057: Ready requires usable negotiated capability ──────────

#[test]
fn handshake_success_reports_ready_with_reconciled_counts() {
    let audit = audit_screen("mcp", "views.mcp.handshake-success")
        .expect("handshake success must render");
    audit.validate().expect("handshake success audit bijection");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);

    assert!(
        text.contains("Ready"),
        "success with usable capabilities reports Ready: {text}"
    );
    assert!(
        text.contains("Project docs") || text.contains("project-docs"),
        "usable capability identity remains visible"
    );
    let (n, a, b, u) = parse_summary_counts(&text)
        .expect("summary counts must render negotiated/allowed/blocked/unavailable");
    assert!(a >= 1, "allowed usable must be non-zero for Ready: {text}");
    assert_eq!(n, a + b + u, "counts must reconcile: n={n} a={a} b={b} u={u}");

    let case = replay_case("mcp-handshake-success", 1).expect("handshake success scenario");
    assert_eq!(case.before.selection, "project-docs");
    assert_eq!(case.after.selection, "project-docs");
    assert!(
        case.after.focus.contains("ready"),
        "after focus marks Ready: {}",
        case.after.focus
    );
    assert!(
        case.side_effects[0].status.contains("ready")
            || case.side_effects[0].effect_id.contains("handshake-success")
            || case.side_effects[0].kind.contains("mcp"),
        "side effect records usable Ready path: {:?}",
        case.side_effects[0]
    );
}

#[test]
fn handshake_zero_usable_never_reports_ready() {
    let audit = audit_screen("mcp", "views.mcp.handshake-zero-usable")
        .expect("zero-usable handshake must render");
    audit.validate().expect("zero-usable audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);

    assert!(
        !text.lines().any(|line| {
            let l = line.to_ascii_lowercase();
            l.contains("ready")
                && (l.contains("restricted")
                    || l.contains("usable")
                    || l.contains("connection")
                    || line.contains("·"))
        }),
        "zero usable capability must not paint Ready on the attention row: {text}"
    );
    // Explicit attention copy / state.
    assert!(
        text.contains("No usable capability")
            || text.contains("zero permitted")
            || text.contains("zero usable")
            || text.contains("not Ready")
            || text.contains("Policy"),
        "attention language required for zero-usable: {text}"
    );
    assert!(
        text.contains("Review policy") || text.contains("[enter] Review"),
        "one appropriate primary action required: {text}"
    );
    let primaries = primary_labels(&audit);
    assert_eq!(primaries.len(), 1, "exactly one primary: {primaries:?}");
    assert!(
        primaries.iter().any(|p| p.contains("Review policy")),
        "primary is Review policy: {primaries:?}"
    );

    let (n, a, b, u) = parse_summary_counts(&text).expect("counts present");
    assert_eq!(a, 0, "allowed must be zero: {text}");
    assert_eq!(n, a + b + u, "reconcile counts: n={n} a={a} b={b} u={u}");
    assert!(n >= 1 || b >= 1, "negotiated or blocked identities exist");

    let case = replay_case("mcp-handshake-zero-usable", 1).expect("zero-usable scenario");
    assert_eq!(case.after.selection, "restricted-repo");
    assert!(
        case.after.focus.contains("zero") || case.after.focus.contains("attention"),
        "focus stays attention, not ready: {}",
        case.after.focus
    );
    assert!(
        !case.after.focus.contains("ready"),
        "zero usable never focuses ready"
    );
    assert!(
        case.side_effects[0].status.contains("attention")
            || case.side_effects[0].effect_id.contains("zero-usable"),
        "audit records zero usable attention: {:?}",
        case.side_effects[0]
    );
}

#[test]
fn handshake_failed_never_reports_ready() {
    let audit = audit_screen("mcp", "views.mcp.handshake-failed")
        .expect("failed handshake must render");
    audit.validate().expect("failed handshake audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);

    assert!(
        text.contains("Failed") || text.contains("failed"),
        "failed handshake remains Failed: {text}"
    );
    // Docs bridge attention row must not be Ready.
    for line in text.lines() {
        if line.contains("Docs bridge") {
            assert!(
                !line.contains("Ready"),
                "Docs bridge must not be Ready: {line}"
            );
        }
    }
    assert!(text.contains("Retry") || text.contains("[enter] Retry"));

    let case = replay_case("mcp-handshake-failed", 1).expect("failed handshake scenario");
    assert_eq!(case.after.selection, "docs-bridge");
    assert!(
        case.after.focus.contains("failed"),
        "focus is failed: {}",
        case.after.focus
    );
    assert!(!case.after.focus.contains("ready"));
}

#[test]
fn mixed_list_counts_reconcile_from_stable_row_identities() {
    let audit = audit_screen("mcp", "views.mcp.mixed").expect("mixed list");
    audit.validate().expect("mixed audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);

    let (n, a, b, u) = parse_summary_counts(&text).expect("mixed list must show counts");
    assert_eq!(
        n,
        a + b + u,
        "list counts must reconcile negotiated=allowed+blocked+unavailable: n={n} a={a} b={b} u={u} text={text}"
    );
    // Stable identities from mixed fixture still present.
    for id_fragment in ["Deploy hooks", "Project docs", "Issue tracker", "Code search"] {
        assert!(
            text.contains(id_fragment),
            "stable identity missing: {id_fragment}"
        );
    }
    // Capability-row events in audit preserve identities.
    let cap_ids: Vec<_> = audit
        .events
        .iter()
        .filter(|e| e.identity.family == "capability-row")
        .map(|e| e.identity.candidate_id.clone())
        .collect();
    assert!(
        cap_ids.iter().any(|id| id.contains("deploy-hooks")),
        "deploy-hooks identity present: {cap_ids:?}"
    );
    assert!(
        cap_ids.iter().any(|id| id.contains("code-search")),
        "code-search identity present: {cap_ids:?}"
    );
}

// ── VAL-V3-F4-058: Untrusted MCP labels are sanitized ───────────────────

#[test]
fn hostile_labels_are_sanitized_on_list_surface() {
    let audit = audit_screen("mcp", "views.mcp.hostile-labels")
        .expect("hostile labels list must render");
    audit.validate().expect("hostile list audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_statusline_minimal(&text);
    assert_no_control_injection(&text);
    assert_no_secret_leak(&text);

    // Readable identity remains after sanitization.
    assert!(
        text.contains("Docs") || text.contains("tool") || text.contains("hostile"),
        "sanitized label remains readable: {text}"
    );
    // No layout overflow: every row is exactly 120 cols (already asserted).
    // Counts still reconcile on the list.
    let (n, a, b, u) = parse_summary_counts(&text).expect("counts on hostile list");
    assert_eq!(n, a + b + u, "reconcile on hostile list");
}

#[test]
fn hostile_labels_detail_discloses_provenance_without_secrets() {
    let audit = audit_screen("mcp", "views.mcp.hostile-detail")
        .expect("hostile detail must render");
    audit.validate().expect("hostile detail audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_control_injection(&text);
    assert_no_secret_leak(&text);

    assert!(
        text.contains("sanitized") || text.contains("Provenance") || text.contains("provenance"),
        "detail discloses sanitization/provenance: {text}"
    );
    assert!(
        text.contains("Capability")
            || text.contains("Publisher")
            || text.contains("Tool")
            || text.contains("Resource")
            || text.contains("Prompt"),
        "detail names capability-like fields"
    );
    assert!(
        text.contains("redacted")
            || text.contains("[redacted]")
            || text.contains("Not available")
            || text.contains("sanitized"),
        "secrets are redacted placeholders"
    );

    // Identity-stable across surfaces.
    let open = replay_case("mcp-hostile-labels-open", 1).expect("open hostile detail");
    assert_eq!(open.before.selection, "hostile-cap");
    assert_eq!(open.after.selection, "hostile-cap");
    assert!(
        open.after.focus.contains("detail"),
        "opens detail: {}",
        open.after.focus
    );

    let back = replay_case("mcp-hostile-labels-back", 1).expect("back from hostile detail");
    assert_eq!(back.after.selection, "hostile-cap");
    assert_eq!(back.after.scroll_anchor, back.before.scroll_anchor);
    assert!(
        back.after.focus.contains("list") || !back.after.focus.contains("detail"),
        "returns to list which same identity: {}",
        back.after.focus
    );
}

#[test]
fn diagnostics_hostile_fixture_stays_sanitized_and_width_safe() {
    let audit = audit_screen("mcp", "views.mcp.diagnostics-hostile")
        .expect("diagnostics hostile fixture");
    audit.validate().expect("diagnostics hostile audit");
    let text = audit.render_txt();
    assert_canvas_geometry(&text);
    assert_no_control_injection(&text);
    assert_no_secret_leak(&text);
    assert!(
        text.contains("sanitized")
            || text.contains("untrusted")
            || text.contains("redacted")
            || text.contains("Not available"),
        "diagnostics still sanitize hostile labels"
    );
}

#[test]
fn sanitize_helper_rejects_controls_and_secrets_unit_path() {
    // Indirect unit path: render fixtures that inject dangerous strings and
    // assert clean output (covers the sanitize path identity-stably).
    let hostile = audit_screen("mcp", "views.mcp.hostile-labels").expect("hostile");
    let text = hostile.render_txt();
    assert_no_control_injection(&text);
    assert_no_secret_leak(&text);
    // Identity survives: capability-row id is stable hostile-cap.
    assert!(
        hostile.events.iter().any(|e| {
            e.identity.family == "capability-row"
                && e.identity.candidate_id.contains("hostile-cap")
        }),
        "hostile-cap identity stable in audit"
    );
}
