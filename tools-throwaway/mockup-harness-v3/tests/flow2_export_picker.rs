use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn export_picker_opens_with_readable_formats_default_and_current_selection() {
    let audit = audit_screen("export-picker", "flow2-export-picker")
        .expect("export-picker must produce typed render evidence");
    audit
        .validate()
        .expect("export-picker evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Export conversation"));
    assert!(text.contains("Creates one sandboxed file from this conversation"));
    assert!(text.contains("› Markdown"));
    assert!(text.contains("readable .md"));
    assert!(text.contains("headings and code"));
    assert!(text.contains("[default]"));
    assert!(text.contains("JSON"));
    assert!(text.contains("structured messages"));
    assert!(text.contains(".json"));
    assert!(text.contains("HTML"));
    assert!(text.contains("styled shareable page"));
    assert!(text.contains(".html"));
    assert!(text.contains("Patch"));
    assert!(text.contains("git-applyable diff"));
    assert!(text.contains(".patch"));
    assert!((text.contains("Enter to export") || text.contains("[enter] Export")));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    assert!(text.contains("Paused draft · 3 authored lines"));
    assert!(text.contains("Keep the summary short before we merge."));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));

    // Default badge and current selection both land on Markdown without color dependence.
    assert!(
        text.lines()
            .any(|line| line.contains("› Markdown") && line.contains("[default]")),
        "Markdown must be both the selected row and the labeled default"
    );
    let format_markers = text
        .lines()
        .filter(|line| {
            line.contains("› Markdown")
                || line.contains("› JSON")
                || line.contains("› HTML")
                || line.contains("› Patch")
        })
        .count();
    assert_eq!(
        format_markers, 1,
        "exactly one export format selection marker must be set"
    );
    assert_eq!(
        text.matches("[default]").count(),
        1,
        "exactly one default badge must be set"
    );

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1,
        "export-picker must preselect exactly one format"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "selection" && event.identity.subject_id == "Markdown"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "default" && event.identity.subject_id == "Markdown"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "origin" && event.identity.subject_id == "composer-multiline"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "consequence"
            && event.identity.subject_id.contains("markdown")
    }));
    for format in ["Markdown", "JSON", "HTML", "Patch"] {
        assert!(
            audit.events.iter().any(|event| {
                event.identity.family == "export-format" && event.identity.subject_id == format
            }),
            "{format} must publish an export-format event"
        );
    }
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.export-picker.confirm-selected"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.export-picker.confirm-selected"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.export-picker.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn export_confirm_creates_selected_sandbox_artifact_with_inline_success() {
    let confirmed = replay_case("export-confirm", 1)
        .expect("confirming export must be replayable");
    confirmed
        .validate()
        .expect("export confirm replay must expose one valid transition");

    assert_eq!(confirmed.before.screen_id, "export-picker");
    assert_eq!(confirmed.after.screen_id, "composer-multiline");
    assert_eq!(confirmed.before.origin_stack, confirmed.after.origin_stack);
    assert_eq!(
        confirmed.before.scroll_anchor, confirmed.after.scroll_anchor,
        "export confirm must not move the transcript"
    );
    assert_eq!(
        confirmed.before.composer_height, confirmed.after.composer_height,
        "export confirm must preserve composer height"
    );
    assert_eq!(
        confirmed.before.draft, confirmed.after.draft,
        "export does not mutate the authored draft"
    );
    assert_eq!(
        confirmed.before.focus, confirmed.after.focus,
        "export restores the original focus after success"
    );
    assert!(
        confirmed.after.transcript.contains("Exported conversation as Markdown"),
        "confirm reports a single inline Markdown success summary"
    );
    assert!(
        confirmed
            .after
            .transcript
            .contains("exports/retry-review.md"),
        "success summary names the sandboxed Markdown artifact"
    );
    assert!(
        !confirmed.before.transcript.contains("Exported conversation"),
        "success claim appears only after confirm"
    );

    let artifact = confirmed
        .preserved_field_hashes
        .get("export_artifact")
        .expect("confirm must record the sandbox export artifact");
    assert_ne!(
        artifact.before, artifact.after,
        "confirm creates exactly the selected sandboxed output"
    );
    let result_events = confirmed
        .preserved_field_hashes
        .get("result_event_count")
        .expect("confirm must record result event evidence");
    assert_ne!(
        result_events.before, result_events.after,
        "confirm produces exactly one success result event"
    );
    assert_eq!(confirmed.side_effects.len(), 1);
    assert!(confirmed.side_effects[0].simulated);
    assert_eq!(confirmed.side_effects[0].kind, "export-artifact");
    assert_eq!(
        confirmed.side_effects[0].status,
        "simulated-export-create"
    );
    assert_eq!(
        confirmed.attempts[0].outcome, "exported",
        "one attempt leads to the selected format"
    );

    for field in [
        "draft",
        "composer_height",
        "scroll_anchor",
        "attachment",
    ] {
        let hashes = confirmed
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "confirm restores exact conversation geometry for {field}"
        );
    }
}

#[test]
fn export_cancel_creates_no_artifact_or_success_claim() {
    let cancelled =
        replay_case("export-cancel", 1).expect("cancelling export must be replayable");
    cancelled
        .validate()
        .expect("export cancel replay must expose one valid transition");

    assert_eq!(cancelled.before.screen_id, "export-picker");
    assert_eq!(cancelled.after.screen_id, "composer-multiline");
    assert_eq!(
        cancelled.before.draft, cancelled.after.draft,
        "cancel restores the original draft byte for byte"
    );
    assert_eq!(
        cancelled.before.transcript, cancelled.after.transcript,
        "cancel keeps the transcript unchanged"
    );
    assert!(
        !cancelled.after.transcript.contains("Exported conversation"),
        "cancel shows no success claim"
    );

    let artifact = cancelled
        .preserved_field_hashes
        .get("export_artifact")
        .expect("cancel must record absent export artifact evidence");
    assert_eq!(
        artifact.before, artifact.after,
        "cancel creates no sandboxed export artifact"
    );
    let result_events = cancelled
        .preserved_field_hashes
        .get("result_event_count")
        .expect("cancel must record result event count");
    assert_eq!(
        result_events.before, result_events.after,
        "cancel produces no success result event"
    );
    assert_eq!(cancelled.side_effects.len(), 1);
    assert_eq!(cancelled.side_effects[0].status, "simulated-no-change");
    assert_eq!(cancelled.attempts[0].outcome, "cancelled");

    for field in [
        "draft",
        "selection",
        "focus",
        "composer_height",
        "scroll_anchor",
        "attachment",
        "transcript",
    ] {
        let hashes = cancelled
            .preserved_field_hashes
            .get(field)
            .unwrap_or_else(|| panic!("{field} must have preservation evidence"));
        assert_eq!(
            hashes.before, hashes.after,
            "cancel must byte-restore {field}"
        );
    }
}

#[test]
fn public_selector_publishes_the_export_picker_pair() {
    let output = harness()
        .arg("export-picker")
        .output()
        .expect("public canonical selector must run");
    if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        if pure_mode() {
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
    } else {
        assert!(output.status.success(), "export-picker must publish after renderer preflight:\n{}", String::from_utf8_lossy(&output.stderr));
    }
    }
    }
    }
    for extension in ["txt", "png"] {
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/input/export-picker.{extension}"
            ))
            .is_file(),
            "canonical export-picker.{extension} must publish"
        );
    }
}
