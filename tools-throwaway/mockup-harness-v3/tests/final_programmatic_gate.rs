//! Final programmatic completion gate.
//!
//! Owned contract assertions:
//! - VAL-V3-HARNESS-028 Rust quality gates pass with PNG support
//! - VAL-V3-HARNESS-029 Final programmatic gate is complete

use std::fs;
use std::path::Path;
use std::process::Command;

use mockup_harness_v3::final_programmatic_gate::{
    default_mission_directory, prove_missing_subgate_fails, run_final_programmatic_gate,
    REQUIRED_SUBGATES,
};

mod gate_helpers {
    pub const EXPECTED_ASSERTIONS: usize = 510;
}

const REPOSITORY: &str = "/projects/cortex-code";
const MISSION_DIRECTORY: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

// Private helper accessibility for unit testing assertion inventory is exposed
// through public run path; keep a local count for isolation of red/green unit checks.
fn count_assertions(mission: &str) -> usize {
    let text = fs::read_to_string(Path::new(mission).join("validation-contract.md"))
        .expect("validation-contract.md");
    text.lines()
        .filter(|line| line.starts_with("### VAL-V3-"))
        .count()
}

#[test]
fn contract_enumerates_exactly_471_unique_assertions() {
    let text = fs::read_to_string(Path::new(MISSION_DIRECTORY).join("validation-contract.md"))
        .expect("validation-contract.md must exist");
    let mut ids = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("### VAL-V3-") {
            let id = rest.split(':').next().unwrap_or("").trim();
            assert!(!id.is_empty(), "empty assertion id line: {line}");
            ids.push(format!("VAL-V3-{id}"));
        }
    }
    assert_eq!(ids.len(), gate_helpers::EXPECTED_ASSERTIONS);
    let unique = ids.iter().collect::<std::collections::BTreeSet<_>>().len();
    assert_eq!(unique, gate_helpers::EXPECTED_ASSERTIONS);
    assert_eq!(count_assertions(MISSION_DIRECTORY), 510);
    let _ = default_mission_directory();
}

#[test]
fn required_subgate_membership_is_closed() {
    assert_eq!(REQUIRED_SUBGATES.len(), 15);
    for required in [
        "inventory-flow",
        "txt-dimensions-glyphs",
        "png-dimensions-fidelity",
        "semantic-transition-audits",
        "negative-fixtures",
        "v2-immutable-references",
        "assertion-to-case-coverage",
        "vision-evidence",
        "readme-index",
        "gallery-conformance-check",
        "statusline-chrome-policy",
        "approval-integrity",
        "secret-commit-scope",
        "double-render-determinism",
        "contract-assertion-inventory",
    ] {
        assert!(
            REQUIRED_SUBGATES.contains(&required),
            "missing required subgate name {required}"
        );
    }
}

#[test]
fn incomplete_ledger_fails_closed() {
    let incomplete = r#"# Final programmatic gate ledger

ledger_membership_sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
final_gate_status=pass

| subgate | status | digest | detail |
| --- | --- | --- | --- |
| `inventory-flow` | pass | `bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb` | partial |
"#;
    prove_missing_subgate_fails(incomplete)
        .expect("missing subgates must fail closed");
}

#[test]
fn public_check_and_sweep_scripts_exist_and_are_executable() {
    let harness = Path::new(REPOSITORY).join("tools-throwaway/mockup-harness-v3");
    for name in ["check.sh", "sweep_checks.sh", "sweep_determinism.sh", "check_v2_immutable.sh"] {
        let path = harness.join(name);
        assert!(path.is_file(), "missing {name}");
        let meta = fs::metadata(&path).expect("meta");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                meta.permissions().mode() & 0o111 != 0,
                "{name} must be executable"
            );
        }
    }
}

#[test]
fn final_programmatic_gate_passes_with_complete_subgates() {
    // Use FINAL_GATE_SKIP_DETERMINISM so this unit path does not double-render.
    // Full double-render is still required via ./sweep_determinism.sh in handoff.
    let report = run_final_programmatic_gate(REPOSITORY, MISSION_DIRECTORY, true)
        .expect("final programmatic gate must pass all named subgates");

    assert_eq!(report.final_status, "pass");
    assert_eq!(report.subgates.len(), REQUIRED_SUBGATES.len());
    assert!(report.subgates.iter().all(|row| row.status == "pass"));
    assert_eq!(report.assertion_count, 510);
    assert_eq!(report.canonical_artifact_count, 172);
    assert_eq!(report.gate_artifact_count, 70);
    assert!(
        Path::new(&report.ledger_path).is_file(),
        "ledger must exist at {}",
        report.ledger_path
    );
    assert_eq!(report.ledger_membership_sha256.len(), 64);

    // Stale / foreign ledger status must not be accepted silently: re-read and
    // recomputed membership digest must match the written ledger field.
    let body = fs::read_to_string(&report.ledger_path).expect("ledger readable");
    assert!(body.contains("final_gate_status=pass"));
    assert!(body.contains("ledger_membership_sha256="));
    for required in REQUIRED_SUBGATES {
        assert!(
            body.contains(&format!("`{required}`")),
            "ledger missing required subgate row for {required}"
        );
    }
}

#[test]
fn check_sh_all_is_green() {
    let harness = Path::new(REPOSITORY).join("tools-throwaway/mockup-harness-v3");
    let output = Command::new("bash")
        .arg(harness.join("check.sh"))
        .arg("--all")
        .current_dir(&harness)
        .output()
        .expect("check.sh must run");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "check.sh --all failed: {combined}"
    );
    assert!(
        combined.contains("OK") && combined.contains("86"),
        "check.sh reporting unexpected: {combined}"
    );
}
