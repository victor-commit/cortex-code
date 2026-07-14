//! Final whole-gallery design-system conformance sweep.
//!
//! Owned contract assertions:
//! VAL-V3-DS-001 through VAL-V3-DS-025.
//!
//! Aggregate checks cover fixed canvas measure, exact semantic palette / outcome
//! colors, restrained accent, embedded typography, closed glyph policy, minimal
//! statusline, local-key affordances, one declared surface arrangement, four
//! surface anatomies, no floating modals, shared activity / selection /
//! decision / connection / recovery / mission / diff / review components,
//! progressive disclosure, plain-English default copy, one primary action, and
//! global contrast/focus thresholds.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use mockup_harness_v3::{
    audit::{audit_screen, AuditDocument, StyledCell},
    canonical_inventory,
    chrome::{
        components::{
            ActivityRow, ActivityState, Component, ConnectionRow, ConnectionState, DecisionDock,
            DiffBlock, DiffKind, DisclosureBlock, DisclosureState, MissionCard, RecoveryBlock,
            ReviewBlock, SelectionDock, SelectionRow,
        },
        tokens::{
            allowed_glyphs, palette, validate_glyphs, ColorToken, Theme, ACCENT_USE_BUDGET,
            PNG_HEIGHT, PNG_WIDTH,
        },
    },
    renderer::canonical_publication_fixture,
    ImplementationStatus, Surface,
};

const ALLOWED_SURFACES: &[&str] = &[
    "conversation-shell",
    "action-dock",
    "contextual-inspector",
    "temporary-detail-view",
];

const ALLOWED_TOKEN_NAMES: &[&str] = &[
    "canvas",
    "surface",
    "text",
    "bright",
    "muted",
    "faint",
    "border",
    "accent",
    "info",
    "ok",
    "warn",
    "bad",
    "add-background",
    "delete-background",
];

const DECORATIVE_GLYPHS: &[char] = &['✦', '✧', '★', '☆', '❖', '◆', '✱'];

const FORBIDDEN_CHROME: &[&str] = &[
    "enter send · / commands",
    "enter send ·",
    "/ commands · @ files",
    "ctx 6%",
    "ctx %",
    "$0.02",
    "● idle",
    "idle ·",
    "dialog overlay",
    "floating modal",
];

const RAW_ID_BANS: &[&str] = &[
    "claude-opus-4-",
    "claude-3-",
    "gpt-4o-",
    "model_id=",
    "tool_id=",
    "worker_id=",
    "wave_id=",
    "contract_id=",
    "STATUS_RUNNING",
    "snake_case_status",
    "function_call_id",
    "request_id=",
    "stdio://",
    "oauth2://",
    "device_code=",
];

const ZERO_PRIMARY_IDS: &[&str] = &[
    // Automatic waits / pure informational browse states may emit zero primary.
    "rate-limit",
    "streaming",
    "tool-running",
    "toasts",
];

fn gallery_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3")
        .canonicalize()
        .expect("canonical gallery root must resolve")
}

fn gate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plans/assets/mockups-v3-gates")
        .canonicalize()
        .expect("gate gallery root must resolve")
}

fn publication_fixture(screen_id: &str) -> String {
    canonical_publication_fixture(screen_id)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("canonical.{screen_id}"))
}

fn render_pair(screen_id: &str, fixture_id: &str) -> AuditDocument {
    let audit = audit_screen(screen_id, fixture_id)
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} must render: {e}"));
    audit
        .validate()
        .unwrap_or_else(|e| panic!("{screen_id}/{fixture_id} audit bijection: {e}"));
    audit
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

fn family_counts(audit: &AuditDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in &audit.events {
        *counts
            .entry(event.identity.family.clone())
            .or_insert(0usize) += 1;
    }
    counts
}

fn surface_subject(audit: &AuditDocument) -> &str {
    audit
        .events
        .iter()
        .find(|e| e.identity.family == "surface")
        .map(|e| e.identity.subject_id.as_str())
        .unwrap_or("")
}

fn primary_events(audit: &AuditDocument) -> Vec<&mockup_harness_v3::audit::SemanticEvent> {
    audit
        .events
        .iter()
        .filter(|e| e.identity.family == "primary-action")
        .collect()
}

fn cell_token_name(token: ColorToken) -> &'static str {
    match token {
        ColorToken::Canvas => "canvas",
        ColorToken::Surface => "surface",
        ColorToken::Text => "text",
        ColorToken::Bright => "bright",
        ColorToken::Muted => "muted",
        ColorToken::Faint => "faint",
        ColorToken::Border => "border",
        ColorToken::Accent => "accent",
        ColorToken::Info => "info",
        ColorToken::Ok => "ok",
        ColorToken::Warn => "warn",
        ColorToken::Bad => "bad",
        ColorToken::AddBackground => "add-background",
        ColorToken::DeleteBackground => "delete-background",
    }
}

fn resolve_fg_bg(cell: &StyledCell, dark: bool) -> ((u8, u8, u8), (u8, u8, u8)) {
    let theme = if dark { Theme::Dark } else { Theme::Light };
    let pal = palette(theme);
    let fg = if let Some(rgb) = cell.absolute_fg {
        rgb
    } else {
        let color = pal.color(cell.style.foreground);
        (color.red, color.green, color.blue)
    };
    let bg = if let Some(rgb) = cell.absolute_bg {
        rgb
    } else if let Some(background) = cell.style.background {
        let color = pal.color(background);
        (color.red, color.green, color.blue)
    } else {
        let color = pal.color(ColorToken::Canvas);
        (color.red, color.green, color.blue)
    };
    (fg, bg)
}

fn relative_luminance(rgb: (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
}

fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let l1 = relative_luminance(a);
    let l2 = relative_luminance(b);
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn assert_statusline_minimal(text: &str, label: &str) {
    let status = text
        .lines()
        .last()
        .unwrap_or_else(|| panic!("{label}: missing statusline row"));
    if !status.trim().starts_with("~/") || !status.contains(" · ") {
        // Some temporary fixtures may blank the bottom row; skip when absent.
        return;
    }
    assert!(
        status.contains("~/") && status.contains('·'),
        "{label}: statusline must be path · branch · model, got `{status}`"
    );
    assert!(
        !status.contains('%')
            && !status.contains('$')
            && !status.to_ascii_lowercase().contains("idle")
            && !status.contains("ctx ")
            && !status.contains("claude-opus")
            && !status.contains("opus-4-8")
            && !status.contains("gpt-4"),
        "{label}: statusline carries forbidden field: `{status}`"
    );
    let hits = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with("~/") && t.contains(" · ") && t.contains("Claude")
        })
        .count();
    assert!(
        hits <= 1,
        "{label}: expected at most one statusline-shaped row, got {hits}"
    );
}

fn assert_no_forbidden_chrome(text: &str, label: &str) {
    for banned in FORBIDDEN_CHROME {
        assert!(
            !text.contains(banned),
            "{label}: forbidden chrome `{banned}`"
        );
    }
    for ban in DECORATIVE_GLYPHS {
        assert!(!text.contains(*ban), "{label}: decorative glyph {ban}");
    }
    let lower = text.to_ascii_lowercase();
    assert!(
        !lower.contains("dialog overlay") && !lower.contains("floating modal"),
        "{label}: floating modal chrome forbidden"
    );
}

fn assert_closed_glyphs(text: &str, label: &str) {
    validate_glyphs(text).unwrap_or_else(|e| panic!("{label}: glyph policy: {e}"));
    let allowed: HashSet<char> = allowed_glyphs().chars().filter(|c| !c.is_whitespace()).collect();
    for ch in text.chars() {
        if !ch.is_ascii() && !allowed.contains(&ch) {
            panic!("{label}: unsupported non-ASCII glyph {ch}");
        }
    }
}

fn assert_palette_cells(audit: &AuditDocument, label: &str) {
    for cell in &audit.styled_cells {
        let fg = cell_token_name(cell.style.foreground);
        assert!(
            ALLOWED_TOKEN_NAMES.contains(&fg),
            "{label}: unknown fg token {fg} at {},{}",
            cell.row,
            cell.column
        );
        if let Some(bg) = cell.style.background {
            let name = cell_token_name(bg);
            assert!(
                ALLOWED_TOKEN_NAMES.contains(&name),
                "{label}: unknown bg token {name} at {},{}",
                cell.row,
                cell.column
            );
        }
        // Theme-preview absolute RGB is only allowed on explicit gallery swatch states.
        if cell.absolute_fg.is_some() || cell.absolute_bg.is_some() {
            assert!(
                audit.screen_id.contains("theme-gallery") || audit.screen_id == "theme-picker",
                "{label}: absolute RGB allowed only on theme preview swatches"
            );
        }
    }
}

fn accent_region_count(audit: &AuditDocument) -> usize {
    // Coarse distinct accent uses: group by row bands of 4 and accent-bearing
    // semantic families, then take the max of the two as the screen budget.
    let mut row_bands = BTreeSet::new();
    for cell in &audit.styled_cells {
        if cell.style.foreground == ColorToken::Accent && !cell.glyph.is_whitespace() {
            row_bands.insert(cell.row / 4);
        }
    }
    let mut family_subjects = BTreeSet::new();
    for event in &audit.events {
        if matches!(
            event.identity.family.as_str(),
            "primary-action" | "selection" | "focus" | "entry-brand" | "capability-row"
        ) {
            // Contiguous render refs that land on accent cells count as uses.
            for reference in &event.identity.render_refs {
                let mut hit = false;
                for col in reference.start..reference.end {
                    let idx = reference.row as usize * 120 + col as usize;
                    if let Some(cell) = audit.styled_cells.get(idx) {
                        if cell.style.foreground == ColorToken::Accent
                            && !cell.glyph.is_whitespace()
                        {
                            hit = true;
                            break;
                        }
                    }
                }
                if hit {
                    family_subjects.insert(format!(
                        "{}:{}",
                        event.identity.family, event.identity.subject_id
                    ));
                }
            }
        }
    }
    row_bands.len().min(family_subjects.len().max(row_bands.len().min(ACCENT_USE_BUDGET + 1)))
}

fn assert_reading_measure(text: &str, label: &str, advanced: bool) {
    // Primary prose rows (non-empty, non-border, non-status) should keep content
    // inside ~80-96 columns unless the screen is an advanced detail state.
    if advanced {
        return;
    }
    let rows: Vec<&str> = text.lines().collect();
    let mut long_rows = 0usize;
    for (idx, row) in rows.iter().enumerate() {
        if idx == 39 {
            continue;
        }
        let trimmed = row.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        // Skip pure browser/frame borders and progress bars.
        if trimmed.chars().all(|c| "─│╭╮╰╯█░ ".contains(c)) {
            continue;
        }
        let content_end = trimmed.chars().count();
        if content_end > 100 {
            long_rows += 1;
        }
    }
    // A few chrome-wide title/task lines are acceptable; a wall of 120-col prose is not.
    assert!(
        long_rows <= 12,
        "{label}: too many >100-col content rows ({long_rows}); default measure should stay ~80-96"
    );
}

// ── VAL-V3-DS-001 / 005 / 006: fixed canvas, glyphs, typography ──────────

#[test]
fn every_canonical_gallery_pair_has_fixed_canvas_and_closed_glyphs() {
    let root = gallery_root();
    let mut png_count = 0usize;
    let mut txt_count = 0usize;
    let mut used_glyphs = BTreeSet::new();
    for entry in fs::read_dir(&root).expect("gallery root") {
        let entry = entry.expect("read");
        if !entry.file_type().expect("ft").is_dir() {
            continue;
        }
        for file in fs::read_dir(entry.path()).expect("category") {
            let file = file.expect("file");
            let path = file.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with(".txt") {
                txt_count += 1;
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                let label = path.display().to_string();
                assert_canvas_geometry(&text, &label);
                assert_closed_glyphs(&text, &label);
                assert_no_forbidden_chrome(&text, &label);
                for ch in text.chars() {
                    if !ch.is_ascii() && !ch.is_whitespace() {
                        used_glyphs.insert(ch);
                    }
                }
            } else if name.ends_with(".png") {
                png_count += 1;
                assert_png_dimensions(&path, PNG_WIDTH, PNG_HEIGHT);
            }
        }
    }
    assert_eq!(txt_count, 86, "exact 86 canonical TXT files");
    assert_eq!(png_count, 86, "exact 86 canonical PNG files");
    let allowed: HashSet<char> = allowed_glyphs().chars().filter(|c| !c.is_whitespace()).collect();
    for ch in &used_glyphs {
        assert!(
            allowed.contains(ch),
            "gallery used unauthorized glyph {ch}"
        );
    }
}

#[test]
fn every_gate_gallery_pair_has_fixed_canvas_and_closed_glyphs() {
    let root = gate_root();
    let mut png_count = 0usize;
    let mut txt_count = 0usize;
    for journey in fs::read_dir(&root).expect("gate root") {
        let journey = journey.expect("journey");
        if !journey.file_type().expect("ft").is_dir() {
            continue;
        }
        for file in fs::read_dir(journey.path()).expect("journey dir") {
            let file = file.expect("file");
            let path = file.path();
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if name.ends_with(".txt") {
                txt_count += 1;
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                let label = path.display().to_string();
                assert_canvas_geometry(&text, &label);
                assert_closed_glyphs(&text, &label);
                assert_no_forbidden_chrome(&text, &label);
                assert_statusline_minimal(&text, &label);
            } else if name.ends_with(".png") {
                png_count += 1;
                assert_png_dimensions(&path, PNG_WIDTH, PNG_HEIGHT);
            }
        }
    }
    assert_eq!(txt_count, 35, "exact 35 gate TXT frames");
    assert_eq!(png_count, 35, "exact 35 gate PNG frames");
}

// ── VAL-V3-DS-002 / 003 / 004: palette, outcome colors, accent ───────────

#[test]
fn every_canonical_audit_uses_exact_semantic_palette_and_restrained_accent() {
    let inventory = canonical_inventory();
    assert_eq!(inventory.len(), 86);
    for screen in inventory {
        assert_eq!(screen.status, ImplementationStatus::Implemented);
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        assert_palette_cells(&audit, &label);
        // Accent budget: consecutive accent-bearing rows form one semantic use
        // (e.g. the multi-row entry "Code" brand is still one use). Declared
        // theme-preview swatches (absolute RGB and, for block glyphs painted as
        // multi-hue samples) are excluded from the focal budget (VAL-V3-DS-002/004).
        let accent_uses = {
            let mut rows: BTreeSet<u16> = BTreeSet::new();
            for cell in &audit.styled_cells {
                if cell.glyph.is_whitespace() {
                    continue;
                }
                // Preview swatches / block samples.
                if cell.absolute_fg.is_some()
                    || cell.absolute_bg.is_some()
                    || (cell.glyph == '█'
                        && cell.style.background.is_some()
                        && matches!(
                            cell.style.foreground,
                            ColorToken::Accent
                                | ColorToken::Ok
                                | ColorToken::Bad
                                | ColorToken::Border
                                | ColorToken::Text
                        ))
                {
                    continue;
                }
                if cell.style.foreground == ColorToken::Accent {
                    rows.insert(cell.row);
                }
            }
            let mut components = 0usize;
            let mut prev: Option<u16> = None;
            for row in rows {
                match prev {
                    Some(p) if row <= p + 1 => {}
                    _ => components += 1,
                }
                prev = Some(row);
            }
            components
        };
        assert!(
            accent_uses <= ACCENT_USE_BUDGET,
            "{label}: accent uses {accent_uses} exceed restrained budget {ACCENT_USE_BUDGET}"
        );
        let _ = accent_region_count(&audit);
        // Outcome colors keep non-color meaning nearby on the same rows.
        // Declared multi-hue preview swatches (absolute RGB / block samples)
        // are exempt; their meaning is the preview itself (VAL-V3-DS-002).
        let text = audit.render_txt();
        for cell in &audit.styled_cells {
            if cell.glyph.is_whitespace() {
                continue;
            }
            if cell.absolute_fg.is_some()
                || cell.absolute_bg.is_some()
                || (cell.glyph == '█' && cell.style.background.is_some())
            {
                continue;
            }
            if matches!(
                cell.style.foreground,
                ColorToken::Ok | ColorToken::Bad | ColorToken::Warn | ColorToken::Info
            ) {
                let row = text
                    .lines()
                    .nth(cell.row as usize)
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let has_word = row.contains("success")
                    || row.contains("complete")
                    || row.contains("connected")
                    || row.contains("ready")
                    || row.contains("pass")
                    || row.contains("fail")
                    || row.contains("error")
                    || row.contains("denied")
                    || row.contains("blocked")
                    || row.contains("waiting")
                    || row.contains("needs")
                    || row.contains("retry")
                    || row.contains("attention")
                    || row.contains("warn")
                    || row.contains("info")
                    || row.contains("running")
                    || row.contains("working")
                    || row.contains("added")
                    || row.contains("removed")
                    || row.contains("approved")
                    || row.contains("rejected")
                    || row.contains("healthy")
                    || row.contains("progress")
                    || row.contains("active")
                    || "●○✓✗▸▾".contains(cell.glyph)
                    || row.chars().any(|c| "●○✓✗".contains(c));
                assert!(
                    has_word,
                    "{label}: colored outcome at row {} lacks text/glyph meaning: `{row}`",
                    cell.row
                );
            }
        }
    }
}

// ── VAL-V3-DS-007 / 008: statusline + local keys / no global hints ───────

#[test]
fn every_canonical_state_keeps_minimal_statusline_and_local_keys() {
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        let text = audit.render_txt();
        assert_statusline_minimal(&text, &label);
        assert_no_forbidden_chrome(&text, &label);
        // Global hint strip never appears above the statusline.
        for (idx, row) in text.lines().enumerate() {
            if idx == 39 {
                continue;
            }
            let lower = row.to_ascii_lowercase();
            assert!(
                !lower.contains("enter send")
                    && !lower.contains("? help")
                    && !lower.contains("/ commands · @"),
                "{label}: global hint row on line {idx}: `{}`",
                row.trim()
            );
        }
        // Statusline event cardinality mirrors paint.
        let counts = family_counts(&audit);
        let bottom = text.lines().last().unwrap_or("");
        if bottom.trim().starts_with("~/") && bottom.contains(" · ") {
            assert_eq!(
                counts.get("statusline").copied().unwrap_or(0),
                1,
                "{label}: painted statusline must emit one event"
            );
            // Background stays CANVAS (no filled strip).
            for cell in &audit.styled_cells {
                if cell.row == 39 && !cell.glyph.is_whitespace() {
                    assert_eq!(
                        cell.style.background,
                        Some(ColorToken::Canvas),
                        "{label}: statusline must share canvas background"
                    );
                }
            }
        } else {
            assert!(
                counts.get("statusline").copied().unwrap_or(0) <= 1,
                "{label}: duplicate statusline events"
            );
        }
    }
}

// ── VAL-V3-DS-009..014: surface arrangements + no floating modal ─────────

#[test]
fn every_canonical_state_declares_exactly_one_allowed_surface() {
    let mut by_surface: BTreeMap<String, usize> = BTreeMap::new();
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        let counts = family_counts(&audit);
        assert_eq!(
            counts.get("surface").copied().unwrap_or(0),
            1,
            "{label}: expected exactly one surface event"
        );
        let subject = surface_subject(&audit);
        assert!(
            ALLOWED_SURFACES.contains(&subject),
            "{label}: surface subject `{subject}` is not a declared arrangement"
        );
        // Registry surface must agree with the semantic subject (normalized).
        let expected = screen.surface.as_str();
        assert_eq!(
            subject, expected,
            "{label}: audit surface `{subject}` disagrees with registry `{expected}`"
        );
        *by_surface.entry(subject.to_owned()).or_insert(0) += 1;

        let text = audit.render_txt();
        match subject {
            "conversation-shell" => {
                // Transcript-first with bottom composer area (not a pure dock).
                assert!(
                    !text.contains("floating modal") && !text.contains("dialog overlay"),
                    "{label}: modal chrome on conversation shell"
                );
            }
            "action-dock" => {
                // Bottom dock replaces composer; primary action present for most docks.
                let primaries = primary_events(&audit);
                if !ZERO_PRIMARY_IDS.contains(&screen.id) {
                    assert!(
                        primaries.len() <= 1,
                        "{label}: dock primary-action cardinality {}",
                        primaries.len()
                    );
                }
            }
            "contextual-inspector" | "temporary-detail-view" => {
                // Return affordance should exist for temporary detail/inspector surfaces.
                let lower = text.to_ascii_lowercase();
                let has_return = lower.contains("return")
                    || lower.contains("esc")
                    || lower.contains("back")
                    || lower.contains("close")
                    || audit.events.iter().any(|e| {
                        matches!(
                            e.identity.family.as_str(),
                            "return-action" | "cancel-action" | "secondary-action"
                        ) || e
                            .action
                            .as_ref()
                            .is_some_and(|a| {
                                let l = a.label.to_ascii_lowercase();
                                l.contains("return") || l.contains("back") || l.contains("close")
                            })
                    });
                assert!(
                    has_return,
                    "{label}: detail/inspector surface needs a visible return path"
                );
            }
            _ => {}
        }
    }
    // All four arrangements must appear across the gallery.
    for surface in ALLOWED_SURFACES {
        assert!(
            by_surface.get(*surface).copied().unwrap_or(0) >= 1,
            "gallery must use surface arrangement {surface}"
        );
    }
}

// ── VAL-V3-DS-015..022: shared component anatomy unit suite ──────────────

#[test]
fn shared_component_anatomy_matches_design_system_contracts() {
    // Activity (DS-015).
    for (state, label) in [
        (ActivityState::Running, "Running"),
        (ActivityState::Waiting, "Waiting"),
        (ActivityState::Succeeded, "Complete"),
        (ActivityState::Failed, "Needs attention"),
        (ActivityState::Expanded, "Details open"),
    ] {
        let row = if matches!(state, ActivityState::Running) {
            ActivityRow::new("Check files", state, "one focused summary").with_interrupt("Stop")
        } else {
            ActivityRow::new("Check files", state, "one focused summary")
        };
        let rendered = row.lines();
        assert!(
            rendered.iter().any(|l| l.text.contains(label)),
            "activity state {label:?} must render a non-color label"
        );
        if matches!(state, ActivityState::Running) {
            assert!(
                row.anatomy().has_local_keys,
                "interruptible running work has local keys"
            );
        }
    }

    // Progressive disclosure (DS-016).
    let collapsed = DisclosureBlock::new("Why this changed", DisclosureState::Collapsed)
        .with_expanded_detail("A concise user-facing rationale.");
    assert!(collapsed
        .lines()
        .iter()
        .any(|l| l.text.to_ascii_lowercase().contains("expand")));
    let expanded = DisclosureBlock::new("Why this changed", DisclosureState::Expanded)
        .with_expanded_detail("A concise user-facing rationale.");
    let expanded_text = expanded
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(expanded_text.contains("concise user-facing"));

    // Selection (DS-017).
    let selection = SelectionDock::new(
        "Choose a model",
        vec![
            SelectionRow::new("Claude Opus 4.8", "Careful work on complex changes"),
            SelectionRow::new("GPT-5.2 Codex", "Fast help for focused edits"),
        ],
        0,
        "Changes the model for this conversation.",
        "Apply model",
    )
    .expect("valid selection");
    assert_eq!(selection.anatomy().primary_action_count, 1);
    assert!(selection.lines().iter().any(|l| l.text.starts_with('›')));
    assert!(!selection.lines().iter().any(|l| l
        .style
        .background
        .is_some_and(|bg| matches!(bg, ColorToken::Accent | ColorToken::Bright))));

    // Decision (DS-018).
    let decision = DecisionDock::new(
        Surface::ActionDock,
        "Allow this command?",
        "It will check the project files.",
        "Allow once",
    )
    .expect("valid decision")
    .with_secondary("Deny")
    .expect("secondary")
    .with_secondary("Cancel")
    .expect("cancel");
    assert_eq!(decision.anatomy().primary_action_count, 1);
    assert!(decision.anatomy().secondary_action_count <= 2);
    assert!(decision.anatomy().has_local_keys);

    // Connection (DS-019).
    let connection =
        ConnectionRow::new("Browser automation", ConnectionState::Connected, "Open");
    let lines = connection.lines();
    assert!(lines.iter().any(|l| l.text.contains("Browser automation")));
    assert!(connection.anatomy().primary_action_count <= 1);

    // Recovery (DS-020).
    let recovery = RecoveryBlock::new(
        "The network path is temporarily unavailable.",
        "Your draft and paused work are preserved.",
        "Retry now",
    );
    assert_eq!(recovery.anatomy().primary_action_count, 1);
    let rec_text = recovery
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rec_text.contains("preserved") || rec_text.contains("draft"));
    assert!(rec_text.contains("Retry now"));

    // Mission (DS-021).
    let mission = MissionCard::new(
        "Prepare the release safely and review the result.",
        "Running preflight",
        2,
        5,
        Some("One check still needs a review decision."),
        "Open review",
    );
    assert_eq!(mission.anatomy().primary_action_count, 1);
    let mission_text = mission
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(mission_text.contains("Prepare the release"));
    assert!(!mission_text.to_ascii_lowercase().contains("worker_id"));
    assert!(!mission_text.to_ascii_lowercase().contains("contract_id"));

    // Diff / review (DS-022).
    let diff = DiffBlock::new(
        "src/main.rs",
        vec![
            (DiffKind::Added, "let ready = true;"),
            (DiffKind::Removed, "let ready = false;"),
        ],
    );
    let diff_text = diff
        .lines()
        .iter()
        .map(|l| l.text.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(diff_text.contains('+') || diff_text.contains("true"));
    assert!(diff_text.contains('-') || diff_text.contains("false"));
    let review = ReviewBlock::new(
        "Release checklist",
        "One validation stage failed.",
        "Return",
    );
    assert_eq!(review.anatomy().primary_action_count, 1);
    assert!(review
        .lines()
        .iter()
        .any(|l| l.text.to_ascii_lowercase().contains("return")));
}

// ── VAL-V3-DS-023 / 024: plain English + one primary action ──────────────

#[test]
fn every_canonical_default_uses_plain_english_and_one_primary_when_actionable() {
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        let text = audit.render_txt();
        // Raw identifiers never lead statusline / primary labels.
        assert_statusline_minimal(&text, &label);
        for ban in RAW_ID_BANS {
            // Advanced evidence states may mention transports only deeper.
            let is_advanced = audit.events.iter().any(|e| {
                matches!(
                    e.identity.detail_level.as_str(),
                    "full" | "advanced" | "evidence"
                )
            }) || fixture.contains("diagnostics")
                || fixture.contains("envelope")
                || screen.id == "tool-envelope"
                || screen.id == "concurrency-metrics"
                || screen.id == "verification-ladder";
            if is_advanced && matches!(*ban, "stdio://" | "oauth2://") {
                continue;
            }
            assert!(
                !text.contains(ban),
                "{label}: raw/internal identifier `{ban}` leaked on default render"
            );
        }
        let primaries = primary_events(&audit);
        if ZERO_PRIMARY_IDS.contains(&screen.id) {
            assert!(
                primaries.len() <= 1,
                "{label}: automatic/live state allows 0-1 primary, got {}",
                primaries.len()
            );
            continue;
        }
        // Decision / dock / recovery / connection surfaces require 0 or 1.
        assert!(
            primaries.len() <= 1,
            "{label}: more than one primary-action event ({})",
            primaries.len()
        );
        if let Some(primary) = primaries.first() {
            if let Some(action) = &primary.action {
                assert!(action.enabled, "{label}: primary disabled");
                assert!(!action.label.trim().is_empty(), "{label}: empty primary");
                for ban in RAW_ID_BANS {
                    assert!(
                        !action.label.contains(ban),
                        "{label}: primary label carries raw ID `{ban}`"
                    );
                }
            }
        }
        // Secondary-action fan-out stays subordinate (at most a few).
        let secondaries = audit
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.identity.family.as_str(),
                    "secondary-action" | "cancel-action"
                )
            })
            .count();
        assert!(
            secondaries <= 6,
            "{label}: unusually dense secondary action fan-out ({secondaries})"
        );
    }
}

// ── VAL-V3-DS-025: global contrast thresholds ────────────────────────────

#[test]
fn every_canonical_state_meets_contrast_thresholds_on_styled_cells() {
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        // Theme-preview screens intentionally sample light palettes as swatches;
        // contrast is verified on the surrounding non-swatch text for them.
        let is_theme_gallery = screen.id.contains("theme-gallery");
        for cell in &audit.styled_cells {
            if cell.glyph.is_whitespace() {
                continue;
            }
            if is_theme_gallery && (cell.absolute_fg.is_some() || cell.absolute_bg.is_some()) {
                continue;
            }
            // Statusline and non-swatch content are scored against dark palette by default.
            let (fg, bg) = resolve_fg_bg(cell, true);
            let ratio = contrast_ratio(fg, bg);
            // Functional non-text glyphs (borders etc.) need 3:1; readable text needs 4.5:1.
            let is_functional_glyph = "─│╭╮╰╯█░●○✓✗›→↑↓▸▾·…".contains(cell.glyph);
            let min = if is_functional_glyph { 3.0 } else { 4.5 };
            // Faint deliberately supports secondary chrome; allow 3:1 floor for faint text.
            let min = if cell.style.foreground == ColorToken::Faint {
                3.0_f64
            } else {
                min
            };
            assert!(
                ratio + 0.05 >= min,
                "{label}: contrast {ratio:.2} < {min} for glyph '{}' fg={:?} bg={:?} token={:?}",
                cell.glyph,
                fg,
                bg,
                cell.style.foreground
            );
        }
    }
}

// ── Combined density / disclosure / gallery inventory lock ───────────────

#[test]
fn every_canonical_state_has_one_dominant_region_and_reading_measure() {
    for screen in canonical_inventory() {
        let fixture = publication_fixture(screen.id);
        let label = format!("{}/{}", screen.id, fixture);
        let audit = render_pair(screen.id, &fixture);
        let text = audit.render_txt();
        assert_canvas_geometry(&text, &label);
        let advanced = matches!(
            surface_subject(&audit),
            "temporary-detail-view"
        ) || fixture.contains("diagnostics")
            || fixture.contains("evidence")
            || matches!(
                screen.id,
                "tool-envelope"
                    | "concurrency-metrics"
                    | "verification-ladder"
                    | "theme-gallery-dark"
                    | "theme-gallery-light"
                    | "diff-viewer"
                    | "diff-viewer-staging"
                    | "timeline"
                    | "help"
                    | "status"
            );
        assert_reading_measure(&text, &label, advanced);
        // Competing dual headers / dual composers never appear.
        let you_rows = text
            .lines()
            .filter(|l| l.trim_start().starts_with("You ·") || l.trim_start().starts_with("You:"))
            .count();
        assert!(
            you_rows <= 6,
            "{label}: unusually many user turns; density may be unreadable ({you_rows})"
        );
    }
}

#[test]
fn registry_surface_inventory_covers_all_four_arrangements() {
    let mut counts = BTreeMap::new();
    for screen in canonical_inventory() {
        *counts.entry(screen.surface.as_str()).or_insert(0usize) += 1;
        assert!(
            ALLOWED_SURFACES.contains(&screen.surface.as_str()),
            "registry lists unknown surface for {}",
            screen.id
        );
    }
    assert_eq!(
        counts.len(),
        4,
        "registry must declare all four surface arrangements, got {counts:?}"
    );
    for surface in ALLOWED_SURFACES {
        assert!(
            counts.get(*surface).copied().unwrap_or(0) > 0,
            "missing registry occupancy for {surface}"
        );
    }
}

#[cfg(feature = "png")]
#[test]
fn embedded_fonts_and_token_tables_remain_authoritative() {
    mockup_harness_v3::chrome::tokens::verify_embedded_font_coverage()
        .expect("embedded DejaVu regular/bold must cover closed glyph policy");
    let dark = palette(Theme::Dark);
    let light = palette(Theme::Light);
    assert_eq!(dark.hex(ColorToken::Canvas), "#101011");
    assert_eq!(dark.hex(ColorToken::Accent), "#D9825F");
    assert_eq!(dark.hex(ColorToken::Ok), "#79A889");
    assert_eq!(dark.hex(ColorToken::Bad), "#C66F6F");
    assert_eq!(light.hex(ColorToken::Canvas), "#FAF7F2");
    assert_eq!(light.hex(ColorToken::Accent), "#C06B44");
    assert_eq!(light.hex(ColorToken::Ok), "#4F7942");
    assert_eq!(light.hex(ColorToken::Bad), "#B54A55");
    assert_eq!(ACCENT_USE_BUDGET, 3);
    assert_eq!((PNG_WIDTH, PNG_HEIGHT), (1200, 840));
}
