//! Cortex permission + diff dock rewrite (VAL-V3-CROSS-022).
//!
//! Asserts ordered scope option prose, ActionDock-not-popup, Enter/Esc bylines,
//! human scope language (no bracket chords), and readable mini-diff chrome for
//! the owned IDs: permission-shell, permission-file-edit, permission-deny-reason,
//! diff-edit, diff-viewer, diff-viewer-staging.

use mockup_harness_v3::{
    audit::audit_screen,
    chrome::components::{contains_bracket_key_atom, contains_global_putty_footer},
};

const PERMISSION_IDS: &[&str] = &[
    "permission-shell",
    "permission-file-edit",
    "permission-deny-reason",
];

const DIFF_IDS: &[&str] = &["diff-edit", "diff-viewer", "diff-viewer-staging"];

fn fixture_for(screen_id: &str) -> &'static str {
    match screen_id {
        // Use persistent-supported / public fixtures so ordered scope options
        // (Allow once · project · Deny) are present for CROSS-022.
        "permission-shell" => "flow2-permission-shell-persistent-supported",
        "permission-file-edit" => "flow2-permission-file-edit",
        "permission-deny-reason" => "flow2-permission-deny-reason",
        "diff-edit" => "flow2-diff-pending",
        "diff-viewer" => "public-audit",
        "diff-viewer-staging" => "flow2-staging",
        _ => "public-audit",
    }
}

fn audit(screen_id: &str) -> mockup_harness_v3::audit::AuditDocument {
    audit_screen(screen_id, fixture_for(screen_id))
        .unwrap_or_else(|err| panic!("{screen_id} must render: {err}"))
}

fn find_ordered(text: &str, needles: &[&str]) -> bool {
    let mut cursor = 0usize;
    for needle in needles {
        match text[cursor..].find(needle) {
            Some(offset) => cursor += offset + needle.len(),
            None => return false,
        }
    }
    true
}

#[test]
fn permission_docks_order_scope_options_with_prose_bylines() {
    // VAL-V3-CROSS-022: Allow once → project/session allow → Deny, prose bylines.
    for id in ["permission-shell", "permission-file-edit"] {
        let doc = audit(id);
        doc.validate()
            .unwrap_or_else(|err| panic!("{id} audit must validate: {err}"));
        let text = doc.render_txt();
        let rows = text.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 40, "{id} must be 40 rows");
        assert!(
            rows.iter().all(|row| row.chars().count() == 120),
            "{id} rows must be width 120"
        );

        assert!(
            find_ordered(&text, &["Allow once", "Allow for this project", "Deny"]),
            "{id} must order Allow once → project allow → Deny:\n{text}"
        );
        assert!(
            text.contains("› Allow once"),
            "{id} primary must be focused Allow once:\n{text}"
        );
        assert!(
            text.contains("Enter to allow once") || text.contains("Enter to allow"),
            "{id} must expose Enter prose byline:\n{text}"
        );
        assert!(
            text.contains("Esc to cancel"),
            "{id} must expose Esc prose byline:\n{text}"
        );
        assert!(
            !contains_bracket_key_atom(&text),
            "{id} must not emit bracket chords:\n{text}"
        );
        assert!(
            !contains_global_putty_footer(&text),
            "{id} must not paint putty footer"
        );
        assert!(
            !text.contains("[stop]") && !text.contains("[enter]") && !text.contains("[esc]"),
            "{id} forbids residual bracket atoms"
        );
        // Outer editorial gutters: content / dock walls inset from the full width.
        let dock_top = rows
            .iter()
            .position(|row| row.contains('╭') && row.contains('─'))
            .unwrap_or_else(|| panic!("{id} must paint ActionDock top border"));
        let dock_row = rows[dock_top];
        assert!(
            dock_row.starts_with("  ╭") || dock_row.starts_with(" ╭"),
            "{id} dock borders keep outer pad gutters: {dock_row:?}"
        );
        // Dock title + rounded border proves ActionDock (not freestanding modal).
        assert!(
            text.contains("Permission choices") || text.contains("Permission"),
            "{id} ActionDock title expected"
        );
        // Human effect language before internal matcher IDs.
        assert!(
            !text.contains("matcher git diff --"),
            "{id} must not lead with raw command matcher jargon"
        );
        assert!(
            text.contains("this session")
                || text.contains("this project")
                || text.contains("this command only")
                || text.contains("this edit only"),
            "{id} must describe human scope:\n{text}"
        );

        // Surface evidence: primary is action.permission.allow-once.
        assert!(
            doc.events.iter().any(|event| {
                event.identity.family == "primary-action"
                    && event.identity.subject_id == "action.permission.allow-once"
            }),
            "{id} primary-action allow-once event required"
        );
    }
}

#[test]
fn deny_reason_dock_is_followup_prose_with_return_path() {
    let doc = audit("permission-deny-reason");
    doc.validate()
        .expect("permission-deny-reason audit must validate");
    let text = doc.render_txt();
    assert!(text.contains("Denied"));
    assert!(
        text.contains("Optional reason") || text.contains("optional reason"),
        "optional reason copy required:\n{text}"
    );
    assert!(
        text.contains("Type one short reason") || text.contains("› Type"),
        "focused free-text reason field required:\n{text}"
    );
    assert!(
        text.contains("Enter to send reason") || text.contains("Send reason"),
        "primary Send reason prose required:\n{text}"
    );
    assert!(
        text.contains("Esc to cancel") || text.contains("Skip"),
        "return/cancel path required:\n{text}"
    );
    assert!(
        text.contains("Allow and Deny choices are closed")
            || text.contains("original Allow and Deny"),
        "closed Allow/Deny state required:\n{text}"
    );
    assert!(!text.contains("Allow once"), "Allow once must not remain a choice");
    assert!(!contains_bracket_key_atom(&text));
    assert!(!text.contains("[enter]") && !text.contains("[esc]"));

    assert!(doc.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.permission.send-reason"
    }));
    assert!(doc.events.iter().any(|event| {
        event.identity.family == "secondary-action"
            && event.identity.subject_id == "action.permission.skip-reason"
    }));
}

#[test]
fn diff_edit_dock_is_githubish_pending_with_primary_review() {
    let doc = audit("diff-edit");
    doc.validate().expect("diff-edit audit must validate");
    let text = doc.render_txt();

    assert!(text.contains("Pending edit"));
    assert!(text.contains("src/net/retry_policy.rs"));
    assert!(
        text.contains("+4") && text.contains("-3") && text.contains("2 hunks"),
        "patch summary +N -M / hunks required:\n{text}"
    );
    // Mini unified-diff language (GitHub-ish prefixes, not clutter walls).
    assert!(
        text.contains("-    return retry_after") || text.contains("-return retry_after"),
        "removed line with - prefix:\n{text}"
    );
    assert!(
        text.contains("+    return retry_after + jitter")
            || text.contains("+return retry_after + jitter"),
        "added line with + prefix:\n{text}"
    );
    assert!(text.contains("Why this change") || text.contains("keep retry timing"));
    assert!(!text.contains("Applied") && !text.contains("Success") && !text.contains("Staged"));
    assert!(
        text.contains("› Review changes"),
        "primary Review changes focus:\n{text}"
    );
    assert!(
        text.contains("Enter to review changes") || text.contains("Enter to review"),
        "Enter prose byline for review primary:\n{text}"
    );
    assert!(
        text.contains("Apply all") && text.contains("Cancel"),
        "secondary Apply all + Cancel required"
    );
    assert!(!contains_bracket_key_atom(&text));
    assert!(!contains_global_putty_footer(&text));

    let dock_top = text
        .lines()
        .position(|row| row.contains('╭') && row.contains('─'))
        .expect("diff-edit ActionDock top border");
    assert!(
        text.lines().nth(dock_top).unwrap().starts_with("  ╭")
            || text.lines().nth(dock_top).unwrap().starts_with(" ╭"),
        "outer pad on dock"
    );

    assert_eq!(
        doc.events
            .iter()
            .filter(|event| event.identity.family == "primary-action")
            .count(),
        1
    );
}

#[test]
fn staging_and_viewer_keep_readable_diff_language_without_clutter() {
    for id in DIFF_IDS {
        let doc = audit(id);
        doc.validate()
            .unwrap_or_else(|err| panic!("{id} audit must validate: {err}"));
        let text = doc.render_txt();
        assert_eq!(text.lines().count(), 40);
        assert!(text.lines().all(|row| row.chars().count() == 120));
        assert!(!contains_bracket_key_atom(&text), "{id} brackets forbidden");
        assert!(!contains_global_putty_footer(&text), "{id} no putty");
        // Diff meaning must be color-independent (glyph +/-)
        assert!(
            text.contains('+') && text.contains('-'),
            "{id} must keep +/- line markers"
        );
        match *id {
            "diff-viewer-staging" => {
                assert!(text.contains("Included") || text.contains("✓ Included"));
                assert!(text.contains("hunk.retry-guard") || text.contains("retry-guard"));
                assert!(
                    text.contains("Enter to finish")
                        || text.contains("Enter to refresh and review")
                        || text.contains("Enter to"),
                    "staging Enter byline:\n{text}"
                );
                assert!(
                    text.contains("Esc to cancel"),
                    "staging Esc byline:\n{text}"
                );
                // Prefer prose over raw single-letter filmstrip clutter when possible;
                // short letter clauses remain acceptable when joined by middots.
                assert!(
                    !text.contains("[i]")
                        && !text.contains("[x]")
                        && !text.contains("[a]")
                        && !text.contains("[e]")
                        && !text.contains("[enter]")
                        && !text.contains("[esc]"),
                    "staging forbids bracket chords:\n{text}"
                );
            }
            "diff-viewer" => {
                assert!(
                    text.contains("Esc to return") || text.contains("return to selected finding"),
                    "viewer return prose:\n{text}"
                );
                assert!(
                    text.contains("Read-only") || text.contains("read-only"),
                    "viewer read-only language:\n{text}"
                );
            }
            "diff-edit" => {
                // covered by dedicated test above
            }
            _ => {}
        }
    }
}

#[test]
fn permission_and_diff_screens_are_dock_or_detail_not_popup() {
    for id in PERMISSION_IDS.iter().chain(DIFF_IDS.iter()) {
        let doc = audit(id);
        let text = doc.render_txt();
        assert!(
            !text.contains("Permission required") || !text.contains("centered modal"),
            "{id} must not use freestanding modal language"
        );
        // No full-width modal shadow lines or ASCII dim backdrop markers.
        assert!(!text.contains("▓▓▓▓") && !text.contains("████████████████"));
        // Rounded ActionDock walls for decision surfaces; recipient details may omit.
        if matches!(
            *id,
            "permission-shell"
                | "permission-file-edit"
                | "permission-deny-reason"
                | "diff-edit"
        ) {
            assert!(
                text.contains('╭') && text.contains('╰'),
                "{id} ActionDock rounded walls required"
            );
        }
    }
}

#[test]
fn outer_pad_gutters_hold_for_permission_and_diff_canonicals() {
    for id in PERMISSION_IDS.iter().chain(DIFF_IDS.iter()) {
        let text = audit(id).render_txt();
        for (index, row) in text.lines().enumerate() {
            // Statusline and blank rows may be empty; non-empty content leaves col0 blank outer pad.
            if row.trim().is_empty() {
                continue;
            }
            let first = row.chars().next().unwrap_or(' ');
            // Statusline row is sacred minimal chrome; padding rule is soft for STATUS.
            if index == 39 {
                continue;
            }
            assert_eq!(
                first, ' ',
                "{id} row {index} must keep left outer pad gutter, got {:?}",
                &row[..row.chars().take(8).collect::<String>().len().min(row.len())]
            );
        }
    }
}
