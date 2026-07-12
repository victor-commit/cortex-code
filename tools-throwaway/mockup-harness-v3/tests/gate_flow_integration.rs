use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use mockup_harness_v3::{
    audit::audit_screen,
    flow_manifests,
    gate_integration::{
        pilot_gate_artifacts, pilot_gate_bridges, pilot_gate_publications,
        prove_frame_review_hard_fails_when_non_current,
        prove_frame_review_hard_fails_without_provenance, validate_pilot_gate_integration,
        validate_pilot_gate_publications, validate_pilot_transition_contracts,
        validate_review_session_id,
    },
    scenario::render_audit_png,
};

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";
const CANONICAL_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const GATE_ROOT: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
static PUBLISHED: Once = Once::new();

fn ensure_published() {
    PUBLISHED.call_once(|| {
        let harness = env!("CARGO_BIN_EXE_mockup-harness-v3");
        for arguments in [
            vec!["--implemented"],
            vec!["--flow", "daily-work"],
            vec!["--flow", "input-and-approval"],
            vec!["--flow", "work-configuration"],
        ] {
            let output = Command::new(harness)
                .args(arguments)
                .output()
                .expect("the public renderer must publish pilot evidence");
            assert!(
                output.status.success(),
                "pilot publication failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    });
}

#[test]
fn first_three_manifests_join_to_the_published_gate_contract() {
    ensure_published();
    let report = validate_pilot_gate_integration(REPOSITORY, MISSION_DIRECTORY)
        .expect("the first three gate manifests must join to valid evidence");
    assert_eq!(
        report.flow_names,
        vec!["daily-work", "input-and-approval", "work-configuration",]
    );
    assert_eq!(report.step_count, 18);
    assert_eq!(report.artifact_count, 36);
    assert_eq!(report.bridge_count, 12);
    assert!(report.programmatic_checks_before_vision);
    assert_eq!(report.vision_findings.len(), 18);
    assert!(report.vision_findings.iter().all(|finding| {
        finding.record_is_current
            && finding.model == "anthropic/claude-opus-4.8-fast"
            && finding.role.starts_with("first-three gate frame ")
    }));
    assert!(
        !report.required_pilot_iteration,
        "every frame must have an exact path/role/hash/model-bound vision record"
    );
}

#[test]
fn persisted_integration_report_is_current_and_matches_authoritative_inventory() {
    ensure_published();
    let report = validate_pilot_gate_integration(REPOSITORY, MISSION_DIRECTORY)
        .expect("the persisted first-three integration report must be current");
    assert_eq!(report.step_count, 18);
    assert_eq!(report.artifact_count, 36);
    assert_eq!(report.bridge_count, pilot_gate_bridges().len());
    assert!(!report.required_pilot_iteration);
}

#[test]
fn placeholder_reviewer_sessions_are_rejected() {
    assert!(validate_review_session_id("dcc104c0-092e-4007-82ab-67cb01c8c76e").is_ok());
    for placeholder in [
        "unavailable-from-analyze-image-response",
        "placeholder-review-session",
        "unknown",
        "n/a",
    ] {
        assert!(
            validate_review_session_id(placeholder).is_err(),
            "placeholder reviewer session must fail closed: {placeholder}"
        );
    }
}

#[test]
fn frame_review_provenance_hard_fails_like_bridges_for_forged_and_non_current() {
    prove_frame_review_hard_fails_without_provenance()
        .expect("forged frame provenance must hard-fail, not soft-clear currency");
    prove_frame_review_hard_fails_when_non_current()
        .expect("non-current frame review must hard-fail, not only raise pilot iteration");
}

#[test]
fn publication_records_bind_every_alias_to_its_declared_fixture_and_origin() {
    let publications = pilot_gate_publications();
    assert_eq!(publications.len(), 18);
    validate_pilot_gate_publications(&publications)
        .expect("the authoritative first-three publication map must validate");
    assert!(publications.iter().all(|publication| {
        !publication.fixture_id.starts_with("canonical.")
            && !publication.origin.trim().is_empty()
            && publication.gate_txt.ends_with(".txt")
            && publication.gate_png.ends_with(".png")
    }));

    let mut fallback = publications.clone();
    fallback[0].fixture_id = "canonical.home".to_owned();
    assert!(validate_pilot_gate_publications(&fallback)
        .expect_err("canonical fixture fallback must fail")
        .contains("fixture"));

    let mut wrong_origin = publications;
    wrong_origin[0].origin = "chat-turn".to_owned();
    assert!(validate_pilot_gate_publications(&wrong_origin)
        .expect_err("an alias with the wrong declared origin must fail")
        .contains("origin"));
}

#[test]
fn every_declared_action_and_destination_matches_rendered_audit_or_named_bridge() {
    validate_pilot_transition_contracts()
        .expect("all direct, automatic, summarized, and illustrative contracts must reconcile");
}

#[test]
fn gate_aliases_are_exactly_the_declared_canonical_fixture_renderings() {
    ensure_published();
    let expected = pilot_gate_artifacts();
    let mut seen = BTreeSet::new();

    for artifact in expected {
        assert!(
            seen.insert(artifact.gate_relative.clone()),
            "duplicate gate path"
        );
        let canonical = Path::new(CANONICAL_ROOT).join(&artifact.canonical_relative);
        let gate = Path::new(GATE_ROOT).join(&artifact.gate_relative);
        assert!(
            canonical.is_file(),
            "missing canonical artifact: {canonical:?}"
        );
        assert!(gate.is_file(), "missing gate artifact: {gate:?}");
        let expected = audit_screen(artifact.screen_id, artifact.fixture_id)
            .expect("the declared canonical renderer and fixture must resolve");
        let expected_bytes = if artifact.extension == "txt" {
            expected.render_txt().into_bytes()
        } else {
            render_audit_png(&expected).expect("the declared canonical fixture must rasterize")
        };
        assert_eq!(
            expected_bytes,
            fs::read(&gate).expect("gate artifact must be readable"),
            "gate alias must be canonical-equivalent for its declared fixture: {gate:?}"
        );
    }
}

#[test]
fn every_gate_alias_is_byte_identical_to_its_declared_canonical_artifact() {
    ensure_published();
    for artifact in pilot_gate_artifacts() {
        let gate = fs::read(Path::new(GATE_ROOT).join(&artifact.gate_relative))
            .expect("declared gate alias must be readable");
        let expected = audit_screen(artifact.screen_id, artifact.fixture_id)
            .expect("the declared step fixture must resolve");
        let expected_bytes = if artifact.extension == "txt" {
            expected.render_txt().into_bytes()
        } else {
            render_audit_png(&expected).expect("the declared step fixture must rasterize")
        };
        assert_eq!(
            expected_bytes, gate,
            "every gate alias must match its declared step fixture: {}",
            artifact.gate_relative
        );
        let canonical_fixture =
            mockup_harness_v3::renderer::canonical_publication_fixture(artifact.screen_id)
                .unwrap_or(artifact.fixture_id);
        if canonical_fixture == artifact.fixture_id {
            let canonical =
                fs::read(Path::new(CANONICAL_ROOT).join(&artifact.canonical_relative))
                    .expect("declared canonical artifact must be readable");
            assert_eq!(
                canonical, gate,
                "gate alias must be a byte-identical canonical alias when fixtures match: {}",
                artifact.gate_relative
            );
        }
    }
}

#[test]
fn model_picker_canonical_and_gate_alias_compare_actual_on_disk_bytes() {
    ensure_published();
    for extension in ["txt", "png"] {
        let canonical = fs::read(format!("{CANONICAL_ROOT}/input/model-picker.{extension}"))
            .expect("published canonical model-picker artifact must be readable");
        let gate = fs::read(format!(
            "{GATE_ROOT}/work-configuration/02-model-picker.{extension}"
        ))
        .expect("published gate model-picker artifact must be readable");
        assert_eq!(
            canonical, gate,
            "canonical and gate model-picker {extension} must be byte-identical on disk"
        );
    }
}

#[test]
fn twelve_bridges_require_persisted_txt_png_audit_and_frame_specific_opus_records() {
    assert_eq!(pilot_gate_bridges().len(), 12);
    for bridge in pilot_gate_bridges() {
        assert!(!bridge.evidence_txt.is_empty());
        assert!(!bridge.evidence_png.is_empty());
        assert!(!bridge.audit_json.is_empty());
        assert!(!bridge.vision_evidence.is_empty());
        assert!(bridge.role.starts_with("first-three bridge "));
    }
}

#[test]
fn first_three_gate_directories_have_only_ordered_txt_png_pairs() {
    ensure_published();
    for flow in flow_manifests().iter().take(3) {
        let directory = PathBuf::from(GATE_ROOT).join(flow.name);
        let mut actual = fs::read_dir(&directory)
            .expect("published gate directory must exist")
            .map(|entry| {
                entry
                    .expect("gate directory entry must be readable")
                    .file_name()
            })
            .collect::<Vec<_>>();
        actual.sort();
        let expected = pilot_gate_artifacts()
            .iter()
            .filter(|artifact| {
                artifact
                    .gate_relative
                    .starts_with(&format!("{}/", flow.name))
            })
            .map(|artifact| {
                artifact
                    .gate_relative
                    .split_once('/')
                    .expect("flow prefix")
                    .1
                    .to_owned()
            })
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>();
        let mut expected = expected;
        expected.sort();
        assert_eq!(
            actual, expected,
            "gate artifacts must be ordered and complete"
        );
    }
}
