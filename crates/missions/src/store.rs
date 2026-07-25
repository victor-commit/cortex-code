//! missionDir filesystem store under a configurable missions root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::{
    Feature, FeatureStatus, FeaturesFile, MissionState, ValidationAssertionState,
    ValidationStateFile,
};
use crate::scheduler::ProgressEntry;

/// Errors from the mission store.
#[derive(Debug, Error)]
pub enum StoreError {
    /// I/O failure.
    #[error("io: {0}")]
    Io(#[from] io::Error),
    /// JSON encode/decode failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Missing artifact.
    #[error("not found: {0}")]
    NotFound(String),
    /// Invalid argument.
    #[error("invalid: {0}")]
    Invalid(String),
}

/// Resolved paths for one mission.
#[derive(Debug, Clone)]
pub struct MissionPaths {
    /// Mission id (directory name).
    pub id: String,
    /// Absolute path to the mission directory.
    pub root: PathBuf,
}

impl MissionPaths {
    /// `features.json`
    #[must_use]
    pub fn features_json(&self) -> PathBuf {
        self.root.join("features.json")
    }
    /// `state.json`
    #[must_use]
    pub fn state_json(&self) -> PathBuf {
        self.root.join("state.json")
    }
    /// `validation-state.json`
    #[must_use]
    pub fn validation_state_json(&self) -> PathBuf {
        self.root.join("validation-state.json")
    }
    /// `validation-contract.md`
    #[must_use]
    pub fn validation_contract_md(&self) -> PathBuf {
        self.root.join("validation-contract.md")
    }
    /// `mission.md`
    #[must_use]
    pub fn mission_md(&self) -> PathBuf {
        self.root.join("mission.md")
    }
    /// `AGENTS.md`
    #[must_use]
    pub fn agents_md(&self) -> PathBuf {
        self.root.join("AGENTS.md")
    }
    /// `services.yaml`
    #[must_use]
    pub fn services_yaml(&self) -> PathBuf {
        self.root.join("services.yaml")
    }
    /// `progress_log.jsonl`
    #[must_use]
    pub fn progress_log(&self) -> PathBuf {
        self.root.join("progress_log.jsonl")
    }
    /// `handoffs/`
    #[must_use]
    pub fn handoffs_dir(&self) -> PathBuf {
        self.root.join("handoffs")
    }
    /// `working_directory.txt`
    #[must_use]
    pub fn working_directory_txt(&self) -> PathBuf {
        self.root.join("working_directory.txt")
    }
}

/// Options for creating a mission directory.
#[derive(Debug, Clone)]
pub struct CreateMissionOpts {
    /// Optional explicit id; generated when `None`.
    pub id: Option<String>,
    /// Short title written into mission.md.
    pub title: String,
    /// Parallel max (clamped later by scheduler).
    pub parallel_max: usize,
    /// Initial features (may be empty).
    pub features: Vec<Feature>,
}

impl Default for CreateMissionOpts {
    fn default() -> Self {
        Self {
            id: None,
            title: "untitled mission".into(),
            parallel_max: 3,
            features: Vec::new(),
        }
    }
}

/// Summary returned after create/list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSummary {
    /// Mission id.
    pub id: String,
    /// Mission directory.
    pub mission_dir: String,
    /// Feature count.
    pub feature_count: usize,
    /// Status from state.json.
    pub status: String,
}

/// Create a missionDir under `root_base/<id>/` with the standard layout.
pub fn create_mission(
    root_base: &Path,
    opts: CreateMissionOpts,
) -> Result<MissionPaths, StoreError> {
    let id = opts
        .id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(generate_mission_id);
    if id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(StoreError::Invalid(format!("bad mission id: {id}")));
    }
    let root = root_base.join(&id);
    if root.exists() {
        return Err(StoreError::Invalid(format!(
            "mission already exists: {}",
            root.display()
        )));
    }
    fs::create_dir_all(root.join("handoffs"))?;
    fs::create_dir_all(root.join("library"))?;
    fs::create_dir_all(root.join("skills"))?;
    fs::create_dir_all(root.join("validation"))?;

    let paths = MissionPaths {
        id: id.clone(),
        root: root.clone(),
    };

    let mission_md = format!(
        "# {}\n\nClean-room Cortex mission.\n\nId: `{id}`\n",
        opts.title
    );
    fs::write(paths.mission_md(), mission_md)?;
    fs::write(
        paths.agents_md(),
        "# AGENTS\n\nMission boundaries for Cortex workers.\n",
    )?;
    fs::write(
        paths.validation_contract_md(),
        "# Validation Contract\n\n## VAL-FIXTURE-001\nFixture assertion placeholder.\n",
    )?;
    fs::write(
        paths.services_yaml(),
        "# commands for milestone validators (fixture)\ncommands:\n  test: \"true\"\n  lint: \"true\"\n",
    )?;
    fs::write(
        paths.working_directory_txt(),
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".into()),
    )?;

    let features = FeaturesFile {
        features: opts.features,
    };
    save_features(&paths, &features)?;

    let mut assertions = Vec::new();
    for f in &features.features {
        for id in &f.fulfills {
            assertions.push(ValidationAssertionState {
                id: id.clone(),
                status: "pending".into(),
            });
        }
    }
    if assertions.is_empty() {
        assertions.push(ValidationAssertionState {
            id: "VAL-FIXTURE-001".into(),
            status: "pending".into(),
        });
    }
    save_validation_state(
        &paths,
        &ValidationStateFile { assertions },
    )?;

    save_mission_state(
        &paths,
        &MissionState {
            id: id.clone(),
            status: "created".into(),
            parallel_max: opts.parallel_max.max(1),
            live_workers: Vec::new(),
            validation_invoked: false,
            last_validation: None,
        },
    )?;

    // Touch empty progress log.
    fs::write(paths.progress_log(), b"")?;

    Ok(paths)
}

/// Create the standard parallel fixture mission (≥3 independent features; one can fail).
///
/// Features `f-a`, `f-b`, `f-c` are independent (no deps). When `with_failure` is used via
/// [`create_isolation_fixture_mission`], `f-b` is marked to fail.
pub fn create_fixture_mission(
    root_base: &Path,
    mut opts: CreateMissionOpts,
) -> Result<MissionPaths, StoreError> {
    if opts.features.is_empty() {
        opts.features = default_fixture_features(false);
    }
    create_mission(root_base, opts)
}

/// Fixture with one failing feature for isolation tests.
pub fn create_isolation_fixture_mission(
    root_base: &Path,
    id: &str,
) -> Result<MissionPaths, StoreError> {
    create_mission(
        root_base,
        CreateMissionOpts {
            id: Some(id.into()),
            title: "isolation fixture".into(),
            parallel_max: 3,
            features: default_fixture_features(true),
        },
    )
}

fn default_fixture_features(with_failure: bool) -> Vec<Feature> {
    vec![
        Feature {
            id: "f-a".into(),
            description: "fixture feature A".into(),
            skill_name: "fixture-worker".into(),
            milestone: "m-fixture".into(),
            preconditions: vec![],
            depends_on: vec![],
            expected_behavior: vec!["completes".into()],
            fulfills: vec!["VAL-FIXTURE-A".into()],
            status: FeatureStatus::Pending,
            fail: false,
            work_ms: 80,
        },
        Feature {
            id: "f-b".into(),
            description: "fixture feature B".into(),
            skill_name: "fixture-worker".into(),
            milestone: "m-fixture".into(),
            preconditions: vec![],
            depends_on: vec![],
            expected_behavior: vec!["completes or fails in isolation".into()],
            fulfills: vec!["VAL-FIXTURE-B".into()],
            status: FeatureStatus::Pending,
            fail: with_failure,
            work_ms: 100,
        },
        Feature {
            id: "f-c".into(),
            description: "fixture feature C".into(),
            skill_name: "fixture-worker".into(),
            milestone: "m-fixture".into(),
            preconditions: vec![],
            depends_on: vec![],
            expected_behavior: vec!["completes".into()],
            fulfills: vec!["VAL-FIXTURE-C".into()],
            status: FeatureStatus::Pending,
            fail: false,
            work_ms: 90,
        },
    ]
}

/// Open an existing mission directory.
pub fn open_mission(root_base: &Path, id: &str) -> Result<MissionPaths, StoreError> {
    let root = root_base.join(id);
    if !root.is_dir() {
        return Err(StoreError::NotFound(root.display().to_string()));
    }
    Ok(MissionPaths {
        id: id.to_owned(),
        root,
    })
}

/// Open by absolute mission dir path.
pub fn open_mission_dir(dir: &Path) -> Result<MissionPaths, StoreError> {
    if !dir.is_dir() {
        return Err(StoreError::NotFound(dir.display().to_string()));
    }
    let id = dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| StoreError::Invalid("mission dir has no name".into()))?
        .to_owned();
    Ok(MissionPaths {
        id,
        root: dir.to_path_buf(),
    })
}

/// Load features.json.
pub fn load_features(paths: &MissionPaths) -> Result<FeaturesFile, StoreError> {
    let raw = fs::read_to_string(paths.features_json())?;
    Ok(serde_json::from_str(&raw)?)
}

/// Save features.json.
pub fn save_features(paths: &MissionPaths, features: &FeaturesFile) -> Result<(), StoreError> {
    let body = serde_json::to_string_pretty(features)?;
    fs::write(paths.features_json(), body)?;
    Ok(())
}

/// Load state.json.
pub fn load_mission_state(paths: &MissionPaths) -> Result<MissionState, StoreError> {
    let raw = fs::read_to_string(paths.state_json())?;
    Ok(serde_json::from_str(&raw)?)
}

/// Save state.json.
pub fn save_mission_state(paths: &MissionPaths, state: &MissionState) -> Result<(), StoreError> {
    let body = serde_json::to_string_pretty(state)?;
    fs::write(paths.state_json(), body)?;
    Ok(())
}

/// Load validation-state.json.
pub fn load_validation_state(paths: &MissionPaths) -> Result<ValidationStateFile, StoreError> {
    let raw = fs::read_to_string(paths.validation_state_json())?;
    Ok(serde_json::from_str(&raw)?)
}

/// Save validation-state.json.
pub fn save_validation_state(
    paths: &MissionPaths,
    state: &ValidationStateFile,
) -> Result<(), StoreError> {
    let body = serde_json::to_string_pretty(state)?;
    fs::write(paths.validation_state_json(), body)?;
    Ok(())
}

/// Append one progress log entry (jsonl).
pub fn append_progress(paths: &MissionPaths, entry: &ProgressEntry) -> Result<(), StoreError> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.progress_log())?;
    let line = serde_json::to_string(entry)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Read all progress log entries.
pub fn read_progress_log(paths: &MissionPaths) -> Result<Vec<ProgressEntry>, StoreError> {
    let raw = match fs::read_to_string(paths.progress_log()) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.push(serde_json::from_str(line)?);
    }
    Ok(out)
}

/// List mission summaries under a root.
pub fn list_missions(root_base: &Path) -> Result<Vec<MissionSummary>, StoreError> {
    if !root_base.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(root_base)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let paths = MissionPaths {
            id: name.clone(),
            root: entry.path(),
        };
        if !paths.features_json().exists() {
            continue;
        }
        let features = load_features(&paths).unwrap_or_default();
        let status = load_mission_state(&paths)
            .map(|s| s.status)
            .unwrap_or_else(|_| "unknown".into());
        out.push(MissionSummary {
            id: name,
            mission_dir: entry.path().display().to_string(),
            feature_count: features.features.len(),
            status,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn generate_mission_id() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    // UUID-ish hex without requiring uuid crate.
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Current unix millis (shared helper).
#[must_use]
#[allow(dead_code)]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_mission_writes_store_layout() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("demo-1".into()),
                title: "Demo".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        assert!(paths.root.is_dir());
        assert!(paths.features_json().is_file());
        assert!(paths.state_json().is_file());
        assert!(paths.validation_state_json().is_file());
        assert!(paths.validation_contract_md().is_file());
        assert!(paths.mission_md().is_file());
        assert!(paths.handoffs_dir().is_dir());
        assert!(paths.progress_log().is_file());
        let features = load_features(&paths).unwrap();
        assert!(features.features.len() >= 3);
        let state = load_mission_state(&paths).unwrap();
        assert_eq!(state.status, "created");
        assert_eq!(state.id, "demo-1");
    }

    #[test]
    fn create_does_not_use_factory_path() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("safe".into()),
                title: "safe".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        let s = paths.root.display().to_string();
        assert!(
            !s.contains(".factory/missions"),
            "must never write under Factory missionDirs: {s}"
        );
    }
}
