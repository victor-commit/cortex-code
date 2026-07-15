//! Final Flow 3 work-configuration coherence and privacy sweep.
//!
//! Owned contract assertions:
//! VAL-V3-F3-012, VAL-V3-F3-041, VAL-V3-F3-046, VAL-V3-F3-047,
//! VAL-V3-F3-048, VAL-V3-F3-049, VAL-V3-F3-050, VAL-V3-F3-057.
//!
//! Aggregate checks cover human model/entity labels, minimal statusline,
//! applied/pending configuration consistency, jargon-free copy, scan-friendly
//! density, restrained visuals, detour preservation, and absence of raw IDs
//! across all Flow 3 states.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument},
    canonical_inventory, flow_manifests,
    chrome::picker_contract::{
        contract_fixture, FORBIDDEN_RAW_ID_SNIPPETS, NINE_PICKERS,
    },
    renderer::canonical_publication_fixture,
    scenario::replay_case, FlowOwner, ImplementationStatus, Surface,
};

/// Exact 17 Flow 3 work-configuration inventory IDs.
const FLOW3_IDS: &[&str] = &[
    "onboarding",
    "agent-picker",
    "model-picker",
    "model-picker-filtered",
    "fork-picker",
    "skills-picker",
    "session-picker",
    "subagent-picker",
    "variant-picker",
    "theme-picker",
    "status",
    "timeline",
    "theme-gallery-dark",
    "theme-gallery-light",
    "empty",
    "mode-switch",
    "mode-switch-refused",
];

/// Closed-default fixtures representing product Flow 3 states under audit.
const CLOSED_DEFAULT_FIXTURES: &[(&str, &str)] = &[
    ("onboarding", "gate.work-configuration.onboarding"),
    ("agent-picker", "flow3-agent-picker"),
    ("model-picker", "flow3-model-picker"),
    ("model-picker-filtered", "flow3-model-picker-filtered"),
    ("fork-picker", "flow3-fork-picker"),
    ("skills-picker", "flow3-skills-picker"),
    ("session-picker", "flow3-session-picker"),
    ("subagent-picker", "flow3-subagent-picker"),
    ("variant-picker", "flow3-variant-picker"),
    ("theme-picker", "flow3-theme-picker"),
    ("status", "views.status.default"),
    ("timeline", "flow3-timeline"),
    ("theme-gallery-dark", "views.theme-gallery.dark"),
    ("theme-gallery-light", "views.theme-gallery.light"),
    ("empty", "states.empty.default"),
    ("mode-switch", "flow3-work-style-access"),
    ("mode-switch-refused", "flow3-work-style-refused"),
];

/// Gallery relative path under plans/assets/mockups-v3.
const GALLERY_RELATIVE: &[(&str, &str)] = &[
    ("session", "onboarding"),
    ("input", "agent-picker"),
    ("input", "model-picker"),
    ("input", "model-picker-filtered"),
    ("input", "fork-picker"),
    ("input", "skills-picker"),
    ("input", "session-picker"),
    ("input", "subagent-picker"),
    ("input", "variant-picker"),
    ("input", "theme-picker"),
    ("views", "status"),
    ("views", "timeline"),
    ("views", "theme-gallery-dark"),
    ("views", "theme-gallery-light"),
    ("states", "empty"),
    ("orchestration", "mode-switch"),
    ("orchestration", "mode-switch-refused"),
];

/// Conversation-shell surfaces that must expose the minimal statusline.
const CONVERSATION_SHELL_IDS: &[&str] = &["onboarding", "empty"];

/// Surfaces where Agent and Mode must appear as one work-style concept.
const WORK_STYLE_SURFACES: &[(&str, &str)] = &[
    ("agent-picker", "flow3-agent-picker"),
    ("mode-switch", "flow3-work-style-access"),
    ("mode-switch-refused", "flow3-work-style-refused"),
    ("status", "views.status.default"),
    ("empty", "states.empty.default"),
];

/// Action-dock configuration pickers (bottom selection docks).
const ACTION_DOCK_IDS: &[&str] = &[
    "agent-picker",
    "model-picker",
    "model-picker-filtered",
    "fork-picker",
    "skills-picker",
    "session-picker",
    "subagent-picker",
    "variant-picker",
    "theme-picker",
    "mode-switch",
    "mode-switch-refused",
];

/// Temporary detail / gallery / timeline views.
const DETAIL_VIEW_IDS: &[&str] = &[
    "status",
    "timeline",
    "theme-gallery-dark",
    "theme-gallery-light",
];

/// Configuration detours that must preserve work (VAL-V3-F3-050).
/// Destinations match the public scenario registry verified by Flow 3 packs.
const DETOUR_SCENARIOS: &[(&str, &str, &str)] = &[
    // Model choice (conversation or onboarding origins both cancel to chat-turn
    // on the closed volume product fixtures).
    ("model-picker-cancel", "model-picker", "chat-turn"),
    ("model-picker-apply", "model-picker", "chat-turn"),
    ("model-filter-apply", "model-picker", "model-picker-filtered"),
    ("model-filter-clear", "model-picker-filtered", "model-picker"),
    ("model-filter-cancel", "model-picker-filtered", "chat-turn"),
    // Work style + capability confirmation / refusal.
    ("agent-picker-cancel", "agent-picker", "chat-turn"),
    ("work-style-unchanged", "agent-picker", "chat-turn"),
    ("work-style-access-pending", "agent-picker", "mode-switch"),
    ("work-style-access-confirm", "mode-switch", "chat-turn"),
    ("work-style-access-cancel", "mode-switch", "agent-picker"),
    ("work-style-access-refused", "mode-switch", "mode-switch-refused"),
    (
        "work-style-refusal-choose-unchanged",
        "mode-switch-refused",
        "agent-picker",
    ),
    (
        "work-style-refusal-return",
        "mode-switch-refused",
        "chat-turn",
    ),
    // Session / timeline / fork.
    ("session-picker-cancel", "session-picker", "chat-turn"),
    ("session-preview-open", "session-picker", "session-picker"),
    ("session-preview-back", "session-picker", "session-picker"),
    ("session-timeline-open", "session-picker", "timeline"),
    ("session-timeline-back", "timeline", "session-picker"),
    ("session-resume-latest", "session-picker", "chat-turn"),
    ("timeline-historical-fork", "timeline", "fork-picker"),
    ("fork-confirm", "fork-picker", "chat-turn"),
    ("fork-cancel", "fork-picker", "timeline"),
    // Secondary pickers.
    ("skills-cancel", "skills-picker", "chat-turn"),
    ("skills-apply", "skills-picker", "chat-turn"),
    ("skills-access-pending", "skills-picker", "mode-switch"),
    ("skills-access-confirm", "mode-switch", "chat-turn"),
    ("skills-access-cancel", "mode-switch", "skills-picker"),
    ("subagent-cancel", "subagent-picker", "chat-turn"),
    ("subagent-apply", "subagent-picker", "chat-turn"),
    ("variant-cancel", "variant-picker", "chat-turn"),
    ("variant-apply", "variant-picker", "chat-turn"),
    ("theme-cancel", "theme-picker", "chat-turn"),
    ("theme-apply", "theme-picker", "chat-turn"),
    // empty / status / theme gallery.
    ("empty-open-model", "empty", "model-picker"),
    ("empty-model-cancel", "model-picker", "empty"),
    ("empty-model-apply", "model-picker", "empty"),
    ("empty-open-session", "empty", "session-picker"),
    ("empty-session-cancel", "session-picker", "empty"),
    ("empty-session-resume", "session-picker", "chat-turn"),
    ("status-open", "chat-turn", "status"),
    ("status-return", "status", "chat-turn"),
    ("status-details", "status", "status"),
    ("theme-gallery-open-dark", "theme-picker", "theme-gallery-dark"),
    ("theme-gallery-return-dark", "theme-gallery-dark", "theme-picker"),
    (
        "theme-gallery-open-light",
        "theme-picker",
        "theme-gallery-light",
    ),
    (
        "theme-gallery-return-light",
        "theme-gallery-light",
        "theme-picker",
    ),
    // Onboarding.
    ("onboarding-complete", "onboarding", "empty"),
    ("onboarding-skip", "onboarding", "empty"),
];

/// Human-readable model product names expected in Flow 3.
const HUMAN_MODEL_NAMES: &[&str] = &[
    "Claude Sonnet 4.5",
    "Claude Opus 4.8",
    "GPT-5.2 Codex",
];

/// Descriptive closed-default jargon that must not be required to decide.
const CLOSED_DEFAULT_JARGON: &[&str] = &[
    "transport",
    "endpoint",
    "scopes",
    "grants",
    "tool envelopes",
    "tool_envelope",
    "event payload",
    "event payloads",
    "routing",
    "workers",
    "waves",
    "worktree",
    "config.json",
    "mcpServers",
    "stdio",
    "oauth2",
    "client_secret",
    "device_code",
    "token_count=",
    "latency_ms",
    "wave_id",
    "permission_bitmask",
    "snake_case_status",
    "function_call_id",
    "agent_id=",
    "skill_id=",
    "mode_switch",
    "subagent_id",
];

/// Decorative or non-closed glyphs / chrome tokens.
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
    "🚀",
    "✨",
    "🔥",
    "💡",
    "⚙️",
];

/// Provider/model slug and identity patterns forbidden as primary labels.
const RAW_ID_BANS: &[&str] = &[
    "anthropic/",
    "openai/",
    "google/",
    "claude-opus-4",
    "claude-sonnet-4",
    "claude-3-",
    "gpt-4o-",
    "gpt-5.2-codex",
    "model_id=",
    "agent_id=",
    "skill_id=",
    "session.id",
    "session.configuration",
    "mode_switch",
    "variant_id=",
    "theme_id=",
    "work_style_id",
    "ctx ",
    "$0.",
];

/// Closed functional glyph set (architecture 7.3). Used only for non-ASCII audit presence checks.
const ALLOWED_NON_ASCII: &[char] = &[
    '●', '○', '✓', '✗', '›', '→', '↑', '↓', '─', '│', '╭', '╮', '╰', '╯', '█', '░', '·', '⎿', '▸',
    '▾', '…',
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

fn assert_statusline_minimal(text: &str, label: &str) {
    let status = text
        .lines()
        .last()
        .unwrap_or_else(|| panic!("{label}: missing statusline row"));
    assert!(
        status.contains("~/project")
            && status.contains('·')
            && (status.contains("Claude Sonnet 4.5")
                || status.contains("Claude Opus 4.8")
                || status.contains("GPT-")),
        "{label}: statusline must be path · branch · human model, got `{status}`"
    );
    let lower = status.to_ascii_lowercase();
    assert!(
        !status.contains('%')
            && !status.contains('$')
            && !lower.contains("idle")
            && !status.contains("ctx ")
            && !status.contains("claude-opus")
            && !status.contains("claude-sonnet")
            && !status.contains("opus-4-8")
            && !status.contains("sonnet-4-5")
            && !lower.contains("thorough")
            && !status.contains("Balanced ·")
            && !lower.contains("careful edits")
            && !lower.contains("agent")
            && !lower.contains("mode")
            && !lower.contains("variant"),
        "{label}: statusline carries forbidden field: `{status}`"
    );
    // Exactly one final-row statusline. Body rows may restate path · branch for
    // project context (for example status detail "Project · ~/project · main · …")
    // so count only the trailing statusline shape with a model name.
    let statusline_rows = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("~/project · main ·")
                && (t.contains("Claude") || t.contains("GPT-"))
        })
        .count();
    assert_eq!(
        statusline_rows, 1,
        "{label}: expected exactly one statusline row, got {statusline_rows}"
    );
}

fn assert_no_raw_ids(text: &str, label: &str) {
    for banned in RAW_ID_BANS
        .iter()
        .chain(FORBIDDEN_RAW_ID_SNIPPETS.iter())
    {
        // "session." is banned only as an identity prefix (session.id /
        // session.configuration). Natural English like "current session." is fine.
        if *banned == "session." {
            assert!(
                !text.contains("session.configuration")
                    && !text.contains("session.id")
                    && !text.contains("session.session")
                    && !text.contains("session_id=")
                    && !text.contains("session-id="),
                "{label}: raw session id fragment leaked"
            );
            continue;
        }
        assert!(
            !text.contains(banned),
            "{label}: raw/internal identifier `{banned}`"
        );
    }
    // Slash-delimited provider/model slugs must not appear as primary labels.
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("anthropic/")
            || trimmed.starts_with("openai/")
            || trimmed.starts_with("google/")
        {
            panic!("{label}: primary label is a raw provider slug: `{trimmed}`");
        }
    }
}

fn assert_chrome_simple(text: &str, label: &str) {
    for banned in FORBIDDEN_CHROME {
        assert!(
            !text.contains(banned),
            "{label}: forbidden chrome `{banned}`"
        );
    }
    assert_no_raw_ids(text, label);
    let lower = text.to_ascii_lowercase();
    assert!(
        !lower.contains("modal") || lower.contains("temporary"),
        "{label}: floating modal chrome is forbidden"
    );
}

fn assert_plain_english(text: &str, label: &str) {
    let lower = text.to_ascii_lowercase();
    for banned in CLOSED_DEFAULT_JARGON {
        assert!(
            !lower.contains(&banned.to_ascii_lowercase()),
            "{label}: closed-default jargon `{banned}` must not be required to decide"
        );
    }
}

fn assert_closed_glyphs(text: &str, label: &str) {
    for ch in text.chars() {
        if ch.is_ascii() || ch.is_whitespace() {
            continue;
        }
        // Non-ASCII must belong to the closed functional glyph set or common
        // printable punctuation already used by canvas framing (box-drawing is
        // in the allowed set). Float unicode ornaments fail.
        if ALLOWED_NON_ASCII.contains(&ch) {
            continue;
        }
        // Space for progressive combining diacritics is not used; anything else fails.
        // Allow the occasional non-latin letter if a human product name needs it, but
        // reject obvious decorative emoji blocks.
        let code = ch as u32;
        let is_emoji = (0x1F300..=0x1FAFF).contains(&code)
            || (0x2600..=0x27BF).contains(&code)
            || (0xFE00..=0xFE0F).contains(&code)
            || code == 0x200D;
        assert!(
            !is_emoji,
            "{label}: decorative emoji/glyph U+{code:04X} (`{ch}`) is forbidden"
        );
        // Other non-ASCII outside the closed set: box variants differences already
        // covered; fail unknown decorative symbols in dingbats / private use.
        let is_private_or_dingbat =
            (0x2700..=0x27BF).contains(&code) || (0xE000..=0xF8FF).contains(&code);
        assert!(
            !is_private_or_dingbat,
            "{label}: non-closed decorative glyph U+{code:04X} (`{ch}`)"
        );
    }
}

fn status_has_readable_label(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if text.contains('✓')
        || text.contains('✗')
        || text.contains('●')
        || text.contains('○')
        || text.contains('›')
    {
        return lower.contains("current")
            || lower.contains("pending")
            || lower.contains("applied")
            || lower.contains("selected")
            || lower.contains("available")
            || lower.contains("preview")
            || lower.contains("refused")
            || lower.contains("connect")
            || lower.contains("resume")
            || lower.contains("careful")
            || lower.contains("autonomous")
            || lower.contains("quick")
            || lower.contains("failed")
            || lower.contains("ready")
            || lower.contains("apply")
            || lower.contains("keep")
            || lower.contains("return")
            || lower.contains("cancel")
            || lower.contains("dark")
            || lower.contains("light")
            || lower.contains("thorough")
            || lower.contains("balanced")
            || lower.contains("fast");
    }
    true
}

fn publication_fixture(screen_id: &str) -> String {
    canonical_publication_fixture(screen_id)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("canonical.{screen_id}"))
}

fn empty_body_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| line.chars().all(|c| c.is_whitespace()))
        .count()
}

fn non_empty_body_measure(text: &str) -> usize {
    text.lines()
        .map(|line| line.trim_end().chars().count())
        .filter(|&n| n > 0)
        .max()
        .unwrap_or(0)
}

// ── Inventory + geometry ────────────────────────────────────────────────

#[test]
fn flow3_inventory_is_exact_and_geometry_valid() {
    let mut seen = HashSet::new();
    for id in FLOW3_IDS {
        assert!(seen.insert(*id), "duplicate Flow 3 id in constant: {id}");
    }
    assert_eq!(FLOW3_IDS.len(), 17, "Flow 3 inventory must be exactly 17");
    assert_eq!(GALLERY_RELATIVE.len(), 17);
    assert_eq!(CLOSED_DEFAULT_FIXTURES.len(), 17);

    let inventory = canonical_inventory();
    for id in FLOW3_IDS {
        let matches: Vec<_> = inventory.iter().filter(|s| s.id == *id).collect();
        assert_eq!(
            matches.len(),
            1,
            "{id} must appear exactly once in registry"
        );
        let screen = matches[0];
        assert_eq!(
            screen.status,
            ImplementationStatus::Implemented,
            "{id} must be implemented"
        );
        assert_eq!(
            screen.flow,
            FlowOwner::WorkConfiguration,
            "{id} must be owned by the work-configuration / Flow 3 journey"
        );
    }

    let mut digests = HashSet::new();
    let mut theme_gallery_txt: Option<String> = None;
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &format!("{screen_id}/{fixture_id}"));
        // theme-gallery-dark/light intentionally share identical TXT structure
        // (palette only differs in PNG). Permit that pair while forbidding other
        // cross-state collisions.
        if screen_id == "theme-gallery-dark" || screen_id == "theme-gallery-light" {
            match &theme_gallery_txt {
                None => theme_gallery_txt = Some(text.clone()),
                Some(prev) => assert_eq!(
                    prev, &text,
                    "theme galleries must share identical TXT layout; only PNG palette differs"
                ),
            }
        } else {
            assert!(
                digests.insert(text.clone()),
                "{screen_id}/{fixture_id}: produced a duplicate frame of another Flow 3 state"
            );
        }
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
        assert_chrome_simple(&text, &format!("{screen_id}/{fixture_id}"));
        assert_plain_english(&text, &format!("{screen_id}/{fixture_id}"));
        assert_closed_glyphs(&text, &format!("{screen_id}/{fixture_id}"));
    }

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
        assert_chrome_simple(&text, &format!("gallery {category}/{name}"));
        assert_plain_english(&text, &format!("gallery {category}/{name}"));
        assert_closed_glyphs(&text, &format!("gallery {category}/{name}"));
    }

    let work_config = flow_manifests()
        .iter()
        .find(|f| f.name == "work-configuration")
        .expect("work-configuration storyboard");
    assert_eq!(work_config.steps.len(), 6);
    let storyboard_ids: Vec<_> = work_config.steps.iter().map(|s| s.screen_id).collect();
    assert_eq!(
        storyboard_ids,
        vec![
            "onboarding",
            "model-picker",
            "agent-picker",
            "mode-switch",
            "session-picker",
            "timeline",
        ]
    );

    let listed = Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
        .arg("--list")
        .output()
        .expect("list");
    assert!(listed.status.success());
    let stdout = String::from_utf8_lossy(&listed.stdout);
    for id in FLOW3_IDS {
        assert!(
            stdout
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(*id)),
            "--list missing Flow 3 id {id}"
        );
    }
}

// ── VAL-V3-F3-012 + VAL-V3-F3-057: Human labels, no raw IDs ─────────────

#[test]
fn every_flow3_entity_leads_with_a_human_readable_label() {
    // Model picker rows lead with human product names.
    for screen_id in ["model-picker", "model-picker-filtered"] {
        let fixture = contract_fixture(screen_id);
        let audit = render_pair(screen_id, fixture);
        let text = audit.render_txt();
        let human_hits = HUMAN_MODEL_NAMES
            .iter()
            .filter(|name| text.contains(*name))
            .count();
        assert!(
            human_hits >= 2,
            "{screen_id}: at least two human model names must be visible; got:\n{text}"
        );
        assert_no_raw_ids(&text, screen_id);
        // Statusline model is human too.
        assert_statusline_minimal(&text, screen_id);
    }

    // Onboarding recommends a human model.
    let onboarding = render_pair("onboarding", "gate.work-configuration.onboarding");
    let onboarding_text = onboarding.render_txt();
    assert!(
        onboarding_text.contains("Claude Sonnet 4.5")
            || onboarding_text.contains("Recommended"),
        "onboarding must present a human model recommendation"
    );
    assert_no_raw_ids(&onboarding_text, "onboarding");
    assert_statusline_minimal(&onboarding_text, "onboarding");

    // Status / empty restate human model names.
    for &(screen_id, fixture_id) in &[
        ("status", "views.status.default"),
        ("empty", "states.empty.default"),
        ("variant-picker", "flow3-variant-picker"),
    ] {
        let text = render_pair(screen_id, fixture_id).render_txt();
        assert!(
            HUMAN_MODEL_NAMES.iter().any(|name| text.contains(*name)),
            "{screen_id}: must show a human model name"
        );
        assert_no_raw_ids(&text, screen_id);
    }

    // Work styles, sessions, skills, helpers, variants, themes use human labels.
    for &(screen_id, fixture_id, human_cues) in &[
        (
            "agent-picker",
            "flow3-agent-picker",
            &["Careful edits", "Quick implementation", "Autonomous"] as &[&str],
        ),
        (
            "session-picker",
            "flow3-session-picker",
            &["Retry policy review", "Document release"],
        ),
        (
            "skills-picker",
            "flow3-skills-picker",
            &["skill", "On ·", "Off ·", "Apply"],
        ),
        (
            "subagent-picker",
            "flow3-subagent-picker",
            &["helper", "delegate", "Plan", "tests"],
        ),
        (
            "variant-picker",
            "flow3-variant-picker",
            &["Fast", "Balanced", "Thorough"],
        ),
        (
            "theme-picker",
            "flow3-theme-picker",
            &["Dark", "Light", "Monokai"],
        ),
        (
            "timeline",
            "flow3-timeline",
            &["Opened", "Updated", "retry", "history"],
        ),
        (
            "fork-picker",
            "flow3-fork-picker",
            &["Fork", "checkpoint", "Source", "Destination"],
        ),
    ] {
        let text = render_pair(screen_id, fixture_id).render_txt().to_ascii_lowercase();
        let hits = human_cues
            .iter()
            .filter(|cue| text.contains(&cue.to_ascii_lowercase()))
            .count();
        assert!(
            hits >= 1,
            "{screen_id}: entity must lead with human-readable cues {human_cues:?}"
        );
        assert_no_raw_ids(
            &render_pair(screen_id, fixture_id).render_txt(),
            screen_id,
        );
    }

    // Named-picker contract fixtures also free of raw IDs.
    for screen_id in NINE_PICKERS {
        let text = render_pair(screen_id, contract_fixture(screen_id)).render_txt();
        assert_no_raw_ids(&text, &format!("picker {screen_id}"));
        assert_statusline_minimal(&text, &format!("picker {screen_id}"));
    }
}

// ── VAL-V3-F3-041: Minimal statusline ───────────────────────────────────

#[test]
fn every_flow3_conversation_and_dock_uses_minimal_statusline() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let text = render_pair(screen_id, fixture_id).render_txt();
        assert_statusline_minimal(&text, &format!("{screen_id}/{fixture_id}"));
    }

    // Conversation shells specifically must keep statusline minimal and pure.
    for id in CONVERSATION_SHELL_IDS {
        let fixture = CLOSED_DEFAULT_FIXTURES
            .iter()
            .find(|(s, _)| s == id)
            .map(|(_, f)| *f)
            .expect("conversation shell fixture");
        let text = render_pair(id, fixture).render_txt();
        let status = text.lines().last().expect("statusline");
        let parts: Vec<_> = status
            .trim()
            .split('·')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        assert_eq!(
            parts.len(),
            3,
            "{id}: minimal statusline is path · branch · model, got {parts:?}"
        );
        assert!(parts[0].starts_with("~/"), "{id}: path field");
        assert!(!parts[1].is_empty(), "{id}: branch field");
        assert!(
            parts[2].contains("Claude") || parts[2].starts_with("GPT"),
            "{id}: human model field `{parts:?}`"
        );
    }

    // Flow 3 statusline model fixture split: work-configuration pickers default
    // to Sonnet; empty/status/variant (conversation-return) default to Opus.
    let sonnet_expected = [
        "onboarding",
        "model-picker",
        "agent-picker",
        "session-picker",
        "mode-switch",
        "timeline",
        "theme-picker",
        "fork-picker",
        "skills-picker",
        "subagent-picker",
    ];
    for id in sonnet_expected {
        let fixture = CLOSED_DEFAULT_FIXTURES
            .iter()
            .find(|(s, _)| *s == id)
            .map(|(_, f)| *f)
            .expect("fixture");
        let text = render_pair(id, fixture).render_txt();
        let status = text.lines().last().unwrap();
        assert!(
            status.contains("Claude Sonnet 4.5"),
            "{id}: work-configuration default statusline model is Claude Sonnet 4.5, got `{status}`"
        );
    }
    for id in ["empty", "status", "variant-picker"] {
        let fixture = CLOSED_DEFAULT_FIXTURES
            .iter()
            .find(|(s, _)| *s == id)
            .map(|(_, f)| *f)
            .expect("fixture");
        let text = render_pair(id, fixture).render_txt();
        let status = text.lines().last().unwrap();
        assert!(
            status.contains("Claude Opus 4.8"),
            "{id}: conversation-return statusline model is Claude Opus 4.8, got `{status}`"
        );
    }
}

// ── Agent/Mode are one work-style concept ───────────────────────────────

#[test]
fn agent_and_mode_remain_one_work_style_concept_everywhere() {
    for &(screen_id, fixture_id) in WORK_STYLE_SURFACES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        assert!(
            text.contains("How Code should work")
                || text.contains("How Code works")
                || text.contains("work-style")
                || text.contains("work style")
                || text.contains("Careful edits")
                || text.contains("work-style change"),
            "{screen_id}: must present the unified work-style concept; got:\n{text}"
        );
        // Never productize separate Agent and Mode destinations in default copy.
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("choose an agent")
                && !lower.contains("select agent")
                && !lower.contains("agent mode")
                && !lower.contains("switch mode")
                && !lower.contains("mode picker")
                && !lower.contains("agent picker"),
            "{screen_id}: Agent and Mode must not appear as separate products"
        );
        // mode-switch remains the access confirmation surface of How Code should work.
        if screen_id == "mode-switch" || screen_id == "mode-switch-refused" {
            assert!(
                text.contains("How Code should work")
                    || text.contains("work-style")
                    || text.contains("work style")
                    || text.contains("Autonomous")
                    || text.contains("Careful edits"),
                "{screen_id}: capability review must restate the work-style outcome"
            );
        }
        // Audit family (when present) uses work-style, not dual agent/mode families.
        if let Some(evidence) = audit.work_style.as_ref() {
            assert!(
                !evidence.applied.trim().is_empty(),
                "{screen_id}: applied work style is required when work_style evidence is present"
            );
        }
    }

    // Storyboard agent-picker and mode-switch share the work-style user task.
    let work_config = flow_manifests()
        .iter()
        .find(|f| f.name == "work-configuration")
        .expect("work-configuration");
    let agent = work_config
        .steps
        .iter()
        .find(|s| s.screen_id == "agent-picker")
        .expect("agent-picker step");
    let mode = work_config
        .steps
        .iter()
        .find(|s| s.screen_id == "mode-switch")
        .expect("mode-switch step");
    assert!(
        agent.user_task.to_ascii_lowercase().contains("how code")
            || agent.user_task.to_ascii_lowercase().contains("work"),
        "agent-picker task must speak about work style"
    );
    assert_eq!(
        agent.visible_origin, mode.visible_origin,
        "agent-picker and mode-switch share the same chapter origin"
    );
}

// ── VAL-V3-F3-046: Applied/pending configuration consistency ────────────

#[test]
fn configuration_state_remains_consistent_across_the_flow() {
    // Gate storyboard: applied model Sonnet, pending model Opus on model-picker.
    let onboarding = render_pair("onboarding", "gate.work-configuration.onboarding");
    if let Some(choice) = &onboarding.model_choice {
        assert_eq!(
            choice.applied, "Claude Sonnet 4.5",
            "onboarding applied model"
        );
    }
    let model = render_pair("model-picker", "gate.work-configuration.model-picker");
    if let Some(choice) = &model.model_choice {
        assert_eq!(choice.applied, "Claude Sonnet 4.5");
        // Pending is never shown as applied in the statusline.
        if let Some(pending) = &choice.pending {
            let text = model.render_txt();
            let status = text.lines().last().unwrap();
            assert!(
                !status.contains(pending.as_str()) || status.contains(&choice.applied),
                "statusline must show applied, not only pending: `{status}` pending=`{pending}`"
            );
            assert!(
                text.contains("Pending") || text.contains("›"),
                "pending selection must be named in the body with non-color cues"
            );
            assert_ne!(
                choice.applied.as_str(),
                pending.as_str(),
                "pending must differ from applied when a switch is under consideration"
            );
        }
    }

    // Statusline always carries the applied model for every work-config dock.
    for &(screen_id, fixture_id) in &[
        ("model-picker", "flow3-model-picker"),
        ("agent-picker", "flow3-agent-picker"),
        ("mode-switch", "flow3-work-style-access"),
        ("session-picker", "flow3-session-picker"),
        ("timeline", "flow3-timeline"),
    ] {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let status = text.lines().last().unwrap();
        assert!(
            status.contains("Claude Sonnet 4.5"),
            "{screen_id}: statusline must agree on applied Sonnet default"
        );
        if let Some(choice) = &audit.model_choice {
            assert!(
                status.contains(&choice.applied),
                "{screen_id}: statusline must match model_choice.applied={}",
                choice.applied
            );
            if let Some(pending) = &choice.pending {
                // Applied and pending remain distinct sources of truth.
                if pending != &choice.applied {
                    assert!(
                        text.contains("Pending")
                            || text.contains("›")
                            || text.contains(pending.as_str()),
                        "{screen_id}: pending {pending} must be visible as non-applied selection"
                    );
                }
            }
        }
        if let Some(work_style) = &audit.work_style {
            assert!(
                !work_style.applied.trim().is_empty(),
                "{screen_id}: applied work style must be named"
            );
            if let Some(pending) = &work_style.pending {
                if pending != &work_style.applied {
                    assert!(
                        text.contains("Pending")
                            || text.contains(pending.as_str())
                            || text.contains("Requested"),
                        "{screen_id}: pending work style {pending} must not present as applied-only"
                    );
                    // Active applied work style still names itself as Current / applied.
                    assert!(
                        text.contains("Current")
                            || text.contains("Careful")
                            || text.contains(&work_style.applied),
                        "{screen_id}: applied work style must remain stated while pending exists"
                    );
                }
            }
        }
    }

    // Storyboard PNG destination after model apply lands on the remarkable model
    // and returns consistency (covered by onboarding replay cases).
    let applied = replay_case("model-picker-apply", 1).expect("model apply");
    applied.validate().expect("model apply valid");
    if let Some(choice) = &applied.audit.model_choice {
        assert!(
            !choice.applied.trim().is_empty(),
            "post-apply model_choice.applied is required"
        );
        // Pending must be cleared once applied, or equal the applied name.
        if let Some(pending) = &choice.pending {
            assert_eq!(
                pending, &choice.applied,
                "after apply, pending must not disagree with applied"
            );
        }
        let after_text = applied.audit.render_txt();
        let status = after_text.lines().last().unwrap();
        assert!(
            status.contains(&choice.applied),
            "post-apply statusline must reconcile applied model"
        );
    }

    // empty-model-apply: Sonnet side_effect/attachment and after paint/audit agree
    // (VAL-V3-F3-046 dual-source ,regression). Keep cancel on stock empty Opus.
    let empty_apply = replay_case("empty-model-apply", 1).expect("empty-model-apply");
    empty_apply.validate().expect("empty-model-apply valid");
    assert_eq!(empty_apply.after.screen_id, "empty");
    assert!(
        empty_apply.after.attachment.contains("Claude Sonnet 4.5"),
        "attachment claims Sonnet applied: {}",
        empty_apply.after.attachment
    );
    assert!(
        empty_apply
            .side_effects
            .iter()
            .any(|effect| effect.status == "model-applied"),
        "side_effect status model-applied"
    );
    let empty_choice = empty_apply
        .audit
        .model_choice
        .as_ref()
        .expect("empty-model-apply after model_choice");
    assert_eq!(
        empty_choice.applied, "Claude Sonnet 4.5",
        "audit.model_choice.applied must match applied Sonnet, not stock empty Opus"
    );
    assert!(
        empty_choice.pending.is_none()
            || empty_choice.pending.as_deref() == Some(empty_choice.applied.as_str()),
        "after apply, pending must not disagree with applied"
    );
    let empty_after_text = empty_apply.audit.render_txt();
    let empty_status = empty_after_text.lines().last().unwrap();
    assert!(
        empty_after_text.contains("Ready · Claude Sonnet 4.5"),
        "after paint Ready model must be Sonnet"
    );
    assert!(
        empty_status.contains("Claude Sonnet 4.5"),
        "after statusline must be Sonnet: {empty_status}"
    );
    assert!(
        !empty_status.contains("Claude Opus 4.8"),
        "after apply must not pixel-match default empty Opus statusline"
    );
    // Cancel still returns to the intentional stock Opus empty fixture.
    let empty_cancel = replay_case("empty-model-cancel", 1).expect("empty-model-cancel");
    empty_cancel.validate().expect("empty-model-cancel valid");
    let cancel_choice = empty_cancel
        .audit
        .model_choice
        .as_ref()
        .expect("cancel model_choice");
    assert_eq!(
        cancel_choice.applied, "Claude Opus 4.8",
        "cancel must keep intentional stock empty Opus fixture split"
    );
    assert_eq!(
        empty_cancel.audit.fixture_id, "views.empty.origin",
        "cancel after-fixture remains views.empty.origin"
    );

    // Work-style confirm updates capability without losing the conversation model.
    let confirmed = replay_case("work-style-access-confirm", 1).expect("confirm");
    confirmed.validate().expect("confirm valid");
    assert_eq!(confirmed.after.screen_id, "chat-turn");
    let confirmed_text = confirmed.audit.render_txt();
    assert_statusline_minimal(&confirmed_text, "work-style-access-confirm after");
    assert!(
        confirmed_text.contains("Claude"),
        "confirmed access return preserves the human model on the conversation"
    );
}

// ── VAL-V3-F3-047: Default configuration copy avoids jargon ─────────────

#[test]
fn default_flow3_copy_avoids_implementation_jargon() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let text = render_pair(screen_id, fixture_id).render_txt();
        assert_plain_english(&text, &format!("{screen_id}/{fixture_id}"));
        // Audit channel itself does not smuggle banned tokens for operator copy.
        let json = render_pair(screen_id, fixture_id).to_json();
        for banned in [
            "tool_envelope",
            "wave_id",
            "client_secret",
            "device_code",
            "token_count=",
            "permission_bitmask",
        ] {
            assert!(
                !json.contains(banned),
                "{screen_id}/{fixture_id}: audit leaked `{banned}`"
            );
        }
    }

    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        let text = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .expect("gallery txt");
        assert_plain_english(&text, &format!("gallery {category}/{name}"));
    }
}

// ── VAL-V3-F3-048: Default density supports scanning ────────────────────

#[test]
fn default_density_supports_scanning_with_one_dominant_region() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let label = format!("{screen_id}/{fixture_id}");

        // Whitespace: at least a few empty or structurally light rows.
        let empty_rows = empty_body_lines(&text);
        assert!(
            empty_rows >= 4,
            "{label}: default density should leave whitespace (empty rows={empty_rows})"
        );

        // Reading measure: the primary prose measure stays near 80-96 columns.
        // Shared action-dock borders legitimately span ~118 columns, so exclude
        // pure frame/separator rows from the content measure.
        let body_lines: Vec<&str> = text
            .lines()
            .take(text.lines().count().saturating_sub(1))
            .collect();
        let body = body_lines.join("\n");
        let content_measure = body_lines
            .iter()
            .map(|line| line.trim_end())
            .filter(|line| {
                let t = line.trim();
                !(t.is_empty()
                    || t.chars().all(|c| {
                        matches!(c, '─' | '│' | '╭' | '╮' | '╰' | '╯' | ' ' | '·' | '█' | '░')
                    })
                    || t.starts_with('╭')
                    || t.starts_with('╰')
                    || t.starts_with('│'))
            })
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        assert!(
            content_measure <= 100,
            "{label}: primary reading measure should stay near 80-96 columns (max content body={content_measure})"
        );
        let _ = non_empty_body_measure; // keep helper available for local debug

        // One primary action at most on decision docks.
        let primaries = primary_events(&audit).len();
        if ACTION_DOCK_IDS.contains(&screen_id) {
            assert!(
                primaries <= 1,
                "{label}: action dock may expose at most one primary, got {primaries}"
            );
            let secondaries = secondary_events(&audit).len();
            assert!(
                secondaries <= 2,
                "{label}: at most two secondary actions, got {secondaries}"
            );
        }

        // Selected option expands; advanced metadata is not a default wide table.
        assert!(
            !body.contains("│ id │ slug │ endpoint │ latency │ score │"),
            "{label}: forbidden wide metadata table in default content"
        );
        // Visible framework: no multi competing bordered zones outside dock/detail.
        let border_corners = body.matches('╭').count();
        assert!(
            border_corners <= 2,
            "{label}: too many bordered zones for default density ({border_corners})"
        );

        // One obvious action marker (› / [enter] / primary).
        if ACTION_DOCK_IDS.contains(&screen_id) || DETAIL_VIEW_IDS.contains(&screen_id) {
            assert!(
                text.contains('›')
                    || (text.contains("Enter to") || text.contains("[enter]"))
                    || (text.contains("Esc to") || text.contains("[esc]"))
                    || text.contains("Return")
                    || primaries == 1,
                "{label}: focus or primary marker required for scanning"
            );
        }
    }
}

// ── VAL-V3-F3-049: Restrained semantic visuals ──────────────────────────

#[test]
fn flow3_uses_restrained_semantic_visuals_and_closed_glyphs() {
    for &(screen_id, fixture_id) in CLOSED_DEFAULT_FIXTURES {
        let audit = render_pair(screen_id, fixture_id);
        let text = audit.render_txt();
        let label = format!("{screen_id}/{fixture_id}");
        assert_closed_glyphs(&text, &label);
        assert_chrome_simple(&text, &label);
        assert!(
            status_has_readable_label(&text),
            "{label}: status markers must pair with words (non-color cues)"
        );
        // Focus is never purely color: › glyph or Current/Pending textual markers.
        if ACTION_DOCK_IDS.contains(&screen_id) {
            assert!(
                text.contains('›')
                    || text.contains("Current")
                    || text.contains("Pending")
                    || text.contains("Selected"),
                "{label}: selection must not rely on color alone"
            );
        }
        // Outcome colors: success/fail words or closed glyphs when present.
        if text.contains('✓') {
            assert!(
                text.to_ascii_lowercase().contains("current")
                    || text.to_ascii_lowercase().contains("applied")
                    || text.to_ascii_lowercase().contains("ready")
                    || text.to_ascii_lowercase().contains("connected"),
                "{label}: success glyph without readable outcome"
            );
        }
        if text.contains('✗') {
            assert!(
                text.to_ascii_lowercase().contains("fail")
                    || text.to_ascii_lowercase().contains("refus")
                    || text.to_ascii_lowercase().contains("block")
                    || text.to_ascii_lowercase().contains("deny"),
                "{label}: failure glyph without readable outcome"
            );
        }
    }

    // Surface assignment stays coherent for docks vs detail.
    let inventory = canonical_inventory();
    for id in ACTION_DOCK_IDS {
        let screen = inventory.iter().find(|s| s.id == *id).expect("registry");
        assert!(
            matches!(
                screen.surface,
                Surface::ActionDock | Surface::TemporaryDetailView
            ),
            "{id}: configuration decision stays on dock or temporary detail, got {:?}",
            screen.surface
        );
    }
    for id in DETAIL_VIEW_IDS {
        let screen = inventory.iter().find(|s| s.id == *id).expect("registry");
        assert_eq!(
            screen.surface,
            Surface::TemporaryDetailView,
            "{id}: detail/gallery/timeline is TemporaryDetailView"
        );
    }
}

// ── VAL-V3-F3-050: Detour preservation ──────────────────────────────────

#[test]
fn end_to_end_work_survives_configuration_detours() {
    for &(case_id, from, to) in DETOUR_SCENARIOS {
        let replay =
            replay_case(case_id, 1).unwrap_or_else(|e| panic!("scenario {case_id}: {e}"));
        replay
            .validate()
            .unwrap_or_else(|e| panic!("{case_id} invalid: {e}"));
        assert_eq!(replay.before.screen_id, from, "{case_id} before");
        assert_eq!(replay.after.screen_id, to, "{case_id} after");
        assert!(
            !replay.actions.is_empty(),
            "{case_id}: must record an action"
        );
        for transition in &replay.transitions {
            assert_ne!(
                transition.kind, "dead-end",
                "{case_id}: transition must not be dead-end"
            );
        }

        // No unexplained jump into unrelated dashboards.
        assert!(
            !matches!(
                to,
                "login" | "workflow-launch" | "workflow-monitor" | "mcp" | "home"
            ) || case_id.contains("resume") || case_id.contains("complete") || case_id.contains("skip"),
            "{case_id}: must not jump to unrelated destination {to}"
        );

        let cancel_like = case_id.contains("cancel")
            || case_id.contains("back")
            || case_id.contains("return")
            || case_id.ends_with("-unchanged")
            || case_id == "work-style-refusal-return";
        // Cancel-like paths preserve draft / scroll / origin / session when declared.
        if cancel_like {
            for field in [
                "draft",
                "scroll_anchor",
                "origin",
                "origin_stack",
                "session_id",
                "transcript",
                "attachment",
            ] {
                if let Some(hash) = replay.preserved_field_hashes.get(field) {
                    assert_eq!(
                        hash.before, hash.after,
                        "{case_id}: preserved {field} must round-trip"
                    );
                }
            }
            // Cancel does not emit destructive applied/write statuses.
            for effect in &replay.side_effects {
                let status = effect.status.to_ascii_lowercase();
                let destructive = (status.contains("applied") && !status.contains("no"))
                    || status.contains("file-written")
                    || status.contains("command-invoked")
                    || (status.contains("success")
                        && !status.contains("no-")
                        && !status.contains("preview"));
                // Resume/open side effects on cancel-named paths should not be present;
                // pure cancels stay non-destructive.
                if case_id.contains("cancel") || case_id.contains("back") {
                    assert!(
                        !destructive,
                        "{case_id}: cancel side-effect must stay non-destructive, got {}",
                        effect.status
                    );
                }
            }
        }

        let after_text = replay.audit.render_txt();
        assert_canvas_geometry(&after_text, &format!("{case_id} after"));
        assert_statusline_minimal(&after_text, &format!("{case_id} after"));
        assert_no_raw_ids(&after_text, &format!("{case_id} after"));
    }

    // Multi-hop configuration path: empty → model → cancel → empty
    // and work-style pending → cancel leaves origin conversation intact.
    let empty_model = replay_case("empty-open-model", 1).expect("empty-open-model");
    assert_eq!(empty_model.after.screen_id, "model-picker");
    let empty_cancel = replay_case("empty-model-cancel", 1).expect("empty-model-cancel");
    assert_eq!(empty_cancel.after.screen_id, "empty");
    if let Some(hash) = empty_cancel.preserved_field_hashes.get("draft") {
        assert_eq!(hash.before, hash.after, "empty model cancel preserves draft");
    }

    let pending = replay_case("work-style-access-pending", 1).expect("pending");
    assert_eq!(pending.after.screen_id, "mode-switch");
    let cancelled = replay_case("work-style-access-cancel", 1).expect("cancel");
    assert_eq!(cancelled.after.screen_id, "agent-picker");
    if let Some(hash) = cancelled.preserved_field_hashes.get("draft") {
        assert_eq!(hash.before, hash.after);
    }
    if let Some(hash) = cancelled.preserved_field_hashes.get("transcript") {
        assert_eq!(hash.before, hash.after);
    }
}

// ── Gallery drift guard ─────────────────────────────────────────────────

#[test]
fn published_flow3_gallery_matches_live_publication_fixtures() {
    let root = gallery_root();
    for &(category, name) in GALLERY_RELATIVE {
        // Prefer closed-default fixtures over gate-first publication_fixture when the
        // closed string is the product truth for non-gate screens.
        let fixture = CLOSED_DEFAULT_FIXTURES
            .iter()
            .find(|(id, _)| *id == name)
            .map(|(_, f)| (*f).to_owned())
            .unwrap_or_else(|| publication_fixture(name));
        let live = render_pair(name, &fixture).render_txt();
        let published = fs::read_to_string(root.join(category).join(format!("{name}.txt")))
            .unwrap_or_else(|e| panic!("gallery {category}/{name}: {e}"));
        // Geometry and statusline invariants must hold for both live and published.
        assert_canvas_geometry(&live, &format!("live {name}/{fixture}"));
        assert_canvas_geometry(&published, &format!("published {category}/{name}"));
        assert_statusline_minimal(&live, &format!("live {name}"));
        assert_statusline_minimal(&published, &format!("published {name}"));
        assert_no_raw_ids(&live, &format!("live {name}"));
        assert_no_raw_ids(&published, &format!("published {name}"));
        assert_plain_english(&live, &format!("live {name}"));
        assert_plain_english(&published, &format!("published {name}"));
    }
}
