//! Project, workspace, worktree, and path routes backed by real filesystem and git state.
//!
//! These read-only handlers describe where the server is operating: the active project and its
//! stable id, the workspace root and VCS, the git worktrees attached to the repository, and
//! metadata for an individual workspace path. Every route reflects on-disk reality (git toplevel,
//! `git worktree list`, filesystem `stat`) and refuses paths that escape the workspace with a clean
//! `4xx` rather than leaking host state.

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, ServerState};
use crate::file_ext::{resolve_in_workspace, run_git_shared};

/// A project: a workspace directory and the stable per-workspace key Cortex derives for it.
///
/// `directories` reflects the durable multi-directory project model: when the workspace directory is
/// registered to a project that spans several directories, every directory is listed (so one
/// logical project is visible as a unit); otherwise it is just the active directory.
#[derive(Debug, Serialize)]
pub(super) struct Project {
    id: String,
    directory: String,
    directories: Vec<String>,
    worktree: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vcs: Option<&'static str>,
}

fn current_project(cwd: &Path, db_path: &Path) -> Project {
    let worktree = git_toplevel(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let vcs = git_toplevel(cwd).is_some().then_some("git");
    Project {
        id: format!(
            "{}_{}",
            db::prefix::PROJECT,
            checkpoints::workspace_key(cwd)
        ),
        directory: redaction::redact(&cwd.to_string_lossy()),
        directories: project_directories(cwd, db_path),
        worktree: redaction::redact(&worktree.to_string_lossy()),
        vcs,
    }
}

/// The directories of the project that owns `cwd`, from the durable project model, redacted at the
/// boundary. Falls back to just the active directory when no multi-directory project is registered.
fn project_directories(cwd: &Path, db_path: &Path) -> Vec<String> {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if let Ok(db) = db::Database::open(db_path)
        && let Ok(Some(project)) = db
            .projects()
            .find_by_directory(&canonical.to_string_lossy())
    {
        let directories: Vec<String> = project
            .directories
            .iter()
            .map(|directory| redaction::redact(directory))
            .collect();
        if !directories.is_empty() {
            return directories;
        }
    }
    vec![redaction::redact(&cwd.to_string_lossy())]
}

/// `GET /project` — list the projects known to this server (the active workspace project).
pub(super) async fn list_projects(
    State(state): State<ServerState>,
) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(vec![current_project(&state.cwd, state.db_path())]))
}

/// `GET /project/current` — the active workspace project.
pub(super) async fn current_project_route(
    State(state): State<ServerState>,
) -> Result<Json<Project>, ApiError> {
    let project = current_project(&state.cwd, state.db_path());
    state
        .event_bus
        .publish(&events::Event::ProjectUpdated(events::ProjectUpdated {
            path: project.directory.clone(),
        }));
    Ok(Json(project))
}

/// Workspace summary: the root directory, VCS, and number of attached worktrees.
#[derive(Debug, Serialize)]
pub(super) struct Workspace {
    id: String,
    root: String,
    directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vcs: Option<&'static str>,
    worktrees: usize,
}

/// `GET /workspace` — describe the current workspace from real git/filesystem state.
pub(super) async fn current_workspace(
    State(state): State<ServerState>,
) -> Result<Json<Workspace>, ApiError> {
    let root = git_toplevel(&state.cwd).unwrap_or_else(|| state.cwd.clone());
    let vcs = git_toplevel(&state.cwd).is_some().then_some("git");
    let worktrees = worktree_entries(&state.cwd).len().max(1);
    Ok(Json(Workspace {
        id: checkpoints::workspace_key(&state.cwd),
        root: redaction::redact(&root.to_string_lossy()),
        directory: redaction::redact(&state.cwd.to_string_lossy()),
        vcs,
        worktrees,
    }))
}

/// One git worktree, matching the porcelain `git worktree list` fields.
#[derive(Debug, Serialize)]
pub(super) struct Worktree {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    bare: bool,
    detached: bool,
    locked: bool,
}

/// `GET /worktree` — list the git worktrees attached to the workspace repository.
///
/// Returns an empty list when the workspace is not a git repository, so the route is safe to poll.
pub(super) async fn list_worktrees(
    State(state): State<ServerState>,
) -> Result<Json<Vec<Worktree>>, ApiError> {
    Ok(Json(worktree_entries(&state.cwd)))
}

fn worktree_entries(cwd: &Path) -> Vec<Worktree> {
    let Some(output) = run_git_shared(cwd, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                worktrees.push(entry);
            }
            current = Some(Worktree {
                path: redaction::redact(path.trim()),
                head: None,
                branch: None,
                bare: false,
                detached: false,
                locked: false,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = Some(head.trim().to_owned());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                entry.branch = Some(redaction::redact(branch.trim()));
            } else if line == "bare" {
                entry.bare = true;
            } else if line == "detached" {
                entry.detached = true;
            } else if line.starts_with("locked") {
                entry.locked = true;
            }
        }
    }
    if let Some(entry) = current.take() {
        worktrees.push(entry);
    }
    worktrees
}

/// Request body for `POST /worktree`.
#[derive(Debug, Deserialize)]
pub(super) struct CreateWorktreeRequest {
    /// Workspace-relative (or absolute, within the workspace) path for the new worktree checkout.
    path: String,
    /// Check out (and optionally create) this branch in the new worktree.
    #[serde(default)]
    branch: Option<String>,
    /// Create `branch` as a new branch rather than checking out an existing one.
    #[serde(default)]
    new_branch: bool,
    /// The commit-ish to base the worktree on (defaults to HEAD).
    #[serde(default, rename = "ref")]
    reference: Option<String>,
    /// Force creation even when the path is non-empty or the branch is already checked out.
    #[serde(default)]
    force: bool,
}

/// `POST /worktree` — create a real git worktree and broadcast a `worktree.ready`/`worktree.failed`
/// event to subscribers (the SSE stream and in-process bus).
///
/// The target path is resolved inside the workspace; a path that escapes it is `403`. On a git
/// failure (non-empty path, branch already checked out, ...) the route emits `worktree.failed` and
/// returns `422` with git's own scrubbed message so the caller can correct it.
pub(super) async fn create_worktree(
    State(state): State<ServerState>,
    Json(payload): Json<CreateWorktreeRequest>,
) -> Result<(StatusCode, Json<Worktree>), ApiError> {
    let requested = payload.path.trim();
    if requested.is_empty() {
        return Err(ApiError::bad_request(
            "worktree path must be a non-empty workspace path",
        ));
    }
    if run_git_shared(&state.cwd, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err(ApiError::bad_request(
            "workspace is not a git repository; cannot create a worktree",
        ));
    }
    let target = resolve_in_workspace(&state.cwd, requested)?;
    let target_display = target.to_string_lossy().into_owned();

    let mut args: Vec<String> = vec!["worktree".to_owned(), "add".to_owned()];
    if payload.force {
        args.push("--force".to_owned());
    }
    let branch = payload
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty());
    let reference = payload
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let commitish = if payload.new_branch {
        if let Some(branch) = branch {
            args.push("-b".to_owned());
            args.push(branch.to_owned());
        }
        reference
    } else {
        // No `-b`/`-B`: an existing branch is the commit-ish checked out after the path, so the
        // worktree attaches to it as-is. `-B` would reset that branch to HEAD, silently moving it.
        branch.or(reference)
    };
    args.push(target_display.clone());
    if let Some(commitish) = commitish {
        args.push(commitish.to_owned());
    }

    match run_git_mut(&state.cwd, &args) {
        Ok(()) => {
            state
                .event_bus
                .publish(&events::Event::worktree_ready(redaction::redact(
                    &target_display,
                )));
            let entry = worktree_entries(&state.cwd)
                .into_iter()
                .find(|entry| entry.path == redaction::redact(&target_display))
                .unwrap_or(Worktree {
                    path: redaction::redact(&target_display),
                    head: None,
                    branch: payload.branch.as_deref().map(redaction::redact),
                    bare: false,
                    detached: false,
                    locked: false,
                });
            Ok((StatusCode::CREATED, Json(entry)))
        }
        Err(message) => {
            state.event_bus.publish(&events::Event::worktree_failed(
                redaction::redact(&target_display),
                &message,
            ));
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message))
        }
    }
}

/// Query for `DELETE /worktree`.
#[derive(Debug, Deserialize)]
pub(super) struct RemoveWorktreeQuery {
    /// Workspace path of the worktree to remove.
    path: String,
    /// Force removal even when the worktree has uncommitted changes.
    #[serde(default)]
    force: bool,
}

/// Response for `DELETE /worktree`.
#[derive(Debug, Serialize)]
pub(super) struct WorktreeRemoved {
    status: &'static str,
    path: String,
}

/// `DELETE /worktree?path=<rel>` — remove a git worktree and broadcast a `worktree.removed`
/// (or `worktree.failed`) event. A git failure returns `422` with git's scrubbed message.
pub(super) async fn remove_worktree(
    State(state): State<ServerState>,
    Query(query): Query<RemoveWorktreeQuery>,
) -> Result<Json<WorktreeRemoved>, ApiError> {
    let requested = query.path.trim();
    if requested.is_empty() {
        return Err(ApiError::bad_request(
            "worktree path must be a non-empty workspace path",
        ));
    }
    if run_git_shared(&state.cwd, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Err(ApiError::bad_request(
            "workspace is not a git repository; cannot remove a worktree",
        ));
    }
    let target = resolve_in_workspace(&state.cwd, requested)?;
    let target_display = target.to_string_lossy().into_owned();

    let mut args: Vec<String> = vec!["worktree".to_owned(), "remove".to_owned()];
    if query.force {
        args.push("--force".to_owned());
    }
    args.push(target_display.clone());

    match run_git_mut(&state.cwd, &args) {
        Ok(()) => {
            state
                .event_bus
                .publish(&events::Event::worktree_removed(redaction::redact(
                    &target_display,
                )));
            Ok(Json(WorktreeRemoved {
                status: "removed",
                path: redaction::redact(&target_display),
            }))
        }
        Err(message) => {
            state.event_bus.publish(&events::Event::worktree_failed(
                redaction::redact(&target_display),
                &message,
            ));
            Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message))
        }
    }
}

/// Run a mutating `git` command in `cwd`, returning the scrubbed stderr (or a generic message) on
/// failure. Used only by the worktree create/remove routes, which are the sole project routes that
/// modify on-disk git state.
fn run_git_mut(cwd: &Path, args: &[String]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("could not run git: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    let message = if detail.is_empty() {
        "git worktree command failed".to_owned()
    } else {
        redaction::redact(detail)
    };
    Err(message)
}

/// Query for `GET /path`.
#[derive(Debug, Deserialize)]
pub(super) struct PathQuery {
    #[serde(default)]
    path: Option<String>,
}

/// Workspace/app paths summary returned by `GET /path` with no query.
#[derive(Debug, Serialize)]
pub(super) struct PathSummary {
    directory: String,
    worktree: String,
    root: String,
    config: String,
}

/// Metadata for a single workspace path returned by `GET /path?path=<rel>`.
#[derive(Debug, Serialize)]
pub(super) struct PathMetadata {
    path: String,
    absolute: String,
    #[serde(rename = "type")]
    entry_type: &'static str,
    exists: bool,
}

/// `GET /path` — workspace paths summary, or metadata for a specific `?path=` inside the workspace.
///
/// A `path` that escapes the workspace is `403`; a path that does not exist is `404`. Without a
/// `path` query the route returns the directory/worktree/root/config summary.
pub(super) async fn path_info(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<PathQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(requested) => {
            let resolved = resolve_in_workspace(&state.cwd, requested)?;
            let metadata = std::fs::metadata(&resolved).map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => ApiError::new(
                    StatusCode::NOT_FOUND,
                    format!("`{requested}` was not found in the workspace"),
                ),
                std::io::ErrorKind::PermissionDenied => ApiError::new(
                    StatusCode::FORBIDDEN,
                    format!("`{requested}` cannot be read: permission denied"),
                ),
                _ => ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("cannot stat `{requested}`: {err}"),
                ),
            })?;
            let entry_type = if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let value = serde_json::to_value(PathMetadata {
                path: redaction::redact(requested),
                absolute: redaction::redact(&resolved.to_string_lossy()),
                entry_type,
                exists: true,
            })
            .map_err(serialize_error)?;
            Ok(Json(value))
        }
        None => {
            let worktree = git_toplevel(&state.cwd).unwrap_or_else(|| state.cwd.clone());
            let config = state.cwd.join("cortex.toml");
            let value = serde_json::to_value(PathSummary {
                directory: redaction::redact(&state.cwd.to_string_lossy()),
                worktree: redaction::redact(&worktree.to_string_lossy()),
                root: redaction::redact(&worktree.to_string_lossy()),
                config: redaction::redact(&config.to_string_lossy()),
            })
            .map_err(serialize_error)?;
            Ok(Json(value))
        }
    }
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = run_git_shared(cwd, &["rev-parse", "--show-toplevel"])?;
    let trimmed = output.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn serialize_error(err: serde_json::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("could not serialize path response: {err}"),
    )
}
