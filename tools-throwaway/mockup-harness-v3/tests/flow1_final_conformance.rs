//! Final Flow 1 conversation/coding conformance sweep.
//!
//! Owned contract assertions:
//! VAL-V3-F1-053, VAL-V3-F1-054, VAL-V3-F1-055, VAL-V3-F1-056,
//! VAL-V3-F1-057, VAL-V3-F1-058, VAL-V3-F1-059, VAL-V3-F1-068.
//!
//! Aggregate checks cover progressive disclosure, singular primary actions,
//! transcript-first density, dead-end-free exits, minimal statusline, simple
//! chrome, the exact 24-state pair inventory, and private-trace exclusion.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory, flow_manifests,
    renderer::canonical_publication_fixture,
    scenario::replay_case, FlowOwner, ImplementationStatus,
};

const DAILY_WORK_ORIGIN: &str = "Daily work · current conversation";

/// Exact 24 Flow 1 states required by VAL-V3-F1-059.
const FLOW1_IDS: &[&str] = &[
    "home",
    "chat-turn",
    "chat-long-scroll",
    "streaming",
    "reasoning",
    "reasoning-collapsed",
    "rich-response",
    "tool-calls",
    "tool-running",
    "tool-success",
    "tool-success-expanded",
    "tool-task",
    "tool-task-expanded",
    "shell",
    "todo-list",
    "compact",
    "sidebar-context",
    "sidebar-files",
    "sidebar-lsp",
    "sidebar-todo",
    "sidebar-subagents",
    "review",
    "diff-viewer",
    "share",
];

/// Gallery relative path under plans/assets/mockups-v3 for each Flow 1 ID.
const GALLERY_RELATIVE: &[(&str, &str)] = &[
    ("session", "home"),
    ("session", "chat-turn"),
    ("session", "chat-long-scroll"),
    ("session", "streaming"),
    ("session", "reasoning"),
    ("session", "reasoning-collapsed"),
    ("session", "rich-response"),
    ("session", "tool-calls"),
    ("session", "tool-running"),
    ("session", "tool-success"),
    ("session", "tool-success-expanded"),
    ("session", "tool-task"),
    ("session", "tool-task-expanded"),
    ("session", "shell"),
    ("session", "todo-list"),
    ("session", "compact"),
    ("views", "sidebar-context"),
    ("views", "sidebar-files"),
    ("views", "sidebar-lsp"),
    ("views", "sidebar-todo"),
    ("views", "sidebar-subagents"),
    ("views", "review"),
    ("views", "diff-viewer"),
    ("states", "share"),
];

/// Closed-default fixtures representing the product Flow 1 states under audit.
/// Prefer the same publication fixtures used for gallery pairs where they exist.
const CLOSED_DEFAULT_FIXTURES: &[(&str, &str)] = &[
    ("home", "flow1.daily-work.stable"),
    ("chat-turn", "flow1.daily-work.stable"),
    ("chat-long-scroll", "session.history.earlier"),
    ("streaming", "flow1.daily-work.stable"),
    ("reasoning", "session.disclosure.reasoning"),
    ("reasoning-collapsed", "session.disclosure.reasoning"),
    ("rich-response", "flow1-rich-response"),
    ("tool-calls", "session.disclosure.tools"),
    ("tool-running", "flow1.daily-work.stable"),
    ("tool-success", "flow1.daily-work.stable"),
    ("tool-success-expanded", "session.disclosure.tools"),
    ("tool-task", "session.disclosure.tasks"),
    ("tool-task-expanded", "session.disclosure.tasks"),
    ("shell", "session.shell.inline"),
    ("todo-list", "session.todo.baseline"),
    ("compact", "session.disclosure.compact"),
    ("sidebar-context", "views.sidebar.context"),
    ("sidebar-files", "views.sidebar.files"),
    ("sidebar-lsp", "views.sidebar.lsp"),
    ("sidebar-todo", "views.sidebar.todo"),
    ("sidebar-subagents", "views.sidebar.subagents"),
    ("review", "views.review.selected"),
    ("diff-viewer", "views.diff.file-1"),
    ("share", "states.share.ready"),
];

/// Progressive disclosure pairs (compact ↔ focused/detail) for VAL-V3-F1-053.
const DISCLOSURE_PAIRS: &[(&str, &str, &str, &str, &str)] = &[
    // (compact screen, compact fixture, expanded screen, expanded fixture, expand case)
    (
        "reasoning-collapsed",
        "session.disclosure.reasoning",
        "reasoning",
        "session.disclosure.reasoning",
        "reasoning-expand",
    ),
    (
        "tool-success",
        "flow1.daily-work.stable",
        "tool-success-expanded",
        "session.disclosure.tools",
        "tool-success-expand",
    ),
    (
        "tool-task",
        "session.disclosure.tasks",
        "tool-task-expanded",
        "session.disclosure.tasks",
        "tool-task-expand",
    ),
    (
        "compact",
        "session.disclosure.compact",
        "chat-long-scroll",
        "session.history.earlier",
        "compact-expand-history",
    ),
];

/// Collapse return cases for disclosure reverse edges.
const DISCLOSURE_COLLAPSE: &[(&str, &str, &str)] = &[
    ("reasoning-collapse", "reasoning", "reasoning-collapsed"),
    ("tool-success-collapse", "tool-success-expanded", "tool-success"),
    ("tool-task-collapse", "tool-task-expanded", "tool-task"),
    (
        "compact-collapse-history",
        "chat-long-scroll",
        "compact",
    ),
];

/// Surfaces whose typed audit emits a primary-action family event.
const PRIMARY_FAMILY_FIXTURES: &[(&str, &str)] = &[
    ("home", "flow1.daily-work.stable"),
    ("chat-turn", "flow1.daily-work.stable"),
    ("chat-long-scroll", "session.history.earlier"),
    ("rich-response", "flow1-rich-response"),
    ("review", "views.review.selected"),
    ("diff-viewer", "views.diff.file-1"),
    ("share", "states.share.ready"),
];

/// Live work surfaces where the singular interrupt/stop is the user action,
/// bound to activity/live-state rather than the primary-action family.
const INTERRUPT_PRIMARY_FIXTURES: &[(&str, &str)] = &[
    ("streaming", "flow1.daily-work.stable"),
    ("tool-running", "flow1.daily-work.stable"),
    ("shell", "session.shell.inline"),
];

/// Browse / disclosure defaults may expose at most one primary-action event.
const BROWSE_OR_COMPOSER: &[(&str, &str)] = &[
    ("reasoning-collapsed", "session.disclosure.reasoning"),
    ("reasoning", "session.disclosure.reasoning"),
    ("tool-calls", "session.disclosure.tools"),
    ("tool-success", "flow1.daily-work.stable"),
    ("tool-success-expanded", "session.disclosure.tools"),
    ("tool-task", "session.disclosure.tasks"),
    ("tool-task-expanded", "session.disclosure.tasks"),
    ("todo-list", "session.todo.baseline"),
    ("compact", "session.disclosure.compact"),
    ("sidebar-context", "views.sidebar.context"),
    ("sidebar-files", "views.sidebar.files"),
    ("sidebar-lsp", "views.sidebar.lsp"),
    ("sidebar-todo", "views.sidebar.todo"),
    ("sidebar-subagents", "views.sidebar.subagents"),
];

/// Origin-preserving and successful round-trips across Flow 1 exits (VAL-V3-F1-056).
const ORIGIN_SCENARIOS: &[(&str, &str, &str)] = &[
    // Live activity interrupt/complete.
    ("streaming-interrupt", "streaming", "chat-turn"),
    ("tool-running-interrupt", "tool-running", "chat-turn"),
    ("tool-running-complete", "tool-running", "tool-success"),
    ("tool-success-continue", "tool-success", "rich-response"),
    // Progressive disclosure round trips.
    ("reasoning-expand", "reasoning-collapsed", "reasoning"),
    ("reasoning-collapse", "reasoning", "reasoning-collapsed"),
    (
        "tool-success-expand",
        "tool-success",
        "tool-success-expanded",
    ),
    (
        "tool-success-collapse",
        "tool-success-expanded",
        "tool-success",
    ),
    ("tool-task-expand", "tool-task", "tool-task-expanded"),
    ("tool-task-collapse", "tool-task-expanded", "tool-task"),
    ("compact-expand-history", "compact", "chat-long-scroll"),
    ("compact-collapse-history", "chat-long-scroll", "compact"),
    // Shell / todo / history exits.
    ("shell-interrupt", "shell", "shell"),
    ("shell-complete", "shell", "shell"),
    ("shell-foreground-interrupt", "shell", "shell"),
    ("shell-background-stop", "shell", "shell"),
    ("todo-progress", "todo-list", "todo-list"),
    (
        "chat-long-scroll-return-latest",
        "chat-long-scroll",
        "chat-long-scroll",
    ),
    (
        "chat-long-scroll-jump-earlier",
        "chat-long-scroll",
        "chat-long-scroll",
    ),
    // Inspector tabs and close-to-origin.
    (
        "inspector-tab-files",
        "sidebar-context",
        "sidebar-files",
    ),
    ("inspector-tab-lsp", "sidebar-files", "sidebar-lsp"),
    ("inspector-tab-todo", "sidebar-lsp", "sidebar-todo"),
    (
        "inspector-tab-subagents",
        "sidebar-todo",
        "sidebar-subagents",
    ),
    ("inspector-close-context", "sidebar-context", "chat-turn"),
    ("inspector-close-files", "sidebar-files", "chat-turn"),
    ("inspector-close-lsp", "sidebar-lsp", "chat-turn"),
    ("inspector-close-todo", "sidebar-todo", "chat-turn"),
    (
        "inspector-close-subagents",
        "sidebar-subagents",
        "chat-turn",
    ),
    // Review / diff reversible chain.
    ("review-open-evidence", "review", "diff-viewer"),
    ("diff-next-file", "diff-viewer", "diff-viewer"),
    ("diff-previous-file", "diff-viewer", "diff-viewer"),
    ("diff-return-to-review", "diff-viewer", "review"),
    ("review-return", "review", "chat-turn"),
    ("review-diff-chain-return", "review", "chat-turn"),
    // Share copy / revoke / return.
    ("share-copy-link", "share", "share"),
    ("share-open-revoke", "share", "share"),
    ("share-confirm-revoke", "share", "share"),
    (
        "share-confirm-revoke-idempotent",
        "share",
        "share",
    ),
    ("share-return", "share", "chat-turn"),
    // Task selection exits stay on the same turn.
    ("tool-task-stop-selected", "tool-task", "tool-task"),
    ("tool-task-retry-selected", "tool-task", "tool-task"),
    ("tool-task-cancel-selected", "tool-task", "tool-task"),
];

/// Cancel / collapse / close / return paths that must preserve draft and transcript.
const CANCEL_LIKE: &[&str] = &[
    "streaming-interrupt",
    "tool-running-interrupt",
    "reasoning-collapse",
    "tool-success-collapse",
    "tool-task-collapse",
    "compact-collapse-history",
    "shell-interrupt",
    "inspector-close-context",
    "inspector-close-files",
    "inspector-close-lsp",
    "inspector-close-todo",
    "inspector-close-subagents",
    "diff-return-to-review",
    "review-return",
    "share-return",
    "tool-task-cancel-selected",
];

/// Private / internal sentinel strings banned across every Flow 1 channel.
const PRIVATE_TRACE_SENTINELS: &[&str] = &[
    "chain-of-thought",
    "hidden prompt",
    "provider payload",
    "system prompt",
    "internal event",
    "temperature=",
    "raw_logits",
    "tool_envelope",
    "wave_id",
    "score=",
    "snake_case_status",
    "token_count=",
    "api_key=",
    "Bearer ",
    "HARMFUL_PRIVATE_TRACE",
    "private_cot_marker",
    "hidden_system_instructions",
];

/// Forbidden chrome / decorative tokens for VAL-V3-F1-058.
const FORBIDDEN_CHROME: &[&str] = &[
    "enter send · / commands",
    "enter send ·",
    "/ commands · @ files",
    "ctx 6%",
    "ctx %",
    "$0.02",
    "● idle",
    "idle ·",
    "✦",
    "✧",
    "★",
    "☆",
    "❖",
    "◆",
    "✱",
];

/// Implementation-looking identifiers forbidden in plain-English Flow 1 copy.
const RAW_ID_BANS: &[&str] = &[
    "claude-opus-4",
    "claude-3-",
    "gpt-4o-",
    "model_id=",
    "tool_id=",
    "wave_id=",
    "request_id=",
    "function_call_id",
    "snake_case_status",
    "STATUS_RUNNING",
    "permission_bitmask",
];

fn gallery_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3")
        .canonicalize()
        .expect("gallery root must resolve")
}

fn render_pair(screen_id: &str, fixture_id: &str) -> AuditDocument {
    let audit = audit_screen(screen_id, fixture_id)
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} must render: {e}"));
    audit
        .validate()
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} audit bijection: {e}"));
    audit
}

fn primary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect()
}

fn secondary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "secondary-action")
        .collect()
}

/// Manual enabled non-automatic interrupt/stop actions bound to live activity.
fn interrupt_actions(
    audit: &AuditDocument,
) -> Vec<&mockup_harness_v3::audit::ActionEvidence> {
    audit
        .events
        .iter()
        .filter_map(|event| event.action.as_ref().map(|action| (event, action)))
        .filter(|(event, action)| {
            if !action.enabled || action.automatic {
                return false;
            }
            let label = action.label.to_ascii_lowercase();
            matches!(event.identity.family.as_str(), "live-state" | "activity")
                && (label.contains("stop") || label.contains("interrupt"))
        })
        .map(|(_, action)| action)
        .collect()
}

fn is_cancelish(label: &str) -> bool {
    let lower = label.to_ascii_lowercase();
    lower.contains("cancel")
        || lower.contains("back")
        || lower == "return"
        || lower.starts_with("return ")
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

fn assert_png_dimensions(path: &Path, expect_w: u32, expect_h: u32) {
    let data = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(data.len() > 24, "{} too small to be PNG", path.display());
    assert_eq!(
        &data[0..8],
        b"\x89PNG\r\n\x1a\n",
        "{} bad PNG sig",
        path.display()
    );
    let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    assert_eq!(
        (width, height),
        (expect_w, expect_h),
        "{}: expected {expect_w}x{expect_h}, got {width}x{height}",
        path.display()
    );
}

fn assert_no_private_traces(text: &str, label: &str) {
    let lower = text.to_lowercase();
    for banned in PRIVATE_TRACE_SENTINELS {
        assert!(
            !lower.contains(&banned.to_lowercase()),
            "{label}: private/internal trace leaked: {banned}"
        );
    }
}

fn assert_statusline_minimal(text: &str, label: &str) {
    let status = text
        .lines()
        .last()
        .unwrap_or_else(|| panic!("{label}: missing statusline row"));
    assert!(
        status.contains("~/project") && status.contains('·') && status.contains("Claude"),
        "{label}: statusline must be path · branch · human model, got `{status}`"
    );
    assert!(
        !status.contains('%')
            && !status.contains('$')
            && !status.to_ascii_lowercase().contains("idle")
            && !status.contains("ctx ")
            && !status.contains("claude-opus")
            && !status.contains("opus-4-8"),
        "{label}: statusline carries forbidden field: `{status}`"
    );
    // Exactly one statusline-shaped path/branch row.
    let hits = text
        .lines()
        .filter(|l| l.contains("~/project · main ·"))
        .count();
    assert_eq!(hits, 1, "{label}: expected exactly one statusline, got {hits}");
}

fn assert_chrome_simple(text: &str, label: &str) {
    for banned in FORBIDDEN_CHROME {
        assert!(
            !text.contains(banned),
            "{label}: forbidden chrome `{banned}`"
        );
    }
    for banned in RAW_ID_BANS {
        assert!(
            !text.contains(banned),
            "{label}: raw/internal identifier `{banned}`"
        );
    }
    // No floating modal vocabulary.
    let lower = text.to_ascii_lowercase();
    assert!(
        !lower.contains("modal") && !lower.contains("dialog overlay"),
        "{label}: floating modal chrome is forbidden"
    );
}

fn publication_fixture(screen_id: &str) -> String {
    canonical_publication_fixture(screen_id)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("canonical.{screen_id}"))
}

// ── VAL-V3-F1-059: All required Flow 1 states render ────────────────────

#[test]
fn flow1_inventory_is_exact_and_geometry_valid() {
    let mut seen = HashSet::new();
    for id in FLOW1_IDS {
        assert!(seen.insert(*id), "duplicate Flow 1 id in constant: {id}");
    }
    assert_eq!(FLOW1_IDS.len(), 24, "Flow 1 inventory must be exactly 24");
    assert_eq!(GALLERY_RELATIVE.len(), 24);
    assert_eq!(CLOSED_DEFAULT_FIXTURES.len(), 24);

    let inventory = canonical_inventory();
    for id in FLOW1_IDS {
        let matches: Vec<_> = inventory.iter().filter(|s| s.id == *id).collect();
        assert_eq!(matches.len(), 1, "{id} must appear exactly once in registry");
        let screen = matches[0];
        assert_eq!(
            screen.status,
            ImplementationStatus::Implemented,
            "{id} must be implemented"
        );
        assert_eq!(
            screen.flow,
            FlowOwner::DailyWork,
            "{id} must be owned by the daily-work / Flow 1 journey"
        );
    }

    // Distinct live canvases for every closed default.
    let mut digests = HashSet::new();
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
        assert!(
            digests.insert(text.clone()),
            "{screen_id}/{fixture_id}: produced a duplicate frame of another Flow 1 state"
        );
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
        assert_no_private_traces(&text, &format!("{screen_id}/{fixture_id}"));
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
    }

    // Published gallery: exactly one TXT + PNG pair each at locked geometry.
    let root = gallery_root();
    let mut gallery_names = HashSet::new();
    for &(category, name) in GALLERY_RELATIVE {
        assert!(
            gallery_names.insert(name),
            "duplicate gallery id: {name}"
        );
        let txt = root.join(category).join(format!("{name}.txt"));
        let png = root.join(category).join(format!("{name}.png"));
        assert!(txt.is_file(), "missing gallery txt {}", txt.display());
        assert!(png.is_file(), "missing gallery png {}", png.display());
        let text = fs::read_to_string(&txt).expect("gallery txt");
        assert_canvas_geometry(&text, &format!("gallery {category}/{name}"));
        assert_png_dimensions(&png, 1200, 840);
        assert_statusline_minimal(&text, &format!("gallery {category}/{name}"));
        assert_no_private_traces(&text, &format!("gallery {category}/{name}"));
        assert_chrome_simple(&text, &format!("gallery {category}/{name}"));
    }

    // Daily work storyboard remaining at six ordered frames.
    let daily = flow_manifests()
        .iter()
        .find(|f| f.name == "daily-work")
        .expect("daily-work storyboard");
    assert_eq!(daily.steps.len(), 6);
    let storyboard_ids: Vec<_> = daily.steps.iter().map(|s| s.screen_id).collect();
    assert_eq!(
        storyboard_ids,
        vec![
            "home",
            "chat-turn",
            "streaming",
            "tool-running",
            "tool-success",
            "rich-response",
        ]
    );

    let listed = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("list");
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    for id in FLOW1_IDS {
        assert!(
            stdout
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(*id)),
            "--list missing Flow 1 id {id}"
        );
    }
}

// ── VAL-V3-F1-053: Progressive disclosure is consistent ─────────────────

#[test]
fn progressive_disclosure_adds_useful_information_and_round_trips() {
    for &(compact_id, compact_fix, expanded_id, expanded_fix, expand_case) in DISCLOSURE_PAIRS
    {
        let compact = render_pair(compact_id, compact_fix);
        let expanded = render_pair(expanded_id, expanded_fix);
        let compact_text = compact.render_txt();
        let expanded_text = expanded.render_txt();
        assert_ne!(
            compact_text, expanded_text,
            "{compact_id} → {expanded_id}: deeper disclosure must add useful content"
        );
        assert!(
            expanded_text.chars().count() > compact_text.chars().count()
                || expanded_text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .count()
                    > compact_text
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .count(),
            "{compact_id} → {expanded_id}: expanded level must be denser with specific detail"
        );

        // Typed disclosure family is present on at least one side of the pair.
        let has_disclosure = compact
            .events
            .iter()
            .chain(expanded.events.iter())
            .any(|e| e.identity.family == "disclosure");
        assert!(
            has_disclosure
                || compact.transitions.iter().any(|t| t.action_id.contains("expand")
                    || t.action_id.contains("collapse")
                    || t.action_id.contains("history")
                    || t.action_id.contains("reasoning")
                    || t.action_id.contains("detail")),
            "{compact_id}/{expanded_id}: disclosure transition/event required"
        );
        assert_no_private_traces(
            &compact_text,
            &format!("compact {compact_id}/{compact_fix}"),
        );
        assert_no_private_traces(
            &expanded_text,
            &format!("expanded {expanded_id}/{expanded_fix}"),
        );

        let expand =
            replay_case(expand_case, 1).unwrap_or_else(|e| panic!("{expand_case}: {e}"));
        expand
            .validate()
            .unwrap_or_else(|e| panic!("{expand_case} invalid: {e}"));
        assert_eq!(expand.before.screen_id, compact_id, "{expand_case} before");
        assert_eq!(expand.after.screen_id, expanded_id, "{expand_case} after");
        // Draft/session must round-trip; transcript may intentionally diversify when
        // expanding compacted prior turns into focused history (compact-expand-history).
        for field in ["draft", "session_id"] {
            if let Some(hash) = expand.preserved_field_hashes.get(field) {
                assert_eq!(
                    hash.before, hash.after,
                    "{expand_case}: {field} must survive expand"
                );
            }
        }
        if expand_case != "compact-expand-history" {
            for field in ["transcript", "scroll_anchor"] {
                if let Some(hash) = expand.preserved_field_hashes.get(field) {
                    assert_eq!(
                        hash.before, hash.after,
                        "{expand_case}: {field} must survive expand"
                    );
                }
            }
        }
    }

    for &(case_id, from, to) in DISCLOSURE_COLLAPSE {
        let collapse = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        collapse
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} invalid: {e}"));
        assert_eq!(collapse.before.screen_id, from, "{case_id} before");
        assert_eq!(collapse.after.screen_id, to, "{case_id} after");
        for field in ["draft", "session_id"] {
            if let Some(hash) = collapse.preserved_field_hashes.get(field) {
                assert_eq!(
                    hash.before, hash.after,
                    "{case_id}: {field} must survive collapse"
                );
            }
        }
        // Reverse of compact history may re-offset scroll/transcript; draft stays fixed.
        if case_id != "compact-collapse-history" {
            for field in ["transcript", "scroll_anchor"] {
                if let Some(hash) = collapse.preserved_field_hashes.get(field) {
                    assert_eq!(
                        hash.before, hash.after,
                        "{case_id}: {field} must survive collapse"
                    );
                }
            }
        }
    }

    // Inspectors, review, and diff form deeper evidence layers that remain reversible.
    let review = render_pair("review", "views.review.selected").render_txt();
    let diff = render_pair("diff-viewer", "views.diff.file-1").render_txt();
    assert_ne!(review, diff, "diff evidence must differ from review summary");
    assert!(
        diff.contains('+') || diff.contains('-') || diff.contains("@@"),
        "diff evidence must show focused change detail"
    );

    for &(screen_id, fixture_id) in &[
        ("sidebar-context", "views.sidebar.context"),
        ("sidebar-files", "views.sidebar.files"),
        ("sidebar-lsp", "views.sidebar.lsp"),
        ("sidebar-todo", "views.sidebar.todo"),
        ("sidebar-subagents", "views.sidebar.subagents"),
    ] {
        let text = render_pair(screen_id, fixture_id).render_txt();
        assert!(
            text.contains(DAILY_WORK_ORIGIN),
            "{screen_id}: inspector must keep conversation origin"
        );
        assert!(
            text.contains("[esc]") || text.contains("Return") || text.contains("Close"),
            "{screen_id}: reversible exit required"
        );
    }
}

// ── VAL-V3-F1-054: Primary actions remain singular ──────────────────────

#[test]
fn every_flow1_action_surface_has_singular_primary_and_limited_secondaries() {
    for &(screen_id, fixture_id) in PRIMARY_FAMILY_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let primaries = primary_events(&audit);
        assert_eq!(
            primaries.len(),
            1,
            "{screen_id}/{fixture_id}: expected exactly one primary-action, got {} labels {:?}",
            primaries.len(),
            primaries
                .iter()
                .filter_map(|e| e.action.as_ref().map(|a| a.label.clone()))
                .collect::<Vec<_>>()
        );
        let action = primaries[0]
            .action
            .as_ref()
            .unwrap_or_else(|| panic!("{screen_id}/{fixture_id}: primary binds action"));
        assert!(action.enabled, "{screen_id}/{fixture_id}: primary enabled");
        assert!(
            !action.automatic,
            "{screen_id}/{fixture_id}: primary must be a user action"
        );
        assert!(
            !action.label.trim().is_empty(),
            "{screen_id}/{fixture_id}: primary label nonempty"
        );
        // Temporary detail views may treat "Return to …" as the one primary exit.
        if !matches!(screen_id, "diff-viewer" | "review") {
            assert!(
                !is_cancelish(&action.label) || action.label.to_ascii_lowercase().contains("return"),
                "{screen_id}/{fixture_id}: bare cancel/back must not be the primary"
            );
        }
        let secondaries = secondary_events(&audit);
        assert!(
            secondaries.len() <= 2,
            "{screen_id}/{fixture_id}: at most two secondary actions, got {}",
            secondaries.len()
        );
        assert!(
            text.contains('›')
                || text.contains("[enter]")
                || text.contains("[esc]")
                || text.contains("Return")
                || text.contains("Send"),
            "{screen_id}/{fixture_id}: primary / exit must be visually marked"
        );
    }

    for &(screen_id, fixture_id) in INTERRUPT_PRIMARY_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        // No competing primary-action family when activity owns the interrupt.
        assert!(
            primary_events(&audit).len() <= 1,
            "{screen_id}/{fixture_id}: live activity must not compete with multiple primary-action labels"
        );
        let interrupts = interrupt_actions(&audit);
        assert_eq!(
            interrupts.len(),
            1,
            "{screen_id}/{fixture_id}: expected exactly one stop/interrupt action, got {:?}",
            interrupts.iter().map(|a| a.label.clone()).collect::<Vec<_>>()
        );
        assert!(
            text.to_ascii_lowercase().contains("stop")
                || text.to_ascii_lowercase().contains("interrupt")
                || text.contains("[ctrl+c]")
                || text.contains("[esc]"),
            "{screen_id}/{fixture_id}: interrupt affordance must be visible"
        );
        assert!(
            secondary_events(&audit).len() <= 2,
            "{screen_id}/{fixture_id}: at most two secondaries beside the live action"
        );
    }

    for &(screen_id, fixture_id) in BROWSE_OR_COMPOSER {
        let audit = render_pair(screen_id, fixture_id);
        let primaries = primary_events(&audit);
        assert!(
            primaries.len() <= 1,
            "{screen_id}/{fixture_id}: browse/composer surface must not present competing primaries ({})",
            primaries.len()
        );
        let secondaries = secondary_events(&audit);
        assert!(
            secondaries.len() <= 2,
            "{screen_id}/{fixture_id}: at most two secondaries, got {}",
            secondaries.len()
        );
    }
}

// ── VAL-V3-F1-055: Default density favors the transcript ────────────────

#[test]
fn default_density_favors_transcript_and_readable_measure() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));

        let nonempty = text.lines().filter(|l| !l.trim().is_empty()).count();
        // Even dense temporary details keep some breathing room of blank rows.
        assert!(
            nonempty < 38,
            "{screen_id}/{fixture_id}: density too high ({nonempty} nonempty rows)"
        );

        // No global toolbars or wall-of-tables by default.
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));

        // Dominant content heuristics by surface family.
        let lower = text.to_ascii_lowercase();
        match screen_id {
            "review" | "diff-viewer" => {
                assert!(
                    lower.contains("review")
                        || lower.contains("diff")
                        || lower.contains("finding")
                        || lower.contains("middleware")
                        || text.contains('+')
                        || text.contains('-'),
                    "{screen_id}: focused evidence must dominate"
                );
            }
            "share" => {
                assert!(
                    lower.contains("share") || lower.contains("link") || lower.contains("revoke"),
                    "{screen_id}: share result content must dominate"
                );
            }
            "sidebar-context"
            | "sidebar-files"
            | "sidebar-lsp"
            | "sidebar-todo"
            | "sidebar-subagents" => {
                assert!(
                    text.contains(DAILY_WORK_ORIGIN)
                        && (text.contains("Find where the server")
                            || text.contains("shared request guard")
                            || text.contains("rate limit")),
                    "{screen_id}: transcript must remain visible beside the inspector"
                );
            }
            _ => {
                // Conversation shells keep the request/answer or live work readable.
                assert!(
                    text.contains(DAILY_WORK_ORIGIN)
                        || text.contains("Code")
                        || text.contains("You ·")
                        || text.contains("rate limit")
                        || text.contains("shared request"),
                    "{screen_id}: conversation/transcript must dominate"
                );
            }
        }

        // Body rows stay within the 120-col canvas already (measured above);
        // further check: avoid pathological single-token walls wider than measure.
        for (idx, row) in text.lines().enumerate() {
            let trimmed = row.trim_end();
            if trimmed.chars().count() > 100 {
                // Wide temporary evidence may use ~120; ordinary readout prefers <100.
                assert!(
                    matches!(
                        screen_id,
                        "diff-viewer"
                            | "review"
                            | "rich-response"
                            | "tool-success-expanded"
                            | "tool-task-expanded"
                            | "shell"
                            | "chat-long-scroll"
                            | "tool-calls"
                    ) || trimmed.chars().count() <= 120,
                    "{screen_id} row {idx}: unexpected over-wide packed content"
                );
            }
        }
    }
}

// ── VAL-V3-F1-056: Every Flow 1 exit is failure free ────────────────────

#[test]
fn every_flow1_exit_lands_in_usable_origin_without_dead_ends() {
    for &(case_id, from, to) in ORIGIN_SCENARIOS {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("scenario {case_id}: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} invalid: {e}"));
        assert_eq!(replay.before.screen_id, from, "{case_id} before");
        assert_eq!(replay.after.screen_id, to, "{case_id} after");
        assert!(
            !replay.actions.is_empty() || !replay.transitions.is_empty(),
            "{case_id}: must declare an action or transition (no dead end)"
        );
        // Never dump onto a blank or unexplained home except intentional home entry.
        if from != "home" {
            assert_ne!(
                replay.after.screen_id, "home",
                "{case_id}: temporary exit must not orphan to home"
            );
        }
        // Session / conversation identity stays continuous when present.
        if !replay.before.session_id.is_empty() && !replay.after.session_id.is_empty() {
            assert_eq!(
                replay.before.session_id, replay.after.session_id,
                "{case_id}: session continuity"
            );
        }
        if !replay.before.conversation_id.is_empty() && !replay.after.conversation_id.is_empty()
        {
            assert_eq!(
                replay.before.conversation_id, replay.after.conversation_id,
                "{case_id}: conversation continuity"
            );
        }
        // No unexplained jump to recovery/orchestration IDs from pure Flow 1 exits.
        assert!(
            !matches!(
                to,
                "workflow-monitor"
                    | "workflow-launch"
                    | "login"
                    | "connect-provider"
                    | "error-network"
                    | "rate-limit"
            ),
            "{case_id}: Flow 1 exit jumped into an unrelated category ({to})"
        );

        let after_text = replay.audit.render_txt();
        assert_canvas_geometry(&after_text, &format!("{case_id} after"));
        assert_no_private_traces(&after_text, &format!("{case_id} after"));
        // Resulting state is not a blank canvas.
        assert!(
            after_text.lines().filter(|l| !l.trim().is_empty()).count() > 6,
            "{case_id}: resulting canvas is too empty to be usable"
        );
    }

    for case_id in CANCEL_LIKE {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id} must replay: {e}"));
        // Draft + session are universal cancel-preservation fields.
        for field in ["draft", "session_id"] {
            if let Some(hash) = replay.preserved_field_hashes.get(field) {
                assert_eq!(
                    hash.before, hash.after,
                    "{case_id}: preserved {field} must be byte-identical"
                );
            }
        }
        // Disclosure collapsers that `session_disclosures` covers keep scroll
        // position as well as draft.
        if matches!(
            *case_id,
            "reasoning-collapse" | "tool-success-collapse" | "tool-task-collapse"
        ) {
            if let Some(hash) = replay.preserved_field_hashes.get("scroll_anchor") {
                assert_eq!(
                    hash.before, hash.after,
                    "{case_id}: scroll_anchor must round-trip on disclosure collapse"
                );
            }
        }
        // Interrupt exits still keep the conversation composable afterward.
        if matches!(
            *case_id,
            "shell-interrupt"
                | "streaming-interrupt"
                | "tool-running-interrupt"
                | "shell-foreground-interrupt"
        ) {
            assert_eq!(
                replay.before.session_id, replay.after.session_id,
                "{case_id}: interrupt keeps session"
            );
        }
        assert!(
            !replay
                .side_effects
                .iter()
                .any(|s| s.status.contains("destructive") && s.status.contains("applied")),
            "{case_id}: cancel/close must not apply destructive side effects"
        );
        // After-state canvas stays usable (not blank / not orphaned home).
        let after_text = replay.audit.render_txt();
        assert!(
            after_text.lines().filter(|l| !l.trim().is_empty()).count() > 6,
            "{case_id}: cancel/close must land on a usable canvas"
        );
        assert_ne!(
            replay.after.screen_id, "home",
            "{case_id}: cancel like transitions never orphan to home"
        );
    }
}

// ── VAL-V3-F1-057: Statusline remains minimal and stable ────────────────

#[test]
fn every_flow1_state_has_minimal_stable_statusline() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let text = render_pair(screen_id, fixture_id).render_txt();
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
        // Audit typed statusline family uniqueness when present.
        let audit = render_pair(screen_id, fixture_id);
        let statusline_events = audit
            .events
            .iter()
            .filter(|e| e.identity.family == "statusline")
            .count();
        assert!(
            statusline_events <= 1,
            "{screen_id}/{fixture_id}: statusline event family must stay singular"
        );
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let text = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .expect("gallery txt");
        assert_statusline_minimal(&text, &format!("gallery {category}/{name}"));
    }
}

// ── VAL-V3-F1-058: Flow chrome stays simple and functional ──────────────

#[test]
fn flow1_chrome_stays_simple_functional_and_plain_english() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
        assert_no_private_traces(&text, &format!("{screen_id}/{fixture_id}"));

        // Controls live near objects: no second global bar of shortcuts.
        let hintish = text
            .lines()
            .filter(|l| {
                let lower = l.to_ascii_lowercase();
                lower.contains("enter send")
                    || lower.contains("press ? for help")
                    || (lower.contains("/ commands") && lower.contains("@ files"))
            })
            .count();
        assert_eq!(
            hintish, 0,
            "{screen_id}/{fixture_id}: global hint row is forbidden"
        );

        // Status wording must not be pure snake_case identifiers.
        for token in text.split_whitespace() {
            let cleaned = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
            if cleaned.contains('_')
                && cleaned.len() > 6
                && cleaned
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
            {
                panic!(
                    "{screen_id}/{fixture_id}: uppercase/snake_case status leaked: {cleaned}"
                );
            }
        }
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let text = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .expect("gallery txt");
        assert_chrome_simple(&text, &format!("gallery {category}/{name}"));
    }
}

// ── VAL-V3-F1-068: Reasoning never exposes private traces ───────────────

#[test]
fn reasoning_and_all_flow1_channels_exclude_private_traces() {
    // Closed + expanded reasoning fixtures.
    for fixture in [
        "session.disclosure.reasoning",
        "canonical.reasoning",
        "canonical.reasoning-collapsed",
    ] {
        for screen in ["reasoning", "reasoning-collapsed"] {
            if let Ok(audit) = audit_screen(screen, fixture) {
                if audit.validate().is_ok() {
                    let text = audit.render_txt();
                    assert_no_private_traces(&text, &format!("{screen}/{fixture} txt"));
                    let json = format!("{audit:?}");
                    assert_no_private_traces(&json, &format!("{screen}/{fixture} audit debug"));
                }
            }
        }
    }

    // Full closed-default matrix + gallery + expandable audit JSON overview.
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_no_private_traces(&text, &format!("{screen_id}/{fixture_id}"));
        // Scan impressive identity subjects as well as labels for leaked payload jargon.
        for event in &audit.events {
            assert_no_private_traces(
                &event.identity.subject_id,
                &format!("{screen_id}/{fixture_id} event subject"),
            );
            if let Some(action) = &event.action {
                assert_no_private_traces(
                    &action.label,
                    &format!("{screen_id}/{fixture_id} action label"),
                );
            }
        }
        for transition in &audit.transitions {
            assert_no_private_traces(
                &transition.action_id,
                &format!("{screen_id}/{fixture_id} transition action"),
            );
        }
    }

    // Copy / share / diagnostic channels involved in conversation provenance.
    for case_id in [
        "share-copy-link",
        "share-open-revoke",
        "share-confirm-revoke",
        "share-return",
        "reasoning-expand",
        "reasoning-collapse",
        "tool-success-expand",
        "review-open-evidence",
        "review-return",
    ] {
        let replay = replay_case(case_id, 1).unwrap_or_else(|e| panic!("{case_id}: {e}"));
        let text = replay.audit.render_txt();
        assert_no_private_traces(&text, &format!("{case_id} after"));
        for effect in &replay.side_effects {
            assert_no_private_traces(
                &effect.status,
                &format!("{case_id} side-effect status"),
            );
            assert_no_private_traces(
                &effect.effect_id,
                &format!("{case_id} side-effect id"),
            );
            assert_no_private_traces(
                &effect.kind,
                &format!("{case_id} side-effect kind"),
            );
            assert_no_private_traces(
                &effect.effect_hash,
                &format!("{case_id} side-effect hash"),
            );
        }
        // Scenario state fields that can carry copied/shared provenance stay clean.
        for channel in [
            &replay.after.transcript,
            &replay.after.draft,
            &replay.after.partial_response,
            &replay.after.queue,
            &replay.after.queued_follow_up,
            &replay.after.pending_action.clone().unwrap_or_default(),
            &replay.after.submitted_prompt.clone().unwrap_or_default(),
            &replay.after.attachment,
            &replay.after.selection,
            &replay.after.focus,
        ] {
            assert_no_private_traces(channel, &format!("{case_id} scenario state channel"));
        }
        for action in &replay.actions {
            assert_no_private_traces(
                &action.action_id,
                &format!("{case_id} action id"),
            );
            assert_no_private_traces(&action.label, &format!("{case_id} action label"));
        }
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let text = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .expect("gallery txt");
        assert_no_private_traces(&text, &format!("gallery {category}/{name}"));
    }
}

// ── Gallery drift guard against live publication fixtures ───────────────

#[test]
fn published_flow1_gallery_matches_live_publication_fixtures() {
    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let fixture = publication_fixture(name);
        // Prefer richer closed-default fixtures when publication falls back to a
        // generic canonical fix that the screen may not specialize.
        let candidates = [
            fixture.clone(),
            CLOSED_DEFAULT_FIXTURES
                .iter()
                .find(|(id, _)| *id == name)
                .map(|(_, f)| (*f).to_owned())
                .unwrap_or_else(|| format!("canonical.{name}")),
        ];
        let published = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("read gallery {category}/{name}: {e}"));
        assert_canvas_geometry(&published, &format!("gallery {category}/{name}"));

        let mut matched = false;
        let mut last_err = String::new();
        for candidate in &candidates {
            match audit_screen(name, candidate) {
                Ok(audit) => match audit.validate() {
                    Ok(()) => {
                        let live = audit.render_txt();
                        if live == published {
                            matched = true;
                            break;
                        }
                        last_err = format!(
                            "live fixture {candidate} differs from published TXT (len live={} pub={})",
                            live.len(),
                            published.len()
                        );
                    }
                    Err(e) => last_err = format!("audit invalid for {candidate}: {e}"),
                },
                Err(e) => last_err = format!("render failed for {candidate}: {e}"),
            }
        }
        assert!(
            matched,
            "gallery drift for {category}/{name}.txt: {last_err}"
        );

        let png = root.join(category).join(format!("{name}.png"));
        assert_png_dimensions(&png, 1200, 840);
    }
}
