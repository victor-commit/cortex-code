//! LSP, formatter, and VCS inspection/operation routes.
//!
//! These handlers expose the same language-server and formatter status that the `cortex lsp status`
//! and `cortex formatter status` CLI commands report, plus read-only git diff inspection and an
//! explicit, opt-in `git apply` operation. Status routes never spawn a language server or formatter
//! and never touch the network. `/vcs/diff` is strictly read-only; only `/vcs/apply` mutates the
//! working tree, and only when a caller posts a patch to it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use super::{ApiError, ServerState, config_api_error};
use crate::file_ext::run_git_shared;

/// One configured language server's status row, matching `cortex lsp status --format json`.
#[derive(Clone, Debug, Serialize)]
pub(super) struct LspStatusRow {
    language: String,
    program: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    install: Option<String>,
}

/// Short TTL for the cached `GET /lsp` status.
///
/// The real probe is heavy and variable-latency: for every configured language server it scans the
/// process table for a running instance and checks installability, which spawns subprocesses and
/// can take several seconds in aggregate. Caching the computed rows for this window gives a fast
/// path for tight sequential status polls without changing semantics — language-server status
/// changes slowly relative to this window, so a freshly cached row is still correct.
const LSP_STATUS_CACHE_TTL: Duration = Duration::from_secs(5);

/// Process-shared cache for the `GET /lsp` status rows, cloned cheaply with [`ServerState`].
///
/// The cache implements stale-while-revalidate: a fresh entry is returned immediately, an expired
/// entry is still returned immediately while a single background task refreshes it, and only the
/// very first request (with no entry at all) pays the full probe latency. Concurrent first-ever
/// callers are single-flighted through `coldstart`, so a burst of cold probes collapses into one
/// compute whose result the rest await. This bounds `GET /lsp` to a prompt response under tight
/// sequential polling and keeps the heavy probe off the request path after the first call.
#[derive(Clone, Default)]
pub(crate) struct LspStatusCache {
    inner: Arc<Mutex<Option<LspStatusCacheEntry>>>,
    refreshing: Arc<AtomicBool>,
    coldstart: Arc<tokio::sync::Mutex<()>>,
}

struct LspStatusCacheEntry {
    computed_at: Instant,
    rows: Vec<LspStatusRow>,
}

/// RAII release for the single background-refresh slot. Dropping the guard clears the `refreshing`
/// flag, so a panic or early return in the spawned refresh task can never permanently latch the
/// slot and block all future revalidation.
struct RefreshGuard {
    refreshing: Arc<AtomicBool>,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.refreshing.store(false, Ordering::Release);
    }
}

impl LspStatusCache {
    /// Return the cached rows when they are still within the TTL, otherwise `None`.
    fn get_fresh(&self) -> Option<Vec<LspStatusRow>> {
        let guard = self.inner.lock().ok()?;
        let entry = guard.as_ref()?;
        (entry.computed_at.elapsed() < LSP_STATUS_CACHE_TTL).then(|| entry.rows.clone())
    }

    /// Return the cached rows regardless of age, for the stale-while-revalidate fast path.
    fn get_any(&self) -> Option<Vec<LspStatusRow>> {
        let guard = self.inner.lock().ok()?;
        guard.as_ref().map(|entry| entry.rows.clone())
    }

    /// Store freshly computed rows, stamping them with the current instant.
    fn store(&self, rows: Vec<LspStatusRow>) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some(LspStatusCacheEntry {
                computed_at: Instant::now(),
                rows,
            });
        }
    }

    /// Claim the single background-refresh slot, returning an RAII [`RefreshGuard`] for exactly one
    /// caller until the guard is dropped, or `None` when a refresh is already in flight. Because the
    /// guard releases the slot on drop (including on panic or early return), the slot can always
    /// recover and never stays latched. Prevents a stampede of concurrent probes.
    fn begin_refresh(&self) -> Option<RefreshGuard> {
        self.refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            .then(|| RefreshGuard {
                refreshing: Arc::clone(&self.refreshing),
            })
    }

    /// Cold-start single-flight: when the cache holds no entry at all, serialize concurrent
    /// first-ever callers so only the first runs the heavy `compute`; the rest await it and return
    /// the rows it cached. Returns the rows plus whether THIS caller performed the fresh compute, so
    /// the caller can emit `lsp.updated` only for a genuine fresh compute and never for a stampeded
    /// awaiter.
    async fn cold_start_single_flight<F, Fut>(
        &self,
        compute: F,
    ) -> Result<(Vec<LspStatusRow>, bool), ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<LspStatusRow>, ApiError>>,
    {
        let _gate = self.coldstart.lock().await;
        // A sibling cold-start caller may have populated the cache while we waited on the gate.
        if let Some(rows) = self.get_any() {
            return Ok((rows, false));
        }
        let rows = compute().await?;
        self.store(rows.clone());
        Ok((rows, true))
    }
}

/// `GET /lsp` — report the configured language servers and whether each is disabled, available,
/// installing, running, unavailable, or crashed. Mirrors `cortex lsp status` and never launches a
/// server.
///
/// The underlying probe is heavy (~seconds), so the result is cached with a short TTL and computed
/// off the async runtime via `spawn_blocking`. A fresh cache hit returns immediately; an expired
/// entry is served immediately while a single background task refreshes it; only the very first
/// request pays the full probe latency, and concurrent first-ever callers are single-flighted so a
/// burst triggers exactly one compute. This bounds the route to a prompt response under tight
/// sequential polling while preserving the reported status semantics.
pub(super) async fn lsp_status(
    State(state): State<ServerState>,
) -> Result<Json<Vec<LspStatusRow>>, ApiError> {
    if let Some(rows) = state.lsp_status_cache.get_fresh() {
        return Ok(Json(rows));
    }
    // Stale-but-present: serve the previous rows immediately and refresh behind the request so the
    // heavy probe never blocks the request path more than once.
    if let Some(stale) = state.lsp_status_cache.get_any() {
        spawn_lsp_refresh(state.clone());
        return Ok(Json(stale));
    }

    // Cold start: single-flight so a burst of concurrent first-ever callers triggers exactly one
    // probe (off the async runtime); the rest await its result instead of each paying the full
    // ~seconds compute. `lsp.updated` is published only for the genuine fresh compute.
    let (rows, fresh) = state
        .lsp_status_cache
        .cold_start_single_flight(|| compute_lsp_rows_off_runtime(&state))
        .await?;
    if fresh {
        publish_running_servers(&state, &rows);
    }
    Ok(Json(rows))
}

/// Spawn a single background task to recompute and cache the LSP status, if one is not already
/// running. Used by the stale-while-revalidate fast path.
fn spawn_lsp_refresh(state: ServerState) {
    let Some(guard) = state.lsp_status_cache.begin_refresh() else {
        return;
    };
    tokio::spawn(async move {
        // Hold the slot for the whole refresh; dropping the guard on completion, early return, or
        // panic releases it so future revalidation can always recover.
        let _guard = guard;
        if let Ok(rows) = compute_lsp_rows_off_runtime(&state).await {
            publish_running_servers(&state, &rows);
            state.lsp_status_cache.store(rows);
        }
    });
}

/// Run the heavy LSP status probe off the async runtime so it never stalls request handling.
async fn compute_lsp_rows_off_runtime(state: &ServerState) -> Result<Vec<LspStatusRow>, ApiError> {
    let cwd = state.cwd.clone();
    let inputs = state.config.inputs(&cwd, BTreeMap::new());
    tokio::task::spawn_blocking(move || compute_lsp_rows(inputs, &cwd))
        .await
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("lsp status probe did not complete: {err}"),
            )
        })?
}

/// Publish an `lsp.updated` event for every running server row.
fn publish_running_servers(state: &ServerState, rows: &[LspStatusRow]) {
    for row in rows {
        if row.state == "running" {
            state
                .event_bus
                .publish(&events::Event::LspUpdated(events::LspUpdated {
                    server: Some(row.language.clone()),
                    path: None,
                    diagnostic_count: None,
                }));
        }
    }
}

/// Compute the configured language-server status rows. This is the heavy, blocking probe; callers
/// run it off the async runtime and cache the result.
fn compute_lsp_rows(
    inputs: settings::ConfigInputs,
    cwd: &Path,
) -> Result<Vec<LspStatusRow>, ApiError> {
    let effective = settings::load(inputs).map_err(config_api_error)?;
    let config = crate::lsp_config::from_effective(&effective).map_err(config_api_error)?;
    let mut manager = lsp::FeedbackManager::new(cwd.to_path_buf(), config);
    let rows: Vec<LspStatusRow> = manager
        .statuses()
        .into_iter()
        .map(|status| {
            let (label, pid) = match status.state {
                lsp::ServerRunState::Running(pid) => ("running".to_owned(), Some(pid)),
                other => (other.label().to_owned(), None),
            };
            LspStatusRow {
                language: redaction::redact(&status.language_id),
                program: redaction::redact(&status.program),
                state: label,
                pid,
                install: status.install_hint.map(|hint| redaction::redact(&hint)),
            }
        })
        .collect();
    Ok(rows)
}

/// One formatter status row, matching `cortex formatter status --format json`.
#[derive(Debug, Serialize)]
pub(super) struct FormatterStatusRow {
    name: String,
    formatter: String,
    state: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    #[serde(default)]
    env_keys: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    marker_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    marker_found: Option<bool>,
}

/// `GET /formatter` — report every known formatter, its state (enabled/disabled/missing), source
/// (default vs config), and matching extensions. Mirrors `cortex formatter status` and never runs a
/// formatter.
pub(super) async fn formatter_status(
    State(state): State<ServerState>,
) -> Result<Json<Vec<FormatterStatusRow>>, ApiError> {
    let inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective = settings::load(inputs).map_err(config_api_error)?;
    let config = format::FormatterConfig::from_table(effective.table())
        .map_err(|message| ApiError::bad_request(format!("invalid formatter config: {message}")))?;
    let rows = format::FormatterRunner::with_config(state.cwd.clone(), config)
        .status()
        .into_iter()
        .map(|row| FormatterStatusRow {
            name: redaction::redact(&row.name),
            formatter: row.formatter.id().to_owned(),
            state: row.state.label().to_owned(),
            source: row.source.label().to_owned(),
            command: row
                .command
                .map(|command| command.iter().map(|arg| redaction::redact(arg)).collect()),
            env_keys: row.env_keys,
            extensions: row.extensions,
            marker_required: row.marker_required,
            marker_found: row.marker_found,
        })
        .collect();
    Ok(Json(rows))
}

/// Query for `GET /vcs/diff`.
#[derive(Debug, Deserialize)]
pub(super) struct VcsDiffQuery {
    #[serde(default)]
    path: Option<String>,
    /// When set, diff the working tree against `HEAD` (staged + unstaged) instead of only unstaged.
    #[serde(default)]
    staged: bool,
}

/// One changed file in a VCS diff, matching the file-status wire shape.
#[derive(Debug, Serialize)]
pub(super) struct VcsFile {
    path: String,
    added: u64,
    removed: u64,
    status: String,
}

/// Response for `GET /vcs/diff`.
#[derive(Debug, Serialize)]
pub(super) struct VcsDiffResponse {
    repository: bool,
    diff: String,
    files: Vec<VcsFile>,
}

/// `GET /vcs/diff` — report the working-tree diff (read-only).
///
/// Returns `repository: false` with an empty diff when the workspace is not a git repository, so the
/// route is always safe to poll. This route never mutates the working tree.
pub(super) async fn vcs_diff(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<VcsDiffQuery>,
) -> Result<Json<VcsDiffResponse>, ApiError> {
    if run_git_shared(&state.cwd, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Ok(Json(VcsDiffResponse {
            repository: false,
            diff: String::new(),
            files: Vec::new(),
        }));
    }

    let path = query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty());
    let base: Vec<&str> = if query.staged {
        vec!["diff", "HEAD"]
    } else {
        vec!["diff"]
    };

    let mut diff_args = base.clone();
    if let Some(path) = path {
        diff_args.push("--");
        diff_args.push(path);
    }
    let diff = run_git_shared(&state.cwd, &diff_args).unwrap_or_default();

    let mut numstat_args = base;
    numstat_args.insert(1, "--numstat");
    if let Some(path) = path {
        numstat_args.push("--");
        numstat_args.push(path);
    }
    let files = parse_numstat(run_git_shared(&state.cwd, &numstat_args).unwrap_or_default());

    if let Some(branch) = run_git_shared(&state.cwd, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        let branch = branch.trim().to_owned();
        if !branch.is_empty() {
            state
                .event_bus
                .publish(&events::Event::VcsBranchUpdated(events::VcsBranchUpdated {
                    branch: redaction::redact(&branch),
                    dirty: Some(!files.is_empty()),
                }));
        }
    }

    Ok(Json(VcsDiffResponse {
        repository: true,
        diff: redaction::redact(&diff),
        files,
    }))
}

fn parse_numstat(output: String) -> Vec<VcsFile> {
    let mut files = Vec::new();
    for line in output.lines() {
        let mut fields = line.split('\t');
        let added = fields.next();
        let removed = fields.next();
        let path = fields.next();
        let (Some(added), Some(removed), Some(path)) = (added, removed, path) else {
            continue;
        };
        // Binary files render their numstat counts as `-`.
        let binary = added == "-" || removed == "-";
        files.push(VcsFile {
            path: redaction::redact(path.trim()),
            added: added.parse().unwrap_or(0),
            removed: removed.parse().unwrap_or(0),
            status: if binary { "binary" } else { "modified" }.to_owned(),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

/// Request body for `POST /vcs/apply`.
#[derive(Debug, Deserialize)]
pub(super) struct VcsApplyRequest {
    diff: String,
    /// Validate the patch with `git apply --check` without writing anything to disk.
    #[serde(default)]
    check: bool,
    /// Apply the patch in reverse (`git apply --reverse`).
    #[serde(default)]
    reverse: bool,
}

/// Response for `POST /vcs/apply`.
#[derive(Debug, Serialize)]
pub(super) struct VcsApplyResponse {
    status: &'static str,
    applied: bool,
    checked: bool,
}

/// `POST /vcs/apply` — apply a unified diff to the working tree via `git apply`.
///
/// This is the only VCS route that mutates disk, and it only does so when a caller posts a patch.
/// With `check: true` the patch is validated (`--check`) without modifying any file. A patch that
/// does not apply cleanly returns `422` with git's own error so the caller can correct it; the
/// working tree is never left partially patched because `git apply` is atomic per invocation.
pub(super) async fn vcs_apply(
    State(state): State<ServerState>,
    Json(payload): Json<VcsApplyRequest>,
) -> Result<Json<VcsApplyResponse>, ApiError> {
    if payload.diff.trim().is_empty() {
        return Err(ApiError::bad_request(
            "apply body must include a non-empty diff",
        ));
    }
    if run_git_shared(&state.cwd, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err(ApiError::bad_request(
            "workspace is not a git repository; cannot apply a patch",
        ));
    }

    let mut args = vec!["apply", "--whitespace=nowarn"];
    if payload.check {
        args.push("--check");
    }
    if payload.reverse {
        args.push("--reverse");
    }
    git_apply(&state.cwd, &args, &payload.diff)?;
    Ok(Json(VcsApplyResponse {
        status: if payload.check { "valid" } else { "applied" },
        applied: !payload.check,
        checked: payload.check,
    }))
}

fn git_apply(cwd: &Path, args: &[&str], diff: &str) -> Result<(), ApiError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not run git apply: {err}"),
            )
        })?;

    let mut stdin = child.stdin.take().ok_or_else(|| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not write patch to git apply",
        )
    })?;
    let mut payload = diff.to_owned();
    if !payload.ends_with('\n') {
        payload.push('\n');
    }
    stdin.write_all(payload.as_bytes()).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not stream patch to git apply: {err}"),
        )
    })?;
    drop(stdin);

    let output = child.wait_with_output().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git apply did not complete: {err}"),
        )
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    Err(ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        if detail.is_empty() {
            "patch did not apply cleanly".to_owned()
        } else {
            format!("patch did not apply cleanly: {}", redaction::redact(detail))
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn sample_rows() -> Vec<LspStatusRow> {
        vec![LspStatusRow {
            language: "rust".to_owned(),
            program: "rust-analyzer".to_owned(),
            state: "available".to_owned(),
            pid: None,
            install: None,
        }]
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cold_start_single_flight_computes_once_for_concurrent_callers() {
        let cache = LspStatusCache::default();
        let computes = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let cache = cache.clone();
            let computes = Arc::clone(&computes);
            handles.push(tokio::spawn(async move {
                cache
                    .cold_start_single_flight(|| async {
                        computes.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Ok(sample_rows())
                    })
                    .await
                    .map(|(_, fresh)| fresh)
                    .expect("cold start single-flight")
            }));
        }
        let mut fresh_count = 0;
        for handle in handles {
            if handle.await.expect("join cold-start task") {
                fresh_count += 1;
            }
        }
        assert_eq!(
            computes.load(Ordering::SeqCst),
            1,
            "concurrent cold-start callers must trigger exactly one underlying compute"
        );
        assert_eq!(
            fresh_count, 1,
            "exactly one caller performs the fresh compute; the rest await its result"
        );
    }

    #[tokio::test]
    async fn cold_start_single_flight_returns_cached_entry_without_computing() {
        let cache = LspStatusCache::default();
        cache.store(sample_rows());
        let computed = AtomicUsize::new(0);
        let (rows, fresh) = cache
            .cold_start_single_flight(|| async {
                computed.fetch_add(1, Ordering::SeqCst);
                Ok(sample_rows())
            })
            .await
            .expect("cold start single-flight");
        assert!(!fresh, "a populated cache must not report a fresh compute");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            computed.load(Ordering::SeqCst),
            0,
            "an already-populated cache must not run the compute closure"
        );
    }

    #[tokio::test]
    async fn begin_refresh_is_exclusive_and_releases_on_drop() {
        let cache = LspStatusCache::default();
        let guard = cache.begin_refresh().expect("first refresh slot");
        assert!(
            cache.begin_refresh().is_none(),
            "a second concurrent refresh must be denied the slot"
        );
        drop(guard);
        assert!(
            cache.begin_refresh().is_some(),
            "dropping the guard must release the refresh slot"
        );
    }

    #[tokio::test]
    async fn refresh_slot_recovers_when_holding_task_panics() {
        let cache = LspStatusCache::default();
        let guard = cache.begin_refresh().expect("refresh slot");
        let handle = tokio::spawn(async move {
            let _guard = guard;
            panic!("refresh task panicked before releasing the slot");
        });
        assert!(
            handle.await.is_err(),
            "the spawned refresh task should have panicked"
        );
        assert!(
            cache.begin_refresh().is_some(),
            "the refresh slot must be released even when the holding task panics"
        );
    }
}
