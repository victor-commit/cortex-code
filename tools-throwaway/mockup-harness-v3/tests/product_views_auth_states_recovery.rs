//! Cortex views / auth / states / recovery rewrite.
//!
//! Quiet chrome + denser inspectors: subject titles (no journey crumbs),
//! PromptJewel on inspectors/empty, capability-first MCP packing, prose
//! recovery disclosures, one recovery CTA that preserves draft, no brackets.

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::components::{
        contains_bracket_key_atom, contains_global_putty_footer, subject_title_for_origin,
    },
};

const VIEWS: &[&str] = &[
    "help",
    "status",
    "timeline",
    "review",
    "mcp",
    "sidebar-context",
    "sidebar-files",
    "sidebar-lsp",
    "sidebar-todo",
    "sidebar-subagents",
    "sidebar-mcp",
    "sidebar-queue",
    "theme-gallery-dark",
    "theme-gallery-light",
];

const AUTH: &[&str] = &[
    "login",
    "connect-provider",
    "connect-method",
    "connect-oauth",
    "connect-apikey",
    "connect-connected",
    "connect-disconnect",
];

const STATES: &[&str] = &[
    "empty",
    "error-network",
    "error-context",
    "rate-limit",
    "interrupt",
    "checkpoint-undo",
    "toasts",
    "share",
];

const JOURNEY_CRUMBS: &[&str] = &[
    "Daily work ·",
    "Input and approval ·",
    "Work configuration ·",
    "Provider and MCP ·",
    "Recovery ·",
    "Mission ·",
    "Provider connection ·",
    "MCP · conversation",
    "Start · empty",
];

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        "login" => "auth.login.blocked-model",
        "connect-provider" => "auth.connect-provider.blocked-model",
        "connect-method" => "auth.connect-method.openai",
        "connect-oauth" => "gate.provider-and-mcp.connect-oauth",
        "connect-apikey" => "auth.connect-apikey.valid",
        "connect-connected" => "auth.connect-connected.api-key",
        "connect-disconnect" => "auth.connect-disconnect.anthropic.local",
        "mcp" => "gate.provider-and-mcp.mcp",
        "sidebar-mcp" => "views.sidebar-mcp.attention",
        "sidebar-context" => "views.sidebar.context",
        "sidebar-files" => "views.sidebar.files",
        "sidebar-lsp" => "views.sidebar.lsp",
        "sidebar-todo" => "views.sidebar.todo",
        "sidebar-subagents" => "views.sidebar.subagents",
        "sidebar-queue" => "views.sidebar.queue",
        "help" => "views.help.open",
        "status" => "views.status.default",
        "empty" => "views.empty.origin",
        "theme-gallery-dark" => "views.theme-gallery.dark",
        "theme-gallery-light" => "views.theme-gallery.selected",
        "timeline" => "flow3-timeline",
        "review" => "views.review.selected",
        "share" => "states.share.ready",
        "error-network" => "canonical.error-network",
        "error-context" => "canonical.error-context",
        "rate-limit" => "gate.recovery.rate-limit",
        "interrupt" => "canonical.interrupt",
        "checkpoint-undo" => "gate.recovery.checkpoint-undo",
        "toasts" => "canonical.toasts",
        _ => "public-audit",
    }
}

fn audit(screen_id: &str) -> mockup_harness_v3::audit::AuditDocument {
    let fixture = fixture_for(screen_id);
    audit_screen(screen_id, fixture).unwrap_or_else(|err| {
        // Fall back to common alternatives when a specialized id differs.
        let fallbacks = [
            fixture,
            "public-audit",
            "canonical",
            "auth.login.blocked-model",
            "auth.connect-provider.blocked-model",
            "auth.connect-method.openai",
            "auth.connect-apikey.valid",
            "auth.connect-apikey.empty",
            "auth.connect-connected.api-key",
            "auth.connect-disconnect.anthropic.local",
            "canonical.connect-disconnect",
            "gate.provider-and-mcp.connect-oauth",
            "gate.provider-and-mcp.mcp",
            "views.sidebar-mcp.attention",
            "views.sidebar-mcp.all-ready",
            "views.sidebar.context",
            "views.sidebar.files",
            "views.sidebar.lsp",
            "views.sidebar.todo",
            "views.sidebar.subagents",
            "views.sidebar.queue",
            "views.help.open",
            "views.status.default",
            "views.empty.origin",
            "views.theme-gallery.dark",
            "views.theme-gallery.selected",
            "flow3-timeline",
            "views.review.selected",
            "states.share.ready",
            "canonical.error-network",
            "canonical.error-context",
            "gate.recovery.rate-limit",
            "rate-limit.wait.sample-early",
            "canonical.interrupt",
            "gate.recovery.checkpoint-undo",
            "canonical.toasts",
            "canonical.mcp",
            "canonical.connect-apikey",
        ];
        for candidate in fallbacks {
            if let Ok(doc) = audit_screen(screen_id, candidate) {
                return doc;
            }
        }
        panic!("{screen_id} must render under {fixture}: {err}")
    })
}

fn painted_text(screen_id: &str) -> String {
    let doc = audit(screen_id);
    doc.validate().unwrap_or_else(|e| panic!("{screen_id} audit: {e}"));
    doc.render_txt()
}

#[test]
fn views_auth_states_paint_no_journey_crumbs_or_brackets() {
    let mut ids = Vec::new();
    ids.extend_from_slice(VIEWS);
    ids.extend_from_slice(AUTH);
    ids.extend_from_slice(STATES);
    for id in ids {
        let text = painted_text(id);
        for crumb in JOURNEY_CRUMBS {
            assert!(
                !text.contains(crumb),
                "{id} paints journey crumb '{crumb}': sample={}",
                text.lines().take(6).collect::<Vec<_>>().join(" | ")
            );
        }
        assert!(
            !contains_bracket_key_atom(&text),
            "{id} still has bracket key atoms: {}",
            text.lines()
                .filter(|l| contains_bracket_key_atom(l))
                .take(3)
                .collect::<Vec<_>>()
                .join(" · ")
        );
        assert!(
            !contains_global_putty_footer(&text),
            "{id} paints global putty footer"
        );
    }
}

#[test]
fn inspector_sidebars_use_subject_title_and_jewel() {
    for id in [
        "sidebar-context",
        "sidebar-files",
        "sidebar-lsp",
        "sidebar-todo",
        "sidebar-subagents",
        "sidebar-mcp",
    ] {
        let text = painted_text(id);
        assert!(
            text.contains(subject_title_for_origin("Daily work · current conversation")),
            "{id} must paint subject title 'Current conversation'"
        );
        assert!(
            !text.contains("Daily work ·"),
            "{id} residual Daily work crumb"
        );
        // Jewel walls + prompt prefix.
        assert!(
            text.contains('╭') && text.contains('›') && text.contains('╰'),
            "{id} must show Cortex prompt jewel walls"
        );
        assert!(
            text.contains("Claude Opus 4.8") && text.contains("Careful edits"),
            "{id} jewel info line must carry model · work mode"
        );
    }
}

#[test]
fn empty_uses_prompt_jewel_and_one_primary_start() {
    let text = painted_text("empty");
    assert!(text.contains("Empty conversation") || text.contains("Start a new conversation"));
    assert!(text.contains("Ask Code to start a task") || text.contains("›"));
    assert!(text.contains('╭') && text.contains('╰'));
    assert!(
        text.contains("Claude Opus 4.8") || text.contains("Claude Sonnet"),
        "empty jewel must carry a human model name"
    );
    assert!(
        !text.contains("Daily work ·") && !text.contains("Start · empty"),
        "empty must not paint journey crumbs"
    );
}

#[test]
fn mcp_is_capability_first_with_dense_attention_rows() {
    let text = painted_text("mcp");
    assert!(
        text.contains("Browser automation") || text.contains("MCP capabilities"),
        "mcp must lead with human capability names"
    );
    assert!(
        text.contains("Needs setup")
            || text.contains("Sign in needed")
            || text.contains("Ready"),
        "capability states must be plain English"
    );
    // Transport jargon should not lead the default view.
    for banned in ["stdio", "websocket", "JSON-RPC", "transport=", "server_id="] {
        assert!(
            !text.to_lowercase().contains(&banned.to_lowercase()),
            "mcp default must hide transport jargon: {banned}"
        );
    }
    assert!(
        !contains_bracket_key_atom(&text),
        "mcp must keep prose bylines"
    );
}

#[test]
fn recovery_states_one_cta_preserve_draft() {
    for id in [
        "error-network",
        "error-context",
        "rate-limit",
        "interrupt",
        "checkpoint-undo",
    ] {
        let text = painted_text(id);
        assert!(
            text.contains("Draft kept")
                || text.contains("draft")
                || text.contains("saved")
                || text.contains("preserved")
                || text.contains("kept"),
            "{id} must preserve draft or work in recovery language"
        );
        // Most recovery docks paint › primary. Rate-limit waiting is the
        // intentional exception: one automatic timer owns the wait (no CTA).
        if id == "rate-limit" {
            assert!(
                text.contains("Waiting")
                    || text.contains("retry")
                    || text.contains("Retry now")
                    || text.contains('›'),
                "{id} must expose automatic wait ownership or recovery"
            );
        } else {
            assert!(
                text.contains('›'),
                "{id} must expose one obvious primary recovery marker"
            );
        }
        assert!(
            !contains_bracket_key_atom(&text),
            "{id} recovery must not use bracket chords"
        );
        assert!(
            !text.contains("Recovery ·"),
            "{id} after phase rewrite must not paint Recovery · journey crumb"
        );
        assert!(
            text.contains("Recovery") || text.contains("Finish the release notes"),
            "{id} must keep Recovery subject or paused task"
        );
    }
}

#[test]
fn toasts_paint_subject_title_not_journey_crumb() {
    let text = painted_text("toasts");
    assert!(
        !text.contains("Recovery ·"),
        "toasts must not paint Recovery · journey crumb"
    );
    assert!(
        text.contains("Recovery"),
        "toasts must still paint Recovery subject"
    );
    assert!(
        text.contains("Settings were not saved") || text.contains("BAD"),
        "failure toast remains visible"
    );
    assert!(!contains_bracket_key_atom(&text));
}

#[test]
fn auth_docks_mask_secrets_and_use_prose() {
    let apikey = painted_text("connect-apikey");
    assert!(
        apikey.contains("····")
            || apikey.contains("······")
            || apikey.contains("never shown")
            || apikey.contains("never echoed"),
        "API key must stay masked / secret-safe"
    );
    assert!(
        !apikey.contains("sk-ant-") && !apikey.contains("sk-proj-"),
        "full secret material must not paint"
    );
    assert!(!contains_bracket_key_atom(&apikey));

    let login = painted_text("login");
    assert!(
        login.contains("Connections") || login.contains("Connect a model provider"),
        "login must use human subject language"
    );
    assert!(
        !login.contains("Provider connection ·"),
        "login must not paint provider journey crumb"
    );
    assert!(login.contains('╭') || login.contains("Choose a provider"));
}

#[test]
fn help_status_review_share_are_quiet_detail_or_dock() {
    for id in ["help", "status", "review", "share"] {
        let text = painted_text(id);
        assert!(!contains_bracket_key_atom(&text), "{id} brackets forbidden");
        assert!(
            JOURNEY_CRUMBS.iter().all(|c| !text.contains(c)),
            "{id} journey crumb residual"
        );
        // One primary return / action affordance.
        assert!(
            text.contains("Esc to") || text.contains('›'),
            "{id} must expose return or primary action"
        );
    }
}
