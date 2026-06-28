//! Project, workspace, worktree, and path routes backed by real filesystem and git state.
//!
//! These read-only handlers describe where the server is operating: the active project and its
//! stable id, the workspace root and VCS, the git worktrees attached to the repository, and
//! metadata for an individual workspace path. Every route reflects on-disk reality (git toplevel,
//! `git worktree list`, filesystem `stat`) and refuses paths that escape the workspace with a clean
//! `4xx` rather than leaking host state.

use std::path::{Path, PathBuf};

use axum::{Json, extract::State, http::StatusCode};
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
        id: checkpoints::workspace_key(cwd),
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
