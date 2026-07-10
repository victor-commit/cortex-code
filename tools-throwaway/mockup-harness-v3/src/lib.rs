//! Immutable reference and boundary checks for the isolated v3 harness.

use std::path::Path;
use std::process::Command;

pub mod flows;
pub mod registry;

pub use flows::{
    flow_manifests, flow_owner, validate_flow_manifests, FlowManifest, FlowStep, TransitionKind,
};
pub use registry::{
    canonical_inventory, validate_inventory, Category, FlowOwner, ImplementationStatus, ScreenSpec,
    Surface,
};

fn run_check(repository: &Path, script_name: &str) -> Result<(), String> {
    let harness = repository.join("tools-throwaway/mockup-harness-v3");
    let output = Command::new("bash")
        .arg(harness.join(script_name))
        .current_dir(&harness)
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
    run_check(repository, "check_v2_immutable.sh")
}

/// Proves all non-v3 paths retain the status present when v3 began.
pub fn verify_forbidden_paths(repository: &Path) -> Result<(), String> {
    run_check(repository, "check_forbidden_paths.sh")
}

/// Validates the public inventory and journey manifests before any renderer
/// can use them.
pub fn validate_canonical_map() -> Result<(), String> {
    validate_inventory(canonical_inventory())?;
    validate_flow_manifests(flow_manifests(), canonical_inventory())
}
