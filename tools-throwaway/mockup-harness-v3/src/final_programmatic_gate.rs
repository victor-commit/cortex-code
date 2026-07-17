//! Final programmatic completion gate (VAL-V3-HARNESS-028, VAL-V3-HARNESS-029).
//!
//! Aggregates the hash-bound subgates that must all pass before the mission is
//! treated as programmatically complete. Individual journey features leave
//! focused aggregate tests; this module proves the full final surface:
//! inventory/flow, TXT/PNG fidelity, semantic/transition audit, negative
//! fixtures, immutable v2 references, assertion-to-case coverage, vision
//! evidence, README index, programmatic check.sh, statusline policy,
//! approval integrity, secret/commit/path scope, contractual 493-assertion
//! inventory, and double-render determinism via the public scripts.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::audit::sha256_hex;

/// Named final subgates required by VAL-V3-HARNESS-029.
pub const REQUIRED_SUBGATES: &[&str] = &[
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
];

const CANONICAL_GALLERY: &str = "/projects/cortex-code/plans/assets/mockups-v3";
const GATE_GALLERY: &str = "/projects/cortex-code/plans/assets/mockups-v3-gates";
/// Exact validation-contract assertion inventory for the final aggregate.
/// Pure Cortex dump added DS-038..040, CROSS-024, HARNESS-042 (481 + 5 = 486).
/// Pure-structure shell added STRUCT-001..006, DS-039, CROSS-025 (486 + 7 = 493).
pub const CONTRACT_ASSERTION_COUNT: usize = 493;
const DEFAULT_MISSION: &str = "/root/.factory/missions/7060a79b-ccfb-445c-9e82-ee4504b320dc";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubgateResult {
    pub name: String,
    pub status: String,
    pub digest: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalProgrammaticGateReport {
    pub subgates: Vec<SubgateResult>,
    pub ledger_path: String,
    pub ledger_membership_sha256: String,
    pub assertion_count: usize,
    pub canonical_artifact_count: usize,
    pub gate_artifact_count: usize,
    pub final_status: String,
}

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repository_root() -> PathBuf {
    harness_dir().join("../..")
}

fn run_capturing(command: &mut Command) -> Result<(i32, String), String> {
    let output = command
        .output()
        .map_err(|error| format!("could not start {:?}: {error}", command))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    let code = output.status.code().unwrap_or(1);
    Ok((code, combined))
}

fn count_txt_png(root: &Path) -> Result<usize, String> {
    let mut count = 0usize;
    if !root.exists() {
        return Err(format!("gallery root missing: {}", root.display()));
    }
    fn walk(path: &Path, count: &mut usize) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|e| format!("read {}: {e}", path.display()))? {
            let entry = entry.map_err(|e| format!("entry: {e}"))?;
            let p = entry.path();
            if p.is_dir() {
                walk(&p, count)?;
            } else if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".txt") || name.ends_with(".png") {
                    *count += 1;
                }
            }
        }
        Ok(())
    }
    walk(root, &mut count)?;
    Ok(count)
}

fn parse_ledger(path: &Path) -> Result<(BTreeMap<String, SubgateResult>, String, String), String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read ledger {}: {error}", path.display()))?;
    let mut map = BTreeMap::new();
    let mut membership = String::new();
    let mut status = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ledger_membership_sha256=") {
            membership = rest.trim().to_owned();
        }
        if let Some(rest) = line.strip_prefix("final_gate_status=") {
            status = rest.trim().to_owned();
        }
        // Table rows: | `name` | status | `digest` | detail |
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || trimmed.contains("---") || trimmed.contains("subgate") {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let name = cells[0].trim_matches('`').to_owned();
        let status_cell = cells[1].to_owned();
        let digest = cells[2].trim_matches('`').to_owned();
        let detail = cells[3].to_owned();
        if name.is_empty() || status_cell.is_empty() {
            continue;
        }
        map.insert(
            name.clone(),
            SubgateResult {
                name,
                status: status_cell,
                digest,
                detail,
            },
        );
    }
    if membership.is_empty() {
        return Err("ledger missing ledger_membership_sha256".to_owned());
    }
    if status.is_empty() {
        return Err("ledger missing final_gate_status".to_owned());
    }
    Ok((map, membership, status))
}

fn count_contract_assertions(mission_directory: &str) -> Result<usize, String> {
    let path = Path::new(mission_directory).join("validation-contract.md");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut ids = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("### VAL-V3-") {
            let id = rest
                .split(':')
                .next()
                .map(|s| format!("VAL-V3-{}", s.trim()))
                .unwrap_or_default();
            if !id.is_empty() {
                ids.push(id);
            }
        }
    }
    let unique = ids.iter().collect::<std::collections::BTreeSet<_>>().len();
    if ids.len() != unique {
        return Err(format!(
            "validation-contract has duplicate assertions ({} rows, {} unique)",
            ids.len(),
            unique
        ));
    }
    Ok(ids.len())
}

/// Runs the public `./sweep_checks.sh` completion gate and validates the ledger.
pub fn run_final_programmatic_gate(
    repository: &str,
    mission_directory: &str,
    skip_determinism_render: bool,
) -> Result<FinalProgrammaticGateReport, String> {
    let harness = Path::new(repository).join("tools-throwaway/mockup-harness-v3");
    let script = harness.join("sweep_checks.sh");
    if !script.is_file() {
        return Err(format!(
            "missing final gate script at {}",
            script.display()
        ));
    }

    let mut command = Command::new("bash");
    command
        .arg(&script)
        .current_dir(&harness)
        .env("V3_MISSION_DIR", mission_directory)
        .env("V3_REPO_ROOT", repository);
    // Propagate the active isolation session so secret-commit-scope (and any
    // nested cargo tests) bind the same worker snapshot as the rest of the gate.
    if let Ok(session) = std::env::var("V3_ISOLATION_SESSION_ID") {
        if !session.trim().is_empty() {
            command.env("V3_ISOLATION_SESSION_ID", session);
        }
    } else {
        let current = Path::new(mission_directory).join("isolation-guard-v3/current-session");
        if let Ok(raw) = fs::read_to_string(&current) {
            let session = raw.trim().to_owned();
            if !session.is_empty() {
                command.env("V3_ISOLATION_SESSION_ID", session);
            }
        }
    }
    if skip_determinism_render {
        command.env("FINAL_GATE_SKIP_DETERMINISM", "1");
    } else {
        command.env("FINAL_GATE_SKIP_DETERMINISM", "0");
    }

    let (code, combined) = run_capturing(&mut command)?;
    let ledger_path = Path::new(mission_directory)
        .join("evidence/v3-final-programmatic-gate/final-gate-ledger.md");
    if !ledger_path.is_file() {
        return Err(format!(
            "sweep_checks.sh exited {code} without writing ledger; output:\n{combined}"
        ));
    }

    let (map, membership, status) = parse_ledger(&ledger_path)?;
    let mut subgates = Vec::new();
    let mut missing = Vec::new();
    for required in REQUIRED_SUBGATES {
        match map.get(*required) {
            Some(row) => {
                if row.status != "pass" {
                    return Err(format!(
                        "subgate {required} status={}, detail={}",
                        row.status, row.detail
                    ));
                }
                subgates.push(row.clone());
            }
            None => missing.push((*required).to_owned()),
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "final gate ledger missing required subgates: {}",
            missing.join(", ")
        ));
    }
    if code != 0 {
        return Err(format!(
            "sweep_checks.sh exited {code}; ledger status={status}; output tail:\n{}",
            combined.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>()
        ));
    }
    if status != "pass" {
        return Err(format!("final_gate_status is {status}, expected pass"));
    }

    let assertion_count = count_contract_assertions(mission_directory)?;
    if assertion_count != CONTRACT_ASSERTION_COUNT {
        return Err(format!(
            "expected {CONTRACT_ASSERTION_COUNT} contract assertions, found {assertion_count}"
        ));
    }

    let canonical_artifact_count = count_txt_png(Path::new(CANONICAL_GALLERY))?;
    let gate_artifact_count = count_txt_png(Path::new(GATE_GALLERY))?;
    if canonical_artifact_count != 172 {
        return Err(format!(
            "canonical artifact count {canonical_artifact_count} != 172"
        ));
    }
    if gate_artifact_count != 70 {
        return Err(format!(
            "gate artifact count {gate_artifact_count} != 70"
        ));
    }

    Ok(FinalProgrammaticGateReport {
        subgates,
        ledger_path: ledger_path.display().to_string(),
        ledger_membership_sha256: membership,
        assertion_count,
        canonical_artifact_count,
        gate_artifact_count,
        final_status: status,
    })
}

/// Verifies that a skipped required subgate makes validation fail closed.
pub fn prove_missing_subgate_fails(ledger_body: &str) -> Result<(), String> {
    let temp = std::env::temp_dir().join(format!(
        "v3-final-gate-missing-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&temp).map_err(|e| format!("temp dir: {e}"))?;
    let path = temp.join("final-gate-ledger.md");
    fs::write(&path, ledger_body).map_err(|e| format!("write temp ledger: {e}"))?;
    let (map, _, status) = parse_ledger(&path)?;
    let _ = fs::remove_dir_all(&temp);
    for required in REQUIRED_SUBGATES {
        if !map.contains_key(*required) {
            return Ok(());
        }
        if map
            .get(*required)
            .map(|row| row.status.as_str())
            != Some("pass")
        {
            return Ok(());
        }
    }
    if status != "pass" {
        return Ok(());
    }
    Err("ledger still reports full pass with incomplete subgates".to_owned())
}

/// Runs cargo test / build / clippy with PNG features for VAL-V3-HARNESS-028.
pub fn run_rust_quality_gates(repository: &str) -> Result<(String, String, String), String> {
    let harness = Path::new(repository).join("tools-throwaway/mockup-harness-v3");
    let mut test_cmd = Command::new("cargo");
    test_cmd
        .args(["test", "--features", "png"])
        .current_dir(&harness)
        .env("CARGO_INCREMENTAL", "1")
        .env("CARGO_BUILD_JOBS", "16");
    let (test_code, test_out) = run_capturing(&mut test_cmd)?;
    if test_code != 0 {
        return Err(format!("cargo test --features png failed:\n{test_out}"));
    }

    let mut build_cmd = Command::new("cargo");
    build_cmd
        .args(["build", "--features", "png"])
        .current_dir(&harness)
        .env("CARGO_INCREMENTAL", "1")
        .env("CARGO_BUILD_JOBS", "16");
    let (build_code, build_out) = run_capturing(&mut build_cmd)?;
    if build_code != 0 {
        return Err(format!("cargo build --features png failed:\n{build_out}"));
    }

    let mut clippy_cmd = Command::new("cargo");
    clippy_cmd
        .args([
            "clippy",
            "--all-targets",
            "--features",
            "png",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(&harness)
        .env("CARGO_INCREMENTAL", "1")
        .env("CARGO_BUILD_JOBS", "16");
    let (clippy_code, clippy_out) = run_capturing(&mut clippy_cmd)?;
    if clippy_code != 0 {
        return Err(format!(
            "cargo clippy --all-targets --features png -- -D warnings failed:\n{clippy_out}"
        ));
    }

    Ok((
        sha256_hex(test_out.as_bytes()),
        sha256_hex(build_out.as_bytes()),
        sha256_hex(clippy_out.as_bytes()),
    ))
}

/// Default mission directory for this multi-agent mission.
pub fn default_mission_directory() -> &'static str {
    DEFAULT_MISSION
}

/// Default repository root for this multi-agent mission.
pub fn default_repository() -> PathBuf {
    repository_root()
}
