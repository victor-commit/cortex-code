//! Mission artifact schemas (features queue, state, validation state, handoffs).

use serde::{Deserialize, Serialize};

/// Feature lifecycle status in `features.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    /// Ready to schedule when preconditions hold.
    #[default]
    Pending,
    /// Currently assigned to a worker.
    Running,
    /// Worker completed successfully with a handoff.
    Completed,
    /// Worker failed; siblings are not killed.
    Failed,
}

/// One feature in the mission queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    /// Stable feature id.
    pub id: String,
    /// Human description.
    #[serde(default)]
    pub description: String,
    /// Worker skill name (informational for noop/fixture executors).
    #[serde(default, rename = "skillName")]
    pub skill_name: String,
    /// Milestone bucket.
    #[serde(default)]
    pub milestone: String,
    /// Soft preconditions (feature ids that should complete first).
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// Explicit hard depends_on edges (feature ids).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Expected behavior bullets.
    #[serde(default, rename = "expectedBehavior")]
    pub expected_behavior: Vec<String>,
    /// Validation assertion IDs this feature fulfills.
    #[serde(default)]
    pub fulfills: Vec<String>,
    /// Runtime status.
    #[serde(default)]
    pub status: FeatureStatus,
    /// When true, the fixture/noop executor fails this feature (isolation tests).
    #[serde(default)]
    pub fail: bool,
    /// Artificial work duration in milliseconds for fixture executors.
    #[serde(default)]
    pub work_ms: u64,
}

/// `features.json` root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeaturesFile {
    /// Ordered feature queue.
    pub features: Vec<Feature>,
}

/// Runtime mission status file (`state.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionState {
    /// Mission id.
    pub id: String,
    /// High-level status: `created` | `running` | `completed` | `failed` | `partial`.
    pub status: String,
    /// Configured parallel max for this run.
    #[serde(default = "default_parallel_max")]
    pub parallel_max: usize,
    /// Currently live worker assignments.
    #[serde(default)]
    pub live_workers: Vec<LiveWorker>,
    /// Whether a milestone validation gate has been invoked.
    #[serde(default)]
    pub validation_invoked: bool,
    /// Last validation outcome summary.
    #[serde(default)]
    pub last_validation: Option<String>,
}

fn default_parallel_max() -> usize {
    3
}

/// One live worker assignment recorded in `state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWorker {
    /// Worker slot label (`#21`).
    pub worker_id: String,
    /// Feature id.
    pub feature_id: String,
}

/// Per-assertion validation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationAssertionState {
    /// Assertion id (`VAL-…`).
    pub id: String,
    /// `pending` | `passed` | `failed` | `blocked`.
    pub status: String,
}

/// `validation-state.json` root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ValidationStateFile {
    /// Assertions keyed by milestone completion.
    #[serde(default)]
    pub assertions: Vec<ValidationAssertionState>,
}

/// Worker completion success state (EndFeatureRun spirit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessState {
    /// Feature fully done.
    Success,
    /// Partial completion; feature may reset to pending in full orchestrator (fixture treats as failed).
    Partial,
    /// Hard failure; isolated from siblings.
    Failure,
}
