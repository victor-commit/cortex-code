use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use mockup_harness_v3::{
    gate_integration::{
        pilot_gate_artifacts, pilot_gate_bridges, validate_pilot_gate_integration,
    },
    flow_manifests,
};

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str =
    "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";
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
    assert_eq!(report.flow_names, vec![
        "daily-work",
        "input-and-approval",
        "work-configuration",
    ]);
    assert_eq!(report.step_count, 18);
    assert_eq!(report.artifact_count, 36);
    assert_eq!(report.bridge_count, pilot_gate_bridges().len());
    assert!(report.programmatic_checks_before_vision);
    assert_eq!(report.vision_findings.len(), 18);
    assert!(
        report
            .vision_findings
            .iter()
            .filter(|finding| !finding.evidence_contains_current_hash)
            .count()
            == 2,
        "the known user-edited pilot frames must be the only stale vision findings"
    );
    assert!(
        report.required_pilot_iteration,
        "stale vision hashes must require a pilot iteration rather than being reused"
    );
}

#[test]
fn gate_aliases_are_exactly_the_canonical_artifacts_for_first_three_flows() {
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
        assert!(canonical.is_file(), "missing canonical artifact: {canonical:?}");
        assert!(gate.is_file(), "missing gate artifact: {gate:?}");
        assert_eq!(
            fs::read(&canonical).expect("canonical artifact must be readable"),
            fs::read(&gate).expect("gate artifact must be readable"),
            "gate alias must be byte-identical: {gate:?}"
        );
    }
}

#[test]
fn first_three_gate_directories_have_only_ordered_txt_png_pairs() {
    ensure_published();
    for flow in flow_manifests()
        .iter()
        .take(3)
    {
        let directory = PathBuf::from(GATE_ROOT).join(flow.name);
        let mut actual = fs::read_dir(&directory)
            .expect("published gate directory must exist")
            .map(|entry| entry.expect("gate directory entry must be readable").file_name())
            .collect::<Vec<_>>();
        actual.sort();
        let expected = pilot_gate_artifacts()
            .iter()
            .filter(|artifact| artifact.gate_relative.starts_with(&format!("{}/", flow.name)))
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
        assert_eq!(actual, expected, "gate artifacts must be ordered and complete");
    }
}
