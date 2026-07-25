//! Clean-room parallel missions engine for Cortex.
//!
//! Factory-**similar** semantics (missionDir, features queue, handoffs, milestone validators)
//! reverse-engineered at the behavioral level — **not** a copy of proprietary droid code.
//! Product differentiator: **parallel implementer pool** (default 3 / max 4) with failure
//! isolation and live scheduler events for Mission Control.
//!
//! Mission roots never write under Factory `~/.factory/missions/*`. Default root is
//! `~/.cortex/missions` (override with `CORTEX_MISSIONS_ROOT`).

mod handoff;
mod paths;
mod scheduler;
mod schema;
mod store;
mod validation;

pub use handoff::{
    ensure_handoffs_dir, load_handoff, write_handoff, Handoff, HandoffTests, HandoffVerification,
    VerificationCommand,
};
pub use paths::{missions_root, MissionsRootInputs, ENV_MISSIONS_ROOT};
pub use scheduler::{
    run_mission, run_mission_blocking, DefaultNoopExecutor, ExecutorOutcome, FeatureExecutor,
    ProgressEntry, RunReport, SchedulerConfig, DEFAULT_MAX_PARALLEL, HARD_MAX_PARALLEL,
};
pub use schema::{
    Feature, FeatureStatus, FeaturesFile, LiveWorker, MissionState, SuccessState,
    ValidationAssertionState, ValidationStateFile,
};
pub use store::{
    append_progress, create_fixture_mission, create_isolation_fixture_mission, create_mission,
    list_missions, load_features, load_mission_state, load_validation_state, open_mission,
    open_mission_dir, read_progress_log, save_features, save_mission_state, save_validation_state,
    CreateMissionOpts, MissionPaths, MissionSummary, StoreError,
};
pub use validation::{
    invoke_milestone_validation, record_validation_result, seal_blocked, write_gate_marker,
    AlwaysFailHook, AlwaysPassHook, CommandListHook, ValidationGateResult, ValidationHook,
    ValidationOutcome,
};

/// Snapshot of a running/finished mission suitable for MC UI or CLI status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MissionSnapshot {
    /// Mission id.
    pub id: String,
    /// Absolute mission directory.
    pub mission_dir: String,
    /// Parallel live workers / max slots.
    pub parallel_live: usize,
    /// Configured max parallel slots (clamped to [`HARD_MAX_PARALLEL`]).
    pub parallel_max: usize,
    /// Features completed.
    pub features_done: usize,
    /// Total features.
    pub features_total: usize,
    /// Features currently running.
    pub running: Vec<WorkerSlotView>,
    /// Features that failed (isolated).
    pub failed: Vec<String>,
    /// Features still pending/queued.
    pub pending: Vec<String>,
    /// Features completed.
    pub completed: Vec<String>,
    /// Overall run status string.
    pub status: String,
    /// Last N progress log lines (newest last).
    pub progress_tail: Vec<String>,
    /// Whether a milestone validation gate has been invoked at least once.
    pub validation_invoked: bool,
    /// Last validation gate outcome when known.
    pub last_validation: Option<String>,
}

/// One live (or recently failed) worker slot for MC.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkerSlotView {
    /// Worker slot id (`#21` style).
    pub worker_id: String,
    /// Feature id.
    pub feature_id: String,
    /// `running` | `failed` | `completed`.
    pub state: String,
}

/// Build a [`MissionSnapshot`] from on-disk mission state + progress log.
pub fn snapshot_mission(paths: &MissionPaths) -> Result<MissionSnapshot, store::StoreError> {
    let features = load_features(paths)?;
    let state = load_mission_state(paths).unwrap_or_else(|_| MissionState {
        id: paths.id.clone(),
        status: "unknown".into(),
        parallel_max: DEFAULT_MAX_PARALLEL,
        live_workers: Vec::new(),
        validation_invoked: false,
        last_validation: None,
    });
    let progress = read_progress_log(paths).unwrap_or_default();
    let progress_tail: Vec<String> = progress
        .iter()
        .rev()
        .take(12)
        .map(|e| {
            format!(
                "{} worker={} feature={} {}",
                e.ts_ms, e.worker_id, e.feature_id, e.message
            )
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut running = Vec::new();
    let mut failed = Vec::new();
    let mut pending = Vec::new();
    let mut completed = Vec::new();
    for f in &features.features {
        match f.status {
            FeatureStatus::Running => {
                let worker_id = state
                    .live_workers
                    .iter()
                    .find(|w| w.feature_id == f.id)
                    .map(|w| w.worker_id.clone())
                    .unwrap_or_else(|| "#?".into());
                running.push(WorkerSlotView {
                    worker_id,
                    feature_id: f.id.clone(),
                    state: "running".into(),
                });
            }
            FeatureStatus::Failed => failed.push(f.id.clone()),
            FeatureStatus::Pending => pending.push(f.id.clone()),
            FeatureStatus::Completed => completed.push(f.id.clone()),
        }
    }

    Ok(MissionSnapshot {
        id: paths.id.clone(),
        mission_dir: paths.root.display().to_string(),
        parallel_live: running.len(),
        parallel_max: state.parallel_max.clamp(1, HARD_MAX_PARALLEL),
        features_done: completed.len(),
        features_total: features.features.len(),
        running,
        failed,
        pending,
        completed,
        status: state.status,
        progress_tail,
        validation_invoked: state.validation_invoked,
        last_validation: state.last_validation,
    })
}
