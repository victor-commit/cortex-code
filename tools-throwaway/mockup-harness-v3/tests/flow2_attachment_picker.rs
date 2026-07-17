use std::{path::Path, process::Command};

use mockup_harness_v3::{audit::audit_screen, scenario::replay_case};


fn pure_mode() -> bool {
    mockup_harness_v3::renderer::pure_cortex_gallery_active()
}

fn harness() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mockup-harness-v3"))
}

#[test]
fn attachment_picker_opens_at_composer_with_consequence_and_project_paths() {
    let audit = audit_screen("attachment-picker", "flow2-attachment-picker")
        .expect("attachment-picker must produce typed render evidence");
    audit
        .validate()
        .expect("attachment-picker evidence must stay bijective");

    let text = audit.render_txt();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|row| row.chars().count() == 120));

    assert!(text.contains("Attach file"));
    assert!(text.contains("png, jpg, pdf, svg"));
    assert!(text.contains("Attaches one project file to this draft"));
    assert!(text.contains("project-relative paths"));
    assert!(text.contains("query:"));
    assert!(text.contains("matching"));
    assert!(text.contains("› assets/retry-diagram.png"));
    assert!(text.contains("eligible"));
    assert!(text.contains("README.md"));
    assert!(text.contains("unsupported"));
    assert!((text.contains("Enter to attach") || text.contains("[enter] Attach")));
    assert!((text.contains("Esc to cancel") || text.contains("[esc] Cancel") || text.contains("Esc to")));
    // Monochrome eligibility remains readable from labels, not color alone.
    assert!(text.contains("png · 48.2k · eligible"));
    assert!(text.contains("md · 11.4k · unsupported"));
    assert!(text.contains("Paused draft · 3 authored lines"));
    assert!(text.contains("Keep the summary short before we merge."));
    assert!(text.contains("~/project · main · Claude Opus 4.8"));

    assert_eq!(
        audit
            .events
            .iter()
            .filter(|event| event.identity.family == "selection")
            .count(),
        1,
        "attachment-picker must preselect exactly one eligible candidate"
    );
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "origin" && event.identity.subject_id == "composer-multiline"
    }));
    assert!(audit.events.iter().any(|event| {
        event.identity.family == "consequence"
            && event.identity.subject_id.contains("attach")
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.attachment-picker.confirm-selected"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "direct"
    }));
    assert!(audit.transitions.iter().any(|transition| {
        transition.action_id == "action.attachment-picker.cancel"
            && transition.to_screen_id == "composer-multiline"
            && transition.kind == "cancel"
    }));
}

#[test]
fn attachment_filtering_and_eligibility_are_explicit_without_color() {
    let filtered = audit_screen("attachment-picker", "flow2-attachment-filter-png")
        .expect("filtered attachment picker must render");
    filtered
        .validate()
        .expect("filtered attachment evidence must stay bijective");
    let filtered_txt = filtered.render_txt();
    assert!(filtered_txt.contains("query: png"));
    assert!(filtered_txt.contains("matching"));
    assert!(filtered_txt.contains("› assets/retry-diagram.png"));
    assert!(filtered_txt.contains("eligible"));
    assert!(!filtered_txt.contains("README.md"));
    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "filter" && event.identity.subject_id == "query-png"
    }));
    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "attachment-candidate"
            && event.identity.subject_id.contains("retry-diagram.png")
    }));
    // Eligible wording and selection exist; unsupported labels remain color-independent.
    assert!(filtered.events.iter().any(|event| {
        event.identity.family == "eligible-row"
            && event.identity.subject_id.contains("retry-diagram.png")
    }));
    assert!(filtered.events.iter().any(|event| {
        event
            .identity
            .family
            .eq("primary-action")
            && event.identity.subject_id == "action.attachment-picker.confirm-selected"
    }));
    // Full catalogue shows ineligible rows; filter-png keeps only matching eligible paths.
    let default = audit_screen("attachment-picker", "flow2-attachment-picker")
        .expect("default attachment catalogue must render");
    default.validate().expect("default catalogue bijective");
    let default_txt = default.render_txt();
    assert!(default_txt.contains("README.md"));
    assert!(default_txt.contains("unsupported"));
    assert!(default.events.iter().any(|event| {
        event.identity.family == "ineligible-row" && event.identity.subject_id.contains("README.md")
    }));
    assert!(
        !default_txt
            .lines()
            .any(|line| line.contains("› README.md")),
        "unsupported rows must not be the selected confirmable target"
    );

    let empty = audit_screen("attachment-picker", "flow2-attachment-no-results")
        .expect("no-results attachment picker must render");
    empty
        .validate()
        .expect("no-results attachment evidence must stay bijective");
    let empty_txt = empty.render_txt();
    assert!(empty_txt.contains("query: zzq"));
    assert!(empty_txt.contains("No project files match zzq."));
    assert!(empty_txt.contains("Paused draft"));
    assert!(empty.events.iter().any(|event| {
        event.identity.family == "empty-result" && event.identity.subject_id.contains("zzq")
    }));
    // Confirm is absent when there are zero matches.
    assert!(!empty.events.iter().any(|event| {
        event.identity.family == "primary-action"
            && event.identity.subject_id == "action.attachment-picker.confirm-selected"
    }));
}

#[test]
fn attachment_confirm_and_cancel_preserve_the_exact_multiline_origin() {
    let confirmed = replay_case("attachment-confirm", 1)
        .expect("confirming an eligible attachment must be replayable");
    let cancelled =
        replay_case("attachment-cancel", 1).expect("cancelling attachment must be replayable");

    confirmed
        .validate()
        .expect("attachment confirm replay must expose one valid transition");
    assert_eq!(confirmed.before.screen_id, "attachment-picker");
    assert_eq!(confirmed.after.screen_id, "composer-multiline");
    assert_eq!(confirmed.before.origin_stack, confirmed.after.origin_stack);
    assert_eq!(
        confirmed.before.scroll_anchor, confirmed.after.scroll_anchor,
        "attachment confirm must not move the transcript"
    );
    assert_eq!(
        confirmed.before.composer_height, confirmed.after.composer_height,
        "attachment confirm must preserve composer height"
    );
    assert_ne!(
        confirmed.before.draft, confirmed.after.draft,
        "confirm must insert exactly one attachment token"
    );
    assert!(
        confirmed
            .after
            .draft
            .contains("[attachment: assets/retry-diagram.png]"),
        "confirm inserts the selected project-relative attachment token"
    );
    assert_eq!(
        confirmed.after.attachment, "assets/retry-diagram.png",
        "exactly one attachment is recorded on the origin composer"
    );
    assert_eq!(
        confirmed.before.attachment, "",
        "the open picker state has no prior attachment"
    );
    let surrounding = confirmed
        .preserved_field_hashes
        .get("draft_surrounding")
        .expect("confirm must hash unchanged draft surrounding text");
    assert_eq!(surrounding.before, surrounding.after);
    let attachment_count = confirmed
        .preserved_field_hashes
        .get("attachment_count")
        .expect("confirm must record attachment-count evidence");
    assert_ne!(
        attachment_count.before, attachment_count.after,
        "confirm increases the attachment count by exactly one"
    );

    cancelled
        .validate()
        .expect("attachment cancel replay must expose one valid transition");
    assert_eq!(cancelled.before.screen_id, "attachment-picker");
    assert_eq!(cancelled.after.screen_id, "composer-multiline");
    assert_eq!(
        cancelled.before.draft, cancelled.after.draft,
        "cancel restores the original draft byte for byte"
    );
    assert_eq!(
        cancelled.before.attachment, cancelled.after.attachment,
        "cancel adds no attachment"
    );
    for field in [
        "draft",
        "selection",
        "focus",
        "composer_height",
        "scroll_anchor",
        "attachment",
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
fn public_selector_publishes_the_attachment_picker_pair() {
    if pure_mode() {
        let output = harness().arg("attachment-picker").output().expect("public selector runs");
        assert!(!output.status.success(), "pure-mode hybrid public publish must refuse");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("pure Cortex") || stderr.contains("pure-cortex"), "{stderr}");
        return;
    }
    let output = harness()
        .arg("attachment-picker")
        .output()
        .expect("public canonical selector must run");
    assert!(
        output.status.success(),
        "attachment-picker must publish after renderer preflight:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for extension in ["txt", "png"] {
        assert!(
            Path::new(&format!(
                "/projects/cortex-code/plans/assets/mockups-v3/input/attachment-picker.{extension}"
            ))
            .is_file(),
            "canonical attachment-picker.{extension} must publish"
        );
    }
}
