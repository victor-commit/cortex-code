//! The background-job registry: tracks long-running `bash` jobs so they are inspectable while
//! running and reaped (terminated and awaited) on shutdown — no orphan process survives.
//!
//! A backgrounded `bash` command returns control to the turn immediately with a job handle while
//! the process keeps running, its output streamed to an on-disk log file. The spawned child is
//! registered here so its PID is tracked. [`BackgroundJobs::shutdown`] kills and reaps every tracked
//! child by PID; the children are also spawned with `kill_on_drop`, so dropping the registry is a
//! fail-safe that still leaves no orphan.

use std::path::PathBuf;
use std::sync::Mutex;

use tokio::process::Child;

/// A spawned background job and the child handle used to reap it.
pub struct BackgroundJob {
    /// The job identifier surfaced in the tool result.
    pub id: String,
    /// The operating-system process id of the spawned child.
    pub pid: u32,
    /// The shell command the job runs.
    pub command: String,
    /// The on-disk file the job's combined stdout/stderr is streamed to.
    pub log_path: PathBuf,
    /// The child handle, owned so the job can be killed and reaped.
    child: Child,
}

impl BackgroundJob {
    /// Build a tracked job from its handle fields and the owned child process.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        pid: u32,
        command: impl Into<String>,
        log_path: PathBuf,
        child: Child,
    ) -> Self {
        Self {
            id: id.into(),
            pid,
            command: command.into(),
            log_path,
            child,
        }
    }
}

/// An inspectable, child-free snapshot of a tracked background job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobHandle {
    /// The job identifier.
    pub id: String,
    /// The operating-system process id of the spawned child.
    pub pid: u32,
    /// The shell command the job runs.
    pub command: String,
    /// The on-disk file the job's output is streamed to.
    pub log_path: PathBuf,
}

/// A registry of background jobs the `bash` tool spawned this session.
///
/// Shared (via `Arc`) between the per-session [`ToolContext`](crate::ToolContext) the `bash` tool
/// runs against and the surface (the CLI) that reaps the jobs when the run ends.
#[derive(Default)]
pub struct BackgroundJobs {
    jobs: Mutex<Vec<BackgroundJob>>,
}

impl BackgroundJobs {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Track `job` so it can be inspected and reaped on shutdown.
    pub fn register(&self, job: BackgroundJob) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.push(job);
        }
    }

    /// An inspectable snapshot of the currently tracked jobs (without their child handles).
    #[must_use]
    pub fn handles(&self) -> Vec<JobHandle> {
        self.jobs
            .lock()
            .map(|jobs| {
                jobs.iter()
                    .map(|job| JobHandle {
                        id: job.id.clone(),
                        pid: job.pid,
                        command: job.command.clone(),
                        log_path: job.log_path.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The number of tracked jobs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.jobs.lock().map(|jobs| jobs.len()).unwrap_or(0)
    }

    /// Whether no jobs are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Terminate and reap every tracked job by PID, leaving no orphan process.
    ///
    /// Each job's whole process group is killed (so a `bash` wrapper and the commands it spawned
    /// both die, not just the direct child), then the direct child is awaited so it is fully reaped
    /// (not left a zombie). The registry is emptied. The lock is released before awaiting so the
    /// synchronous `Mutex` is never held across an `.await`.
    pub async fn shutdown(&self) {
        let drained: Vec<BackgroundJob> = match self.jobs.lock() {
            Ok(mut jobs) => std::mem::take(&mut *jobs),
            Err(_) => return,
        };
        for mut job in drained {
            super::kill_process_group(job.pid).await;
            let _ = job.child.start_kill();
            let _ = job.child.wait().await;
        }
    }
}

/// The kernel-reported start time of process `pid` (field 22 of `/proc/<pid>/stat`, in clock ticks
/// since boot). Two processes sharing a pid but with different start times are necessarily distinct
/// processes, so capturing this at spawn time lets a later liveness probe reject a *reused* pid.
/// `None` when the process is gone or `/proc` is unreadable (Linux-only, matching the target sandbox
/// and the background-job feature).
#[must_use]
pub fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `comm` (field 2) is wrapped in parens and may itself contain spaces or ')'; splitting after
    // the LAST ')' leaves fields 3.. , so `starttime` (field 22) is index 19 of the remainder.
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse::<u64>().ok()
}

/// Whether `pid` still names the *same* process it did when `start_time` was captured, guarding
/// against pid reuse: if the pid exists but its start time no longer matches the baseline, the
/// original job has exited and an unrelated process now holds the pid, so it reads as not alive.
/// When no baseline was captured (`None`), falls back to bare `/proc/<pid>` existence.
#[must_use]
pub fn pid_alive_same_process(pid: u32, start_time: Option<u64>) -> bool {
    match (process_start_time(pid), start_time) {
        (Some(current), Some(expected)) => current == expected,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::process::Command;

    async fn spawn_sleeper() -> Child {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn sleep")
    }

    fn pid_alive(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    #[tokio::test]
    async fn registers_and_inspects_a_job() {
        let registry = BackgroundJobs::new();
        let child = spawn_sleeper().await;
        let pid = child.id().expect("pid");
        registry.register(BackgroundJob::new(
            "bash-1",
            pid,
            "sleep 60",
            PathBuf::from("/tmp/x.log"),
            child,
        ));

        assert_eq!(registry.len(), 1);
        let handles = registry.handles();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].pid, pid);
        assert_eq!(handles[0].command, "sleep 60");

        registry.shutdown().await;
        assert!(registry.is_empty(), "shutdown empties the registry");
    }

    #[tokio::test]
    async fn shutdown_terminates_tracked_jobs_with_no_orphan() {
        let registry = BackgroundJobs::new();
        let child = spawn_sleeper().await;
        let pid = child.id().expect("pid");
        assert!(pid_alive(pid), "the job is running before shutdown");

        registry.register(BackgroundJob::new(
            "bash-1",
            pid,
            "sleep 60",
            PathBuf::from("/tmp/x.log"),
            child,
        ));
        registry.shutdown().await;

        // The reaped child must be gone (no orphan).
        for _ in 0..50 {
            if !pid_alive(pid) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!pid_alive(pid), "pid {pid} must be reaped after shutdown");
    }

    #[tokio::test]
    async fn liveness_guard_rejects_a_reused_pid() {
        // A live process reads alive against its own captured start time, and (crucially) reads NOT
        // alive against a mismatched start time — the shape that guards a completion transition
        // against pid REUSE, where a freed pid is re-taken by an unrelated process.
        let child = spawn_sleeper().await;
        let pid = child.id().expect("pid");

        let start = process_start_time(pid).expect("a live process has a start time");
        assert!(
            pid_alive_same_process(pid, Some(start)),
            "the same process is alive against its own start time"
        );
        assert!(
            !pid_alive_same_process(pid, Some(start.wrapping_add(1))),
            "a mismatched start time (pid reuse) reads as not alive"
        );
        assert!(
            pid_alive_same_process(pid, None),
            "with no baseline, liveness falls back to bare existence"
        );

        // A never-allocated-high pid has no start time and reads not alive.
        assert_eq!(process_start_time(u32::MAX), None);
        assert!(!pid_alive_same_process(u32::MAX, Some(start)));

        drop(child);
    }
}
