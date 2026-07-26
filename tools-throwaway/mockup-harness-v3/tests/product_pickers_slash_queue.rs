//! Cortex bottom-dock pickers + slash/queue/composer-multiline rewrite
//! (VAL-V3-DS-036).
//!
//! Owned expectations:
//! - Uniform dock anatomy (rounded frame + › focus, no filled SELBG strap)
//! - Picker rows use title · tradeoff grammar
//! - Single Claude-style prose byline owned by the dock
//! - No permanent putty `Key: verb` footer
//! - composer-multiline is jewel multi-line with info line (model · work mode)
//! - file-mention uses mild fuzzy match accent (Info token runs)

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::{
        components::{
            contains_bracket_key_atom, contains_global_putty_footer, fuzzy_match_spans,
            picker_apply_byline, Component, PickerRow,
        },
        picker_contract::{contract_fixture, NINE_PICKERS},
        tokens::{ColorToken, TextStyle},
    },
};

const FLOW2_PICKERS: &[&str] = &[
    "slash-menu",
    "command-palette",
    "command-palette-submenu",
    "attachment-picker",
    "export-picker",
    "file-mention",
    "queue-manager",
    "composer-multiline",
];

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        "composer-multiline" | "file-mention" | "slash-menu" => "flow2.input-and-approval.stable",
        "command-palette" | "command-palette-submenu" => "flow2.input-and-approval.stable",
        "attachment-picker" | "export-picker" | "queue-manager" => {
            "flow2.input-and-approval.stable"
        }
        id if NINE_PICKERS.contains(&id) => contract_fixture(id),
        _ => "public-audit",
    }
}

fn audit(screen_id: &str) -> mockup_harness_v3::audit::AuditDocument {
    audit_screen(screen_id, fixture_for(screen_id))
        .unwrap_or_else(|err| panic!("{screen_id} must render: {err}"))
}

#[test]
fn picker_row_helper_emits_title_tradeoff_and_focus_marker() {
    let idle = PickerRow::new("Markdown", "readable .md · headings and code")
        .expect("valid idle picker row");
    assert_eq!(
        idle.compact_line(),
        "  Markdown · readable .md · headings and code"
    );
    let selected = PickerRow::new("Markdown", "readable .md · headings and code")
        .expect("valid selected picker row")
        .selected();
    assert_eq!(
        selected.compact_line(),
        "› Markdown · readable .md · headings and code"
    );
    let lines = selected.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].style, TextStyle::accent());

    let expanded = PickerRow::new("Claude Opus 4.8", "More depth for complex changes · slower")
        .expect("valid expanded row")
        .selected()
        .expanded();
    let elines = expanded.lines();
    assert_eq!(elines.len(), 2);
    assert!(elines[0].text.starts_with('›'));
    assert!(elines[0].text.contains("Claude Opus 4.8"));
    assert!(elines[1].text.contains("More depth"));
    assert_eq!(elines[1].style, TextStyle::muted());
}

#[test]
fn picker_apply_byline_is_claude_prose_without_putty() {
    let byline = picker_apply_byline("apply").expect("byline");
    assert_eq!(byline, "Enter to apply · Esc to cancel · ↑↓ to choose");
    assert!(!contains_bracket_key_atom(&byline));
    assert!(!contains_global_putty_footer(&byline));
    assert!(!byline.contains("Shift+Tab"));
    assert!(!byline.contains("Ctrl+c:"));
}

#[test]
fn fuzzy_match_spans_mark_subsequence_runs_mildly() {
    let spans = fuzzy_match_spans("src/net/retry_policy.rs", "retry");
    assert!(
        spans.iter().any(|span| span.matched && span.text.contains('r')),
        "match runs must set matched=true: {spans:?}"
    );
    let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(joined, "src/net/retry_policy.rs");
    // Empty query yields a single unmatched span (no copper spam).
    let none = fuzzy_match_spans("assets/retry-diagram.png", "");
    assert_eq!(none.len(), 1);
    assert!(!none[0].matched);
}

#[test]
fn bottom_dock_pickers_share_uniform_anatomy_and_prose_bylines() {
    let mut screens = FLOW2_PICKERS.to_vec();
    screens.extend(NINE_PICKERS.iter().copied());
    for screen_id in screens {
        let doc = audit(screen_id);
        doc.validate()
            .unwrap_or_else(|err| panic!("{screen_id} audit must validate: {err}"));
        let text = doc.render_txt();
        let rows = text.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 40, "{screen_id} must be 40 rows");
        assert!(
            rows.iter().all(|row| row.chars().count() == 120),
            "{screen_id} rows must be width 120"
        );

        // Bottom action dock frame near the composer location.
        assert!(
            text.contains('╭') && text.contains('╰') && text.contains('│'),
            "{screen_id} must use the shared bottom rounded dock"
        );
        // Focus marker (resisted filled bar).
        assert!(
            text.contains('›'),
            "{screen_id} must mark focus with ›"
        );
        // No permanent putty footer / journey breadcrumbs / bracket chords.
        assert!(
            !contains_global_putty_footer(&text),
            "{screen_id} must not emit a permanent putty key filmstrip"
        );
        assert!(!text.contains("Shift+Tab:"), "{screen_id} putty ban");
        assert!(
            !contains_bracket_key_atom(&text),
            "{screen_id} must not use bracket key atoms"
        );
        assert!(
            !text.contains("Daily work ·")
                && !text.contains("Input and approval ·")
                && !text.contains("Work configuration · onboarding title"),
            "{screen_id} must not paint journey breadcrumb titles"
        );

        // Dock prose byline: Enter-to and Esc-to ownership.
        // composer-multiline is the multi-line jewel itself (not a decision dock),
        // so its owned chrome is model · work mode info — not Enter/Esc sheets.
        if screen_id != "composer-multiline" {
            assert!(
                text.contains("Enter to") || text.contains("Esc to"),
                "{screen_id} must own a Claude-style prose byline:\n{text}"
            );
            assert!(
                text.contains(" · "),
                "{screen_id} rows/headers must use middot title·tradeoff grammar"
            );
        }
    }
}

#[test]
fn composer_multiline_is_jewel_with_info_line() {
    let doc = audit("composer-multiline");
    let text = doc.render_txt();
    assert!(text.contains("Draft · 3 lines"));
    assert!(
        text.contains("Claude Opus 4.8 · Work mode") || text.contains("Claude Opus 4.8 · Work"),
        "multi-line jewel must paint muted model · work mode info:\n{text}"
    );
    assert!(text.contains("› Check the retry path in @retry"));
    assert!(!text.to_ascii_lowercase().contains("cursor at character"));
    assert!(!text.to_ascii_lowercase().contains("wraps visually"));
    assert!(!contains_global_putty_footer(&text));
    assert!(text.contains('╭') && text.contains('╰'));
}

#[test]
fn file_mention_rows_use_title_tradeoff_and_mild_fuzzy_info_paint() {
    let doc = audit("file-mention");
    let text = doc.render_txt();
    assert!(text.contains("› src/net/retry_policy.rs · project file · best match"));
    assert!(text.contains("src/net/retry_queue.rs · project file"));
    assert!(text.contains("tests/net/retry_policy_test.rs · project file · test"));
    assert!(text.contains("Enter to") || text.contains("Tab to insert file"));
    assert!(text.contains("Esc to dismiss") || text.contains("Esc to"));

    // Match runs should be Info-colored on at least one cell of the best-match
    // path, not copper-only (accent stays on ›).
    let info_cells = doc
        .styled_cells
        .iter()
        .filter(|cell| cell.style.foreground == ColorToken::Info)
        .count();
    assert!(
        info_cells > 0,
        "file-mention must spend mild Info paint on fuzzy match runs; found {info_cells}"
    );
}

#[test]
fn nine_flow3_pickers_keep_contract_titles_with_ds036_rows() {
    for screen_id in NINE_PICKERS {
        let doc = audit(screen_id);
        let text = doc.render_txt();
        assert!(
            text.contains('›'),
            "{screen_id} must keep the focus glyph"
        );
        assert!(
            text.contains("Enter to") || text.contains("Esc to"),
            "{screen_id} dock must own prose byline"
        );
        assert!(!contains_global_putty_footer(&text));
        // Row inventories still communicate tradeoffs beyond bare names.
        assert!(
            text.contains(" · ")
                || text.contains("Current")
                || text.contains("Pending")
                || text.contains("Preview")
                || text.contains("Source"),
            "{screen_id} must keep tradeoff language"
        );
    }
}
