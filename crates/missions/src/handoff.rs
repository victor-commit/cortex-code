//! EndFeature-like handoff JSON written under `handoffs/`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::schema::SuccessState;
use crate::store::{StoreError, MissionPaths};

/// One command recorded in a handoff verification section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCommand {
    /// Shell command.
    pub command: String,
    /// Exit code.
    pub exit_code: i32,
    /// Observation text.
    pub observation: String,
}

/// Verification block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HandoffVerification {
    /// Commands run during the feature.
    #[serde(default, rename = "commandsRun")]
    pub commands_run: Vec<VerificationCommand>,
}

/// Tests block (lightweight).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HandoffTests {
    /// Coverage summary.
    #[serde(default)]
    pub coverage: String,
}

/// Structured handoff written on feature completion (Factory EndFeatureRun spirit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    /// Feature id this handoff belongs to.
    pub feature_id: String,
    /// Worker slot id.
    pub worker_id: String,
    /// Success / failure / partial.
    #[serde(rename = "successState")]
    pub success_state: SuccessState,
    /// Short summary.
    #[serde(rename = "salientSummary")]
    pub salient_summary: String,
    /// What was implemented.
    #[serde(rename = "whatWasImplemented")]
    pub what_was_implemented: String,
    /// What remains.
    #[serde(default, rename = "whatWasLeftUndone")]
    pub what_was_left_undone: String,
    /// Verification block.
    #[serde(default)]
    pub verification: HandoffVerification,
    /// Tests block.
    #[serde(default)]
    pub tests: HandoffTests,
    /// Whether control should return to the orchestrator.
    #[serde(default, rename = "returnToOrchestrator")]
    pub return_to_orchestrator: bool,
}

/// Write handoff JSON to `handoffs/<feature_id>.json`.
pub fn write_handoff(paths: &MissionPaths, handoff: &Handoff) -> Result<std::path::PathBuf, StoreError> {
    let dir = paths.handoffs_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", handoff.feature_id));
    let body = serde_json::to_string_pretty(handoff)?;
    fs::write(&path, body)?;
    // Also record a pointer file for "latest" listing.
    let _ = fs::write(dir.join(".latest"), handoff.feature_id.as_bytes());
    Ok(path)
}

/// Load a handoff if present.
pub fn load_handoff(paths: &MissionPaths, feature_id: &str) -> Result<Handoff, StoreError> {
    let path = paths.handoffs_dir().join(format!("{feature_id}.json"));
    let raw = fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            StoreError::NotFound(path.display().to_string())
        } else {
            StoreError::Io(e)
        }
    })?;
    Ok(serde_json::from_str(&raw)?)
}

/// Ensure handoffs directory exists.
pub fn ensure_handoffs_dir(root: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(root.join("handoffs"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{create_fixture_mission, CreateMissionOpts};
    use tempfile::tempdir;

    #[test]
    fn write_and_load_handoff_roundtrip() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("h1".into()),
                title: "handoff".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        let h = Handoff {
            feature_id: "f-a".into(),
            worker_id: "#21".into(),
            success_state: SuccessState::Success,
            salient_summary: "did the thing".into(),
            what_was_implemented: "feature a".into(),
            what_was_left_undone: String::new(),
            verification: HandoffVerification {
                commands_run: vec![VerificationCommand {
                    command: "true".into(),
                    exit_code: 0,
                    observation: "ok".into(),
                }],
            },
            tests: HandoffTests {
                coverage: "unit".into(),
            },
            return_to_orchestrator: false,
        };
        let path = write_handoff(&paths, &h).unwrap();
        assert!(path.exists());
        let loaded = load_handoff(&paths, "f-a").unwrap();
        assert_eq!(loaded.feature_id, "f-a");
        assert_eq!(loaded.success_state, SuccessState::Success);
        assert_eq!(loaded.worker_id, "#21");
    }
}
