//! Focused regression: turn-error inspect-connection RenderRef must cover
//! the true visible inspect-copy row and label cells (not lag one row above).

use mockup_harness_v3::audit::{audit_screen, AuditDocument};

const ACTION_ID: &str = "action.turn-error.inspect-connection";
const LABEL: &str = "Inspect the connection problem";
const INSPECT_COPY: &str =
    "Inspect the connection problem when the provider path looks broken.";

fn action_event<'a>(
    audit: &'a AuditDocument,
    action_id: &str,
) -> &'a mockup_harness_v3::audit::SemanticEvent {
    audit
        .events
        .iter()
        .find(|event| {
            event
                .action
                .as_ref()
                .is_some_and(|action| action.action_id == action_id)
        })
        .unwrap_or_else(|| panic!("missing semantic action {action_id}"))
}

fn cell_slice(audit: &AuditDocument, row: u16, start: u16, end: u16) -> String {
    audit
        .styled_cells
        .iter()
        .filter(|cell| cell.row == row && cell.column >= start && cell.column < end)
        .map(|cell| cell.glyph)
        .collect()
}

fn row_text(audit: &AuditDocument, row: u16) -> String {
    audit
        .styled_cells
        .iter()
        .filter(|cell| cell.row == row)
        .map(|cell| cell.glyph)
        .collect()
}

#[test]
fn turn_error_inspect_connection_render_ref_matches_visible_inspect_copy() {
    let audit = audit_screen("turn-error", "gate.recovery.turn-error")
        .expect("turn-error must produce typed render evidence");
    audit
        .validate()
        .expect("turn-error audit bijection must stay valid");

    let text = audit.render_txt();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 40, "turn-error canvas stays fixed at 40 rows");

    let inspect_row = rows
        .iter()
        .position(|row| row.contains(INSPECT_COPY))
        .expect("visible inspect-copy line must remain in the recovery body")
        as u16;
    assert!(
        (15..25).contains(&inspect_row),
        "inspect copy must sit in the recovery body above the action dock, found row {inspect_row}"
    );
    assert!(
        !rows[inspect_row as usize].contains("Draft, transcript"),
        "inspect copy must not share the draft-preservation row"
    );

    let event = action_event(&audit, ACTION_ID);
    let action = event.action.as_ref().expect("inspect action evidence");
    assert_eq!(action.label, LABEL);
    assert_eq!(
        event.identity.render_refs.len(),
        1,
        "{ACTION_ID} must bind one visible render ref"
    );
    let reference = &event.identity.render_refs[0];
    assert_eq!(
        reference.row, inspect_row,
        "{ACTION_ID} audit row must track the visible inspect-copy row (not lag above it)"
    );

    let visible = row_text(&audit, reference.row);
    let label_start = visible
        .char_indices()
        .map(|(byte, _)| byte)
        .zip(0u16..)
        .find(|(byte, _)| visible[*byte..].starts_with(LABEL))
        .map(|(_, col)| col)
        .unwrap_or_else(|| panic!("label {LABEL:?} must appear on inspect row {inspect_row}"));
    let label_end = label_start + LABEL.chars().count() as u16;
    assert!(
        reference.start <= label_start && reference.end >= label_end,
        "{ACTION_ID} ref [{}..{}) must cover label {LABEL:?} at [{label_start}..{label_end}) on row {inspect_row}; cells={:?}",
        reference.start,
        reference.end,
        cell_slice(&audit, reference.row, reference.start, reference.end)
    );
    let fragment = cell_slice(&audit, reference.row, reference.start, reference.end);
    assert!(
        fragment.contains(LABEL),
        "{ACTION_ID} cells must contain {LABEL:?}, got {fragment:?}"
    );
}

#[test]
fn turn_error_inspect_connection_geometry_survives_canonical_fixture() {
    for fixture in ["gate.recovery.turn-error", "canonical.turn-error"] {
        let audit = match audit_screen("turn-error", fixture) {
            Ok(audit) => audit,
            Err(_) if fixture == "canonical.turn-error" => {
                // Gate fixture is the recovery authority; skip if canonical id is unused.
                continue;
            }
            Err(error) => panic!("turn-error fixture {fixture} must render: {error}"),
        };
        let inspect_row = audit
            .render_txt()
            .lines()
            .position(|row| row.contains(INSPECT_COPY))
            .unwrap_or_else(|| panic!("fixture {fixture} must keep inspect copy"))
            as u16;
        let event = action_event(&audit, ACTION_ID);
        let reference = &event.identity.render_refs[0];
        assert_eq!(
            reference.row, inspect_row,
            "{fixture}/{ACTION_ID} must follow inspect-copy row {inspect_row}"
        );
        let fragment = cell_slice(&audit, reference.row, reference.start, reference.end);
        assert!(
            fragment.contains(LABEL),
            "{fixture}/{ACTION_ID} geometry must cover {LABEL:?}, got {fragment:?}"
        );
    }
}
