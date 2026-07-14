//! Immutable reference and boundary checks for the isolated v3 harness.

pub mod audit;
pub mod chrome;
pub mod negative_fixtures;
pub mod renderer;
pub mod scenario;
pub mod screens;
use std::path::Path;
use std::process::Command;

pub mod flows;
pub mod gate_approval;
pub mod gate_integration;
pub mod gate_programmatic_publication;
pub mod gate_remaining_integration;
pub mod gate_vision_review;
pub mod final_canonical_vision;
pub mod final_secret_commit_scope;
pub mod registry;

pub use flows::{
    flow_manifests, flow_owner, validate_flow_manifests, FlowManifest, FlowStep, TransitionKind,
};
pub use registry::{
    canonical_inventory, validate_inventory, Category, FlowOwner, ImplementationStatus, ScreenSpec,
    Surface,
};

fn run_check(repository: &Path, script_name: &str, session_id: Option<&str>) -> Result<(), String> {
    let harness = repository.join("tools-throwaway/mockup-harness-v3");
    let mut command = Command::new("bash");
    command.arg(harness.join(script_name)).current_dir(&harness);
    if let Some(session_id) = session_id {
        command.env("V3_ISOLATION_SESSION_ID", session_id);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not start {script_name}: {error}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "{script_name} exited with {status}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        status = output.status
    ))
}

/// Proves every tracked file in the frozen v1/v2 roots still has its recorded
/// SHA-256 digest.
pub fn verify_frozen_references(repository: &Path) -> Result<(), String> {
    run_check(repository, "check_v2_immutable.sh", None)
}

/// Proves no newly introduced uncommitted forbidden-path change exists during
/// this worker session.
pub fn verify_forbidden_paths(repository: &Path, session_id: &str) -> Result<(), String> {
    run_check(repository, "check_forbidden_paths.sh", Some(session_id))
}

/// Audits exactly the commits recorded in the append-only mission ledger.
pub fn verify_mission_commit_paths(repository: &Path, session_id: &str) -> Result<(), String> {
    run_check(repository, "audit_mission_commits.sh", Some(session_id))
}

/// Validates the public inventory and journey manifests before any renderer
/// can use them.
pub fn validate_canonical_map() -> Result<(), String> {
    validate_inventory(canonical_inventory())?;
    validate_flow_manifests(flow_manifests(), canonical_inventory())
}
