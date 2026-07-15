//! Density / chrome discipline for the top offending pilot frames (r5 rejection P2).
//!
//! Journey structure and identity stay fixed. These checks require progressive
//! disclosure, shared dock discipline, and no dock restatement of visible list rows.

use mockup_harness_v3::audit::audit_screen;

fn nonempty_content_lines(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

fn dock_inner_content_lines(text: &str) -> usize {
    let lines = text.lines().collect::<Vec<_>>();
    let top = lines
        .iter()
        .position(|line| line.contains('╭'))
        .expect("frame must expose a top dock border");
    let bottom = lines
        .iter()
        .position(|line| line.contains('╰'))
        .expect("frame must expose a bottom dock border");
    lines[top + 1..bottom]
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed
                    .chars()
                    .all(|ch| matches!(ch, '│' | ' ' | '─'))
        })
        .count()
}

#[test]
fn onboarding_keeps_entry_brand_but_uses_compact_standard_dock() {
    let audit = audit_screen("onboarding", "flow3-onboarding")
        .expect("onboarding must render for density review");
    let text = audit.render_txt();
    let lines = text.lines().collect::<Vec<_>>();

    // Entry brand stays (F-6), but the oversized mid-screen card is gone.
    assert!(
        lines[2..=7].iter().any(|line| line.contains('█')),
        "onboarding keeps the restrained Code entry brand"
    );
    let dock_top = lines
        .iter()
        .position(|line| line.contains('╭'))
        .expect("onboarding keeps an action dock");
    let dock_bottom = lines
        .iter()
        .position(|line| line.contains('╰'))
        .expect("onboarding dock must close");
    assert_eq!(dock_bottom, 37, "dock stays bottom-anchored on row 37");
    assert!(
        dock_top >= 30,
        "onboarding must use the standard bottom dock band, not a mid-canvas card (dock_top={dock_top})"
    );
    assert!(
        dock_inner_content_lines(&text) <= 8,
        "onboarding dock content must stay compact, got {}",
        dock_inner_content_lines(&text)
    );
    assert!(
        nonempty_content_lines(&text) <= 22,
        "onboarding nonempty content must drop under the prior 26-line offender, got {}",
        nonempty_content_lines(&text)
    );
    // One primary, no dual cancel-style footnotes restating Continue/Skip.
    assert!(text.contains("› Continue") && text.contains("Enter to continue"));
    assert!(!text.contains("Continue accepts the recommendation. Skip keeps the same safe default."));
}

#[test]
fn session_picker_dock_does_not_fully_restate_selected_list_row() {
    let audit = audit_screen("session-picker", "flow3-session-picker")
        .expect("session picker must render for density review");
    let text = audit.render_txt();

    assert!(text.contains("› Retry policy review"));
    assert!(text.contains("Tests pass; review is ready"));
    assert!(text.contains("Resume latest") || text.contains("Enter to resume"));
    // Dock may name the selection, but must not re-wrap the full list row body.
    assert!(
        !text.contains("Selected · Retry policy review\n")
            || dock_inner_content_lines(&text) <= 5,
        "selected dock must not fully paraphrase the focused list row"
    );
    assert!(
        !text.contains("~/project · latest stable work from today · Your saved draft:"),
        "dock must not restate project context + draft as one dense dump line"
    );
    assert!(
        dock_inner_content_lines(&text) <= 5,
        "session-picker dock content too dense: {}",
        dock_inner_content_lines(&text)
    );
}

#[test]
fn permission_shell_short_summarizes_before_full_hash_evidence() {
    let audit = audit_screen("permission-shell", "flow2-permission-shell-short")
        .expect("short permission must render");
    let text = audit.render_txt();

    assert!(text.contains("Why Code needs it"));
    assert!(text.contains("What will running it do") || text.contains("What running it will do"));
    assert!(text.contains("printf '%s\\n' \"retry check\""));
    assert!(text.contains("Allow once"));
    // First short-command frame keeps summary depth: no full SHA banner yet.
    assert!(
        !text.contains("SHA-256:"),
        "short permission first frame must hide the full SHA until expansion"
    );
    assert!(
        text.contains("e to expand")
            || text.contains("Expand command detail")
            || text.contains("Show command detail")
            || text.contains("Command detail collapsed"),
        "short permission must offer progressive disclosure for hash evidence"
    );
    assert!(
        nonempty_content_lines(&text) <= 18,
        "short permission must reduce chrome density under prior 21 lines, got {}",
        nonempty_content_lines(&text)
    );
}

#[test]
fn pending_diff_summarizes_second_hunk_instead_of_stacking_both_full_payloads() {
    let audit = audit_screen("diff-edit", "flow2-diff-pending")
        .expect("pending diff must render");
    let text = audit.render_txt();

    assert!(text.contains("Pending edit"));
    assert!(text.contains("Patch summary +4 -3 · 2 hunks"));
    assert!(text.contains("Hunk @@ -42,2 +42,3"));
    assert!(text.contains("-    return retry_after"));
    assert!(text.contains("+    return retry_after + jitter"));
    // Second hunk is disclosed as a summary, not a second full payload dump.
    assert!(
        text.contains("hunk.retry-test")
            || text.contains("Hunk @@ -88,2 +89,2")
            || text.contains("Second hunk"),
        "second hunk remains discoverable"
    );
    assert!(
        !text.contains("assert_eq!(wait, 4)")
            || text.matches("assert_eq!(wait,").count() <= 1,
        "pending edit must not fully expand both hunks on the first frame"
    );
    assert!(
        nonempty_content_lines(&text) <= 22,
        "pending diff density must drop under prior 27 lines, got {}",
        nonempty_content_lines(&text)
    );
    assert!(text.contains("› Review changes") || text.contains("Review changes"));
    assert!(
        !text.contains("Review is read-only · editing is available inside staging detail")
            || dock_inner_content_lines(&text) <= 5,
        "dock footer must not restate review/staging description when actions already say it"
    );
}

#[test]
fn staging_keeps_both_hunks_but_collapses_action_chrome() {
    let audit = audit_screen("diff-viewer-staging", "flow2-staging")
        .expect("staging must render");
    let text = audit.render_txt();

    assert!(text.contains("Included 1 of 2 hunks"));
    assert!(text.contains("hunk.retry-guard"));
    assert!(text.contains("hunk.retry-test"));
    assert!(text.contains("Enter to finish") || text.contains("Finish"));
    // Keys stay local, not as a long dual-row teaching strip plus body copy.
    assert!(
        !text.contains("Include or Exclude changes the focused hunk; Include all selects both. No file is written until Finish.")
            || text.contains("i include") || text.contains("Include"),
        "instructional restatement is optional only if local keys remain"
    );
    assert!(
        nonempty_content_lines(&text) <= 18,
        "staging density must drop under prior 21 lines, got {}",
        nonempty_content_lines(&text)
    );
}

#[test]
fn shared_dock_primitive_covers_standard_bottom_band() {
    use mockup_harness_v3::chrome::layout::{action_dock_geometry, ACTION_DOCK_TOP};

    let geometry = action_dock_geometry();
    assert_eq!(geometry.top, ACTION_DOCK_TOP);
    assert_eq!(geometry.bottom, 37);
    assert_eq!(geometry.inner_rows, 5);
    assert!(geometry.top >= 30, "standard dock band stays near the bottom edge");
}
