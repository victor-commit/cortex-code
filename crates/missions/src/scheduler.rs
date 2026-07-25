//! Parallel feature scheduler (default 3 / hard max 4), dep-aware, failure-isolating.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use events::{Event, EventBus};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;

use crate::handoff::{write_handoff, Handoff, HandoffTests, HandoffVerification, VerificationCommand};
use crate::schema::{Feature, FeatureStatus, FeaturesFile, MissionState, SuccessState};
use crate::store::{
    append_progress, load_features, load_mission_state, save_features, save_mission_state,
    MissionPaths, StoreError,
};
use crate::validation::{
    invoke_milestone_validation, AlwaysPassHook, ValidationHook, ValidationOutcome,
};

/// Default live implementer pool size.
pub const DEFAULT_MAX_PARALLEL: usize = 3;
/// Hard ceiling on parallel implementers.
pub const HARD_MAX_PARALLEL: usize = 4;

/// Scheduler configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Max concurrent implementer workers (clamped to 1..=HARD_MAX_PARALLEL).
    pub max_parallel: usize,
    /// Base worker id number (fixtures use 21 → `#21`).
    pub worker_id_base: u32,
    /// When true, run milestone validation after all features in a milestone finish.
    pub run_validation: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_parallel: DEFAULT_MAX_PARALLEL,
            worker_id_base: 21,
            run_validation: true,
        }
    }
}

impl SchedulerConfig {
    /// Clamp parallel into the legal range.
    #[must_use]
    pub fn clamped_parallel(&self) -> usize {
        self.max_parallel.clamp(1, HARD_MAX_PARALLEL)
    }
}

/// One progress_log.jsonl line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressEntry {
    /// Unix epoch milliseconds.
    pub ts_ms: u64,
    /// Worker slot (`#21`).
    pub worker_id: String,
    /// Feature id.
    pub feature_id: String,
    /// Event message (`started` | `completed` | `failed` | …).
    pub message: String,
    /// Optional live-count snapshot after this event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_count: Option<usize>,
}

/// Outcome from a feature executor.
#[derive(Debug, Clone)]
pub struct ExecutorOutcome {
    /// Success / failure / partial.
    pub success: SuccessState,
    /// Short summary for the handoff.
    pub summary: String,
    /// Implemented description.
    pub implemented: String,
}

/// Pluggable feature executor (tests use sleep/fail; product can spawn agent workers).
pub trait FeatureExecutor: Send + Sync + 'static {
    /// Execute one feature; must not panic (map panics to Failure if needed).
    fn execute(
        &self,
        feature: &Feature,
        worker_id: &str,
    ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + '_>>;
}

/// Default fixture executor: sleeps `work_ms`, fails when `feature.fail`.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultNoopExecutor;

impl FeatureExecutor for DefaultNoopExecutor {
    fn execute(
        &self,
        feature: &Feature,
        worker_id: &str,
    ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + '_>> {
        let fail = feature.fail;
        let work_ms = feature.work_ms.max(1);
        let id = feature.id.clone();
        let worker = worker_id.to_owned();
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(work_ms)).await;
            if fail {
                ExecutorOutcome {
                    success: SuccessState::Failure,
                    summary: format!("feature {id} failed on purpose ({worker})"),
                    implemented: String::new(),
                }
            } else {
                ExecutorOutcome {
                    success: SuccessState::Success,
                    summary: format!("feature {id} completed ({worker})"),
                    implemented: format!("noop implementation of {id}"),
                }
            }
        })
    }
}

/// Final report after a mission run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    /// Mission id.
    pub mission_id: String,
    /// Completed feature ids.
    pub completed: Vec<String>,
    /// Failed feature ids.
    pub failed: Vec<String>,
    /// Still pending (blocked by deps or unfinished).
    pub pending: Vec<String>,
    /// Peak observed concurrent live workers.
    pub peak_live: usize,
    /// Whether validation was invoked.
    pub validation_invoked: bool,
    /// Last validation outcome label.
    pub last_validation: Option<String>,
    /// Whether any overlapping concurrency ≥2 was observed.
    pub saw_parallel: bool,
}

/// Run the scheduler asynchronously.
pub async fn run_mission(
    paths: &MissionPaths,
    config: SchedulerConfig,
    executor: Arc<dyn FeatureExecutor>,
    bus: Option<Arc<EventBus>>,
    validation_hook: Option<Arc<dyn ValidationHook>>,
) -> Result<RunReport, StoreError> {
    let max_parallel = config.clamped_parallel();
    let mut features_file = load_features(paths)?;
    let mut state = load_mission_state(paths).unwrap_or(MissionState {
        id: paths.id.clone(),
        status: "created".into(),
        parallel_max: max_parallel,
        live_workers: Vec::new(),
        validation_invoked: false,
        last_validation: None,
    });
    state.status = "running".into();
    state.parallel_max = max_parallel;
    state.live_workers.clear();
    save_mission_state(paths, &state)?;

    if let Some(bus) = &bus {
        bus.publish(&Event::mission_started(
            paths.id.clone(),
            paths.root.display().to_string(),
            max_parallel,
        ));
    }

    let semaphore = Arc::new(Semaphore::new(max_parallel));
    let peak_live = Arc::new(Mutex::new(0usize));
    let live_count = Arc::new(Mutex::new(0usize));
    // Shared mutable features snapshot protected by mutex for concurrent status updates.
    let features_mu = Arc::new(Mutex::new(features_file.clone()));
    let paths_arc = Arc::new(paths.clone());
    let bus_arc = bus.clone();

    let mut join_set: JoinSet<WorkerFinish> = JoinSet::new();
    let mut next_worker_ordinal = 0u32;
    let mut inflight: HashSet<String> = HashSet::new();
    let mut completed: HashSet<String> = HashSet::new();
    let mut failed: HashSet<String> = HashSet::new();

    // Seed completed/failed from any pre-set statuses.
    for f in &features_file.features {
        match f.status {
            FeatureStatus::Completed => {
                completed.insert(f.id.clone());
            }
            FeatureStatus::Failed => {
                failed.insert(f.id.clone());
            }
            _ => {}
        }
    }

    loop {
        // Drain finished workers without blocking if none ready.
        while let Some(joined) = poll_join_set(&mut join_set).await {
            apply_finish(
                joined,
                &paths_arc,
                &features_mu,
                &mut completed,
                &mut failed,
                &mut inflight,
                &live_count,
                &bus_arc,
            )
            .await?;
        }

        // Schedule newly ready features up to pool capacity.
        let snapshot = features_mu.lock().await.clone();
        let ready = ready_features(&snapshot, &completed, &failed, &inflight);
        let mut scheduled_any = false;
        for feature in ready {
            let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                break;
            };
            let worker_id = format!("#{}", config.worker_id_base + next_worker_ordinal);
            next_worker_ordinal += 1;
            inflight.insert(feature.id.clone());
            scheduled_any = true;

            // Mark running on disk.
            {
                let mut ff = features_mu.lock().await;
                if let Some(f) = ff.features.iter_mut().find(|f| f.id == feature.id) {
                    f.status = FeatureStatus::Running;
                }
                save_features(&paths_arc, &ff)?;
            }
            {
                let mut live = live_count.lock().await;
                *live += 1;
                let mut peak = peak_live.lock().await;
                if *live > *peak {
                    *peak = *live;
                }
                let lc = *live;
                drop(peak);
                drop(live);

                let entry = ProgressEntry {
                    ts_ms: now_ms(),
                    worker_id: worker_id.clone(),
                    feature_id: feature.id.clone(),
                    message: "started".into(),
                    live_count: Some(lc),
                };
                append_progress(&paths_arc, &entry)?;

                // Update state.live_workers
                let mut st = load_mission_state(&paths_arc)?;
                st.live_workers.retain(|w| w.feature_id != feature.id);
                st.live_workers.push(crate::schema::LiveWorker {
                    worker_id: worker_id.clone(),
                    feature_id: feature.id.clone(),
                });
                save_mission_state(&paths_arc, &st)?;

                if let Some(bus) = &bus_arc {
                    bus.publish(&Event::mission_worker_started(
                        paths_arc.id.clone(),
                        worker_id.clone(),
                        feature.id.clone(),
                        lc,
                        max_parallel,
                    ));
                    bus.publish(&Event::mission_progress(
                        paths_arc.id.clone(),
                        worker_id.clone(),
                        feature.id.clone(),
                        "started",
                        lc,
                    ));
                }
            }

            let exec = Arc::clone(&executor);
            let feat = feature.clone();
            let wid = worker_id.clone();
            join_set.spawn(async move {
                let _permit = permit;
                let outcome = exec.execute(&feat, &wid).await;
                WorkerFinish {
                    feature_id: feat.id,
                    worker_id: wid,
                    outcome,
                }
            });
        }

        if inflight.is_empty() && !scheduled_any {
            // Either all done, or remaining are blocked forever.
            break;
        }

        // Wait for at least one worker to finish before next scheduling pass.
        if let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(finish) => {
                    apply_finish(
                        finish,
                        &paths_arc,
                        &features_mu,
                        &mut completed,
                        &mut failed,
                        &mut inflight,
                        &live_count,
                        &bus_arc,
                    )
                    .await?;
                }
                Err(e) => {
                    // Join error: treat as isolated failure of unknown worker.
                    let entry = ProgressEntry {
                        ts_ms: now_ms(),
                        worker_id: "#?".into(),
                        feature_id: "?".into(),
                        message: format!("join_error:{e}"),
                        live_count: None,
                    };
                    let _ = append_progress(&paths_arc, &entry);
                }
            }
        }
    }

    // Final features snapshot.
    features_file = features_mu.lock().await.clone();
    save_features(paths, &features_file)?;

    let pending: Vec<String> = features_file
        .features
        .iter()
        .filter(|f| {
            !completed.contains(&f.id)
                && !failed.contains(&f.id)
                && f.status != FeatureStatus::Completed
                && f.status != FeatureStatus::Failed
        })
        .map(|f| f.id.clone())
        .collect();

    let peak = *peak_live.lock().await;
    let saw_parallel = peak >= 2;

    // Milestone validation (serial): group by milestone among features that ran.
    let mut validation_invoked = false;
    let mut last_validation = None;
    if config.run_validation {
        let hook: Arc<dyn ValidationHook> = validation_hook.unwrap_or_else(|| Arc::new(AlwaysPassHook));
        let milestones = milestones_ready_for_validation(&features_file, &completed, &failed);
        for ms in milestones {
            let result = invoke_milestone_validation(paths, &ms, hook.as_ref())?;
            validation_invoked = true;
            last_validation = Some(format!(
                "{}:{}",
                ms,
                match result.outcome {
                    ValidationOutcome::Passed => "passed",
                    ValidationOutcome::Failed => "failed",
                    ValidationOutcome::Skipped => "skipped",
                }
            ));
            if let Some(bus) = &bus {
                let outcome = match result.outcome {
                    ValidationOutcome::Passed => "passed",
                    ValidationOutcome::Failed => "failed",
                    ValidationOutcome::Skipped => "skipped",
                };
                bus.publish(&Event::mission_validation(
                    paths.id.clone(),
                    ms.clone(),
                    outcome,
                ));
            }
            // Failure blocks seal — stop further milestone seals.
            if result.outcome == ValidationOutcome::Failed {
                break;
            }
        }
    }

    let mut st = load_mission_state(paths)?;
    st.live_workers.clear();
    st.status = if !failed.is_empty() && completed.is_empty() {
        "failed".into()
    } else if !failed.is_empty() {
        "partial".into()
    } else if pending.is_empty() {
        if st.last_validation.as_deref().is_some_and(|v| v.contains(":failed")) {
            "validation_failed".into()
        } else {
            "completed".into()
        }
    } else {
        "partial".into()
    };
    if validation_invoked {
        st.validation_invoked = true;
        if last_validation.is_some() {
            st.last_validation = last_validation.clone();
        }
    }
    save_mission_state(paths, &st)?;

    if let Some(bus) = &bus {
        bus.publish(&Event::mission_finished(
            paths.id.clone(),
            st.status.clone(),
            completed.len(),
            failed.len(),
        ));
    }

    let mut completed_v: Vec<_> = completed.into_iter().collect();
    completed_v.sort();
    let mut failed_v: Vec<_> = failed.into_iter().collect();
    failed_v.sort();

    Ok(RunReport {
        mission_id: paths.id.clone(),
        completed: completed_v,
        failed: failed_v,
        pending,
        peak_live: peak,
        validation_invoked,
        last_validation,
        saw_parallel,
    })
}

/// Blocking wrapper around [`run_mission`] (creates a current-thread/multi-thread runtime as needed).
pub fn run_mission_blocking(
    paths: &MissionPaths,
    config: SchedulerConfig,
    executor: Arc<dyn FeatureExecutor>,
    bus: Option<Arc<EventBus>>,
    validation_hook: Option<Arc<dyn ValidationHook>>,
) -> Result<RunReport, StoreError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(max_threads(config.clamped_parallel()))
        .build()
        .map_err(|e| StoreError::Invalid(format!("tokio runtime: {e}")))?;
    rt.block_on(run_mission(paths, config, executor, bus, validation_hook))
}

fn max_threads(parallel: usize) -> usize {
    (parallel + 2).clamp(2, 8)
}

struct WorkerFinish {
    feature_id: String,
    worker_id: String,
    outcome: ExecutorOutcome,
}

async fn poll_join_set(join_set: &mut JoinSet<WorkerFinish>) -> Option<WorkerFinish> {
    // Non-blocking poll: use try_join_next if available (tokio 1.40+), else return None.
    // For broader compatibility, only used when we already await join_next in the main loop.
    // This helper is a no-op placeholder for the drain-at-top pattern using try_join_next.
    #[allow(unused_mut)]
    {
        // tokio JoinSet::try_join_next exists on recent tokio.
        join_set.try_join_next().and_then(|r| r.ok())
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_finish(
    finish: WorkerFinish,
    paths: &MissionPaths,
    features_mu: &Mutex<FeaturesFile>,
    completed: &mut HashSet<String>,
    failed: &mut HashSet<String>,
    inflight: &mut HashSet<String>,
    live_count: &Mutex<usize>,
    bus: &Option<Arc<EventBus>>,
) -> Result<(), StoreError> {
    inflight.remove(&finish.feature_id);
    let ok = matches!(finish.outcome.success, SuccessState::Success);
    let status = if ok {
        FeatureStatus::Completed
    } else {
        FeatureStatus::Failed
    };
    if ok {
        completed.insert(finish.feature_id.clone());
    } else {
        failed.insert(finish.feature_id.clone());
    }

    {
        let mut ff = features_mu.lock().await;
        if let Some(f) = ff.features.iter_mut().find(|f| f.id == finish.feature_id) {
            f.status = status;
        }
        save_features(paths, &ff)?;
    }

    let handoff = Handoff {
        feature_id: finish.feature_id.clone(),
        worker_id: finish.worker_id.clone(),
        success_state: finish.outcome.success,
        salient_summary: finish.outcome.summary.clone(),
        what_was_implemented: finish.outcome.implemented.clone(),
        what_was_left_undone: if ok {
            String::new()
        } else {
            "feature failed".into()
        },
        verification: HandoffVerification {
            commands_run: vec![VerificationCommand {
                command: "fixture-executor".into(),
                exit_code: if ok { 0 } else { 1 },
                observation: finish.outcome.summary.clone(),
            }],
        },
        tests: HandoffTests {
            coverage: "fixture".into(),
        },
        return_to_orchestrator: false,
    };
    write_handoff(paths, &handoff)?;

    let lc = {
        let mut live = live_count.lock().await;
        *live = live.saturating_sub(1);
        *live
    };

    let msg = if ok { "completed" } else { "failed" };
    let entry = ProgressEntry {
        ts_ms: now_ms(),
        worker_id: finish.worker_id.clone(),
        feature_id: finish.feature_id.clone(),
        message: msg.into(),
        live_count: Some(lc),
    };
    append_progress(paths, &entry)?;

    let mut st = load_mission_state(paths)?;
    st.live_workers
        .retain(|w| w.feature_id != finish.feature_id);
    save_mission_state(paths, &st)?;

    if let Some(bus) = bus {
        if ok {
            bus.publish(&Event::mission_worker_completed(
                paths.id.clone(),
                finish.worker_id.clone(),
                finish.feature_id.clone(),
                lc,
            ));
        } else {
            bus.publish(&Event::mission_worker_failed(
                paths.id.clone(),
                finish.worker_id.clone(),
                finish.feature_id.clone(),
                finish.outcome.summary.clone(),
                lc,
            ));
        }
        bus.publish(&Event::mission_progress(
            paths.id.clone(),
            finish.worker_id,
            finish.feature_id,
            msg,
            lc,
        ));
    }
    Ok(())
}

/// Features that are pending, not inflight, and whose depends_on are all completed
/// (failed deps block the dependent — it stays pending).
fn ready_features(
    file: &FeaturesFile,
    completed: &HashSet<String>,
    failed: &HashSet<String>,
    inflight: &HashSet<String>,
) -> Vec<Feature> {
    let status_map: HashMap<&str, FeatureStatus> = file
        .features
        .iter()
        .map(|f| (f.id.as_str(), f.status))
        .collect();

    file.features
        .iter()
        .filter(|f| {
            if inflight.contains(&f.id) || completed.contains(&f.id) || failed.contains(&f.id) {
                return false;
            }
            if matches!(
                f.status,
                FeatureStatus::Completed | FeatureStatus::Failed | FeatureStatus::Running
            ) {
                // Running should be in inflight; if not, don't re-schedule.
                if f.status == FeatureStatus::Running {
                    return false;
                }
                if f.status == FeatureStatus::Completed || f.status == FeatureStatus::Failed {
                    return false;
                }
            }
            // depends_on + preconditions both act as hard edges for readiness.
            let deps = f.depends_on.iter().chain(f.preconditions.iter());
            for dep in deps {
                if failed.contains(dep) {
                    return false;
                }
                if !completed.contains(dep) {
                    // Also honor disk status.
                    match status_map.get(dep.as_str()) {
                        Some(FeatureStatus::Completed) => {}
                        _ => return false,
                    }
                }
            }
            true
        })
        .cloned()
        .collect()
}

/// Milestones whose features are all terminal (completed or failed) and at least one completed.
fn milestones_ready_for_validation(
    file: &FeaturesFile,
    completed: &HashSet<String>,
    failed: &HashSet<String>,
) -> Vec<String> {
    let mut by_ms: HashMap<String, Vec<&Feature>> = HashMap::new();
    for f in &file.features {
        let ms = if f.milestone.is_empty() {
            "default".to_owned()
        } else {
            f.milestone.clone()
        };
        by_ms.entry(ms).or_default().push(f);
    }
    let mut ready = Vec::new();
    for (ms, feats) in by_ms {
        let all_terminal = feats.iter().all(|f| {
            completed.contains(&f.id)
                || failed.contains(&f.id)
                || matches!(
                    f.status,
                    FeatureStatus::Completed | FeatureStatus::Failed
                )
        });
        let any_done = feats.iter().any(|f| completed.contains(&f.id));
        if all_terminal && any_done {
            ready.push(ms);
        }
    }
    ready.sort();
    ready
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        create_fixture_mission, create_isolation_fixture_mission, read_progress_log,
        CreateMissionOpts,
    };
    use crate::validation::AlwaysFailHook;
    use crate::snapshot_mission;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    /// Executor that tracks peak concurrency via a shared counter.
    struct PeakTrackingExecutor {
        current: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl FeatureExecutor for PeakTrackingExecutor {
        fn execute(
            &self,
            feature: &Feature,
            worker_id: &str,
        ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + '_>> {
            let fail = feature.fail;
            let work_ms = feature.work_ms.max(50);
            let id = feature.id.clone();
            let worker = worker_id.to_owned();
            let current = Arc::clone(&self.current);
            let peak = Arc::clone(&self.peak);
            Box::pin(async move {
                let now = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(work_ms)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                if fail {
                    ExecutorOutcome {
                        success: SuccessState::Failure,
                        summary: format!("{id} fail"),
                        implemented: String::new(),
                    }
                } else {
                    ExecutorOutcome {
                        success: SuccessState::Success,
                        summary: format!("{id} ok ({worker})"),
                        implemented: id,
                    }
                }
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_pool_runs_two_or_more() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("par-1".into()),
                title: "parallel".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        // Stretch work so overlaps are visible.
        let mut ff = load_features(&paths).unwrap();
        for f in &mut ff.features {
            f.work_ms = 120;
            f.fail = false;
        }
        save_features(&paths, &ff).unwrap();

        let current = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let exec = Arc::new(PeakTrackingExecutor {
            current: Arc::clone(&current),
            peak: Arc::clone(&peak),
        });

        let bus = Arc::new(EventBus::new());
        let report = run_mission(
            &paths,
            SchedulerConfig {
                max_parallel: 3,
                worker_id_base: 21,
                run_validation: true,
            },
            exec,
            Some(bus),
            None,
        )
        .await
        .unwrap();

        assert!(
            report.peak_live >= 2,
            "expected ≥2 concurrent workers, peak_live={}",
            report.peak_live
        );
        assert!(report.saw_parallel);
        assert!(peak.load(Ordering::SeqCst) >= 2);
        assert_eq!(report.completed.len(), 3);
        assert!(report.failed.is_empty());
        assert!(report.validation_invoked);

        // progress_log timestamps should show overlapping started entries before completions.
        let log = read_progress_log(&paths).unwrap();
        let starts: Vec<_> = log.iter().filter(|e| e.message == "started").collect();
        assert!(starts.len() >= 3);
        // At least one start should record live_count >= 2.
        let max_live = log.iter().filter_map(|e| e.live_count).max().unwrap_or(0);
        assert!(
            max_live >= 2,
            "progress_log should observe live_count≥2, got {max_live}; log={log:?}"
        );

        // Handoffs written for all three.
        for id in ["f-a", "f-b", "f-c"] {
            assert!(
                paths.handoffs_dir().join(format!("{id}.json")).is_file(),
                "missing handoff for {id}"
            );
        }

        let snap = snapshot_mission(&paths).unwrap();
        assert!(snap.validation_invoked);
        assert_eq!(snap.features_done, 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn worker_failure_isolates_siblings() {
        let tmp = tempdir().unwrap();
        let paths = create_isolation_fixture_mission(tmp.path(), "iso-1").unwrap();
        let mut ff = load_features(&paths).unwrap();
        for f in &mut ff.features {
            f.work_ms = 80;
        }
        save_features(&paths, &ff).unwrap();

        let exec = Arc::new(DefaultNoopExecutor);
        let report = run_mission(
            &paths,
            SchedulerConfig {
                max_parallel: 3,
                worker_id_base: 21,
                run_validation: true,
            },
            exec,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(
            report.failed.contains(&"f-b".to_string()),
            "f-b should fail: {report:?}"
        );
        assert!(
            report.completed.contains(&"f-a".to_string()),
            "f-a should complete despite f-b failure"
        );
        assert!(
            report.completed.contains(&"f-c".to_string()),
            "f-c should complete despite f-b failure"
        );
        // Mixed status on disk.
        let ff = load_features(&paths).unwrap();
        let statuses: HashMap<_, _> = ff.features.iter().map(|f| (f.id.as_str(), f.status)).collect();
        assert_eq!(statuses["f-a"], FeatureStatus::Completed);
        assert_eq!(statuses["f-b"], FeatureStatus::Failed);
        assert_eq!(statuses["f-c"], FeatureStatus::Completed);
        // Handoffs exist for failed too.
        assert!(paths.handoffs_dir().join("f-b.json").is_file());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn validation_gate_failure_blocks_seal() {
        let tmp = tempdir().unwrap();
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("val-block".into()),
                title: "val".into(),
                parallel_max: 3,
                features: vec![],
            },
        )
        .unwrap();
        let mut ff = load_features(&paths).unwrap();
        for f in &mut ff.features {
            f.work_ms = 20;
            f.fail = false;
        }
        save_features(&paths, &ff).unwrap();

        let report = run_mission(
            &paths,
            SchedulerConfig {
                max_parallel: 3,
                worker_id_base: 21,
                run_validation: true,
            },
            Arc::new(DefaultNoopExecutor),
            None,
            Some(Arc::new(AlwaysFailHook)),
        )
        .await
        .unwrap();

        assert!(report.validation_invoked);
        assert!(
            report
                .last_validation
                .as_deref()
                .is_some_and(|s| s.contains("failed")),
            "expected failed validation, got {:?}",
            report.last_validation
        );
        let st = load_mission_state(&paths).unwrap();
        assert_eq!(st.status, "validation_failed");
        assert!(paths
            .root
            .join("validation")
            .join("m-fixture")
            .join("scrutiny.json")
            .is_file());
    }

    #[tokio::test]
    async fn respects_depends_on_ordering() {
        let tmp = tempdir().unwrap();
        let features = vec![
            Feature {
                id: "first".into(),
                description: "first".into(),
                skill_name: "fx".into(),
                milestone: "m".into(),
                preconditions: vec![],
                depends_on: vec![],
                expected_behavior: vec![],
                fulfills: vec![],
                status: FeatureStatus::Pending,
                fail: false,
                work_ms: 30,
            },
            Feature {
                id: "second".into(),
                description: "second".into(),
                skill_name: "fx".into(),
                milestone: "m".into(),
                preconditions: vec![],
                depends_on: vec!["first".into()],
                expected_behavior: vec![],
                fulfills: vec![],
                status: FeatureStatus::Pending,
                fail: false,
                work_ms: 30,
            },
        ];
        let paths = create_fixture_mission(
            tmp.path(),
            CreateMissionOpts {
                id: Some("deps".into()),
                title: "deps".into(),
                parallel_max: 3,
                features,
            },
        )
        .unwrap();

        // Track order via progress log.
        let report = run_mission(
            &paths,
            SchedulerConfig::default(),
            Arc::new(DefaultNoopExecutor),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.completed.len(), 2);
        let log = read_progress_log(&paths).unwrap();
        let first_start = log
            .iter()
            .find(|e| e.feature_id == "first" && e.message == "started")
            .unwrap()
            .ts_ms;
        let second_start = log
            .iter()
            .find(|e| e.feature_id == "second" && e.message == "started")
            .unwrap()
            .ts_ms;
        let first_done = log
            .iter()
            .find(|e| e.feature_id == "first" && e.message == "completed")
            .unwrap()
            .ts_ms;
        assert!(
            second_start >= first_done || second_start >= first_start,
            "second should not start before first is at least underway; first_start={first_start} first_done={first_done} second_start={second_start}"
        );
        // Stronger: second start after first complete.
        assert!(
            second_start >= first_done,
            "depends_on violated: second_start={second_start} first_done={first_done}"
        );
    }

    #[test]
    fn hard_max_clamps_to_four() {
        let cfg = SchedulerConfig {
            max_parallel: 99,
            ..SchedulerConfig::default()
        };
        assert_eq!(cfg.clamped_parallel(), HARD_MAX_PARALLEL);
    }
}
