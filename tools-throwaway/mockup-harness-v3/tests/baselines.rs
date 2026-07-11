use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn active_session_id() -> String {
    let runtime_settings = env::var_os("FACTORY_RUNTIME_SETTINGS_PATH")
        .expect("Factory runtime settings path must be available");
    let state_dir = PathBuf::from(runtime_settings)
        .parent()
        .expect("Factory runtime settings must have a parent directory")
        .join("isolation-guard-v3");
    fs::read_to_string(state_dir.join("current-session"))
        .expect("active worker-session identity must be recorded")
}

#[test]
fn frozen_v1_v2_references_match_the_recorded_manifest() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    mockup_harness_v3::verify_frozen_references(&repository)
        .expect("frozen v1/v2 references must match the immutable baseline");
}

#[test]
fn mission_commits_stay_within_approved_paths() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let session_id = active_session_id();

    mockup_harness_v3::verify_mission_commit_paths(&repository, session_id.trim())
        .expect("mission-authored commits must stay within approved paths");
}

#[test]
fn forbidden_paths_match_the_active_session_snapshot() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let session_id = active_session_id();

    mockup_harness_v3::verify_forbidden_paths(&repository, session_id.trim())
        .expect("forbidden paths must match the active worker-session snapshot");
}
