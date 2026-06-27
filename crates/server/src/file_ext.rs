//! File/filesystem inspection routes layered over the workspace directory.
//!
//! These handlers let a server client safely read project state: list a directory, read a file's
//! text or binary/media metadata, and report the git working-tree status. Every path is resolved
//! against the server's workspace `cwd` and gated so a request can never escape the workspace into
//! an external directory (those are refused with `403` before any read). Missing files inside the
//! workspace are a clean `404`; none of these routes mutate the filesystem.

use std::path::{Path, PathBuf};
use std::process::Command;

use axum::{Json, extract::State, http::StatusCode};
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::{ApiError, ServerState};

/// Bytes sampled from the head of a file to sniff its media type / binary-ness.
const SNIFF_BYTES: usize = 4096;
/// The invalid-UTF-8 byte ratio above which a non-image/PDF file is classified binary.
const BINARY_INVALID_UTF8_RATIO: f64 = 0.30;
/// Maximum number of bytes returned base64-encoded for a binary/media file.
const MAX_BINARY_BYTES: u64 = 5 * 1024 * 1024;
/// Directory entries conventionally ignored by tooling, surfaced with `ignored: true`.
const COMMON_IGNORED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".cortex",
    ".DS_Store",
];

/// Query for the `path`-scoped file routes (`/file`, `/file/content`).
#[derive(Debug, Deserialize)]
pub(super) struct FileQuery {
    #[serde(default)]
    path: String,
}

/// One directory entry, matching the reference `FileNode` wire shape.
#[derive(Debug, Serialize)]
pub(super) struct FileNode {
    name: String,
    path: String,
    absolute: String,
    #[serde(rename = "type")]
    entry_type: &'static str,
    ignored: bool,
}

/// A file's content, matching the reference `FileContent` wire shape.
#[derive(Debug, Serialize)]
pub(super) struct FileContent {
    #[serde(rename = "type")]
    content_type: &'static str,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<&'static str>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

/// One changed file, matching the reference `File` git-status wire shape.
#[derive(Debug, Serialize)]
pub(super) struct FileStatus {
    path: String,
    added: u64,
    removed: u64,
    status: &'static str,
}

/// `GET /file?path=<dir>` — list files and directories under a workspace-relative path.
pub(super) async fn list_files(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<FileQuery>,
) -> Result<Json<Vec<FileNode>>, ApiError> {
    let requested = if query.path.trim().is_empty() {
        "."
    } else {
        query.path.trim()
    };
    let resolved = resolve_in_workspace(&state.cwd, requested)?;
    let metadata = std::fs::metadata(&resolved).map_err(|err| read_error(requested, &err))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request(format!(
            "`{requested}` is not a directory"
        )));
    }

    let mut nodes = Vec::new();
    let entries = std::fs::read_dir(&resolved).map_err(|err| read_error(requested, &err))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let relative = display_relative(&state.cwd, &path);
        nodes.push(FileNode {
            name: name.to_owned(),
            path: redaction::redact(&relative),
            absolute: redaction::redact(&path.to_string_lossy()),
            entry_type: if is_dir { "directory" } else { "file" },
            ignored: is_ignored_name(name),
        });
    }
    nodes.sort_by(|left, right| {
        right
            .entry_type
            .cmp(left.entry_type)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(Json(nodes))
}

/// `GET /file/content?path=<file>` — read a workspace file's text or binary/media metadata.
pub(super) async fn read_file(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<FileQuery>,
) -> Result<Json<FileContent>, ApiError> {
    let requested = query.path.trim();
    if requested.is_empty() {
        return Err(ApiError::bad_request("`path` query parameter is required"));
    }
    let resolved = resolve_in_workspace(&state.cwd, requested)?;
    let metadata = std::fs::metadata(&resolved).map_err(|err| read_error(requested, &err))?;
    if metadata.is_dir() {
        return Err(ApiError::bad_request(format!(
            "`{requested}` is a directory, not a file"
        )));
    }

    let bytes = std::fs::read(&resolved).map_err(|err| read_error(requested, &err))?;
    let prefix = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if let Some(mime) = media_mime_type(&resolved, prefix) {
        return Ok(Json(binary_content(&bytes, metadata.len(), Some(mime))));
    }
    if sniffs_binary(prefix) {
        return Ok(Json(binary_content(
            &bytes,
            metadata.len(),
            Some("application/octet-stream".to_owned()),
        )));
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Json(FileContent {
        content_type: "text",
        content,
        encoding: None,
        mime_type: None,
    }))
}

/// `GET /file/status` — report the git working-tree status of the workspace.
///
/// When the workspace is not a git repository (or git is unavailable) this returns an empty list
/// rather than an error, so the route is always safe to poll.
pub(super) async fn file_status(
    State(state): State<ServerState>,
) -> Result<Json<Vec<FileStatus>>, ApiError> {
    Ok(Json(git_status(&state.cwd)))
}

/// Resolve a workspace-relative (or absolute) request path and refuse anything that escapes the
/// workspace directory, including via `..` traversal or symlinks.
fn resolve_in_workspace(cwd: &Path, requested: &str) -> Result<PathBuf, ApiError> {
    let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let candidate = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        canonical_cwd.join(requested)
    };
    // Canonicalize to resolve symlink escapes for paths that exist, then lexically normalize the
    // result so a `..` traversal to a not-yet-existing path is still caught (canonicalize cannot
    // resolve a path whose ancestors do not exist).
    let resolved = lexically_normalize(&canonicalize_allowing_missing(&candidate));
    if !resolved.starts_with(&canonical_cwd) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("`{requested}` is outside the workspace directory and cannot be read"),
        ));
    }
    Ok(resolved)
}

/// Lexically resolve `.` and `..` components without touching the filesystem.
fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Canonicalize `path`, resolving symlinks; for a not-yet-existing leaf, canonicalize the parent so
/// traversal/symlink escapes are still caught before the existence check.
fn canonicalize_allowing_missing(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

fn display_relative(cwd: &Path, path: &Path) -> String {
    let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match canonical_path.strip_prefix(&canonical_cwd) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => relative.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

fn is_ignored_name(name: &str) -> bool {
    name.starts_with('.') || COMMON_IGNORED.contains(&name)
}

fn read_error(requested: &str, err: &std::io::Error) -> ApiError {
    match err.kind() {
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
            format!("cannot read `{requested}`: {err}"),
        ),
    }
}

fn binary_content(bytes: &[u8], len: u64, mime: Option<String>) -> FileContent {
    if len > MAX_BINARY_BYTES {
        // Oversize binary: surface metadata only so the channel never carries an unbounded blob.
        return FileContent {
            content_type: "binary",
            content: String::new(),
            encoding: None,
            mime_type: mime,
        };
    }
    let content = base64::engine::general_purpose::STANDARD.encode(bytes);
    FileContent {
        content_type: "binary",
        content,
        encoding: Some("base64"),
        mime_type: mime,
    }
}

/// The image/PDF media type for `path`, by extension first then magic bytes, or `None`.
fn media_mime_type(path: &Path, prefix: &[u8]) -> Option<String> {
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        match extension.to_ascii_lowercase().as_str() {
            "png" => return Some("image/png".to_owned()),
            "jpg" | "jpeg" => return Some("image/jpeg".to_owned()),
            "gif" => return Some("image/gif".to_owned()),
            "webp" => return Some("image/webp".to_owned()),
            "bmp" => return Some("image/bmp".to_owned()),
            "svg" => return Some("image/svg+xml".to_owned()),
            "pdf" => return Some("application/pdf".to_owned()),
            _ => {}
        }
    }
    if prefix.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png".to_owned());
    }
    if prefix.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg".to_owned());
    }
    if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        return Some("image/gif".to_owned());
    }
    if prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"WEBP" {
        return Some("image/webp".to_owned());
    }
    if prefix.starts_with(b"%PDF-") {
        return Some("application/pdf".to_owned());
    }
    None
}

/// Classify a non-image/PDF file as binary: a NUL byte or a high invalid-UTF-8 ratio in the sampled
/// head, so binary bytes are never emitted as mojibake text.
fn sniffs_binary(sample: &[u8]) -> bool {
    if sample.is_empty() {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }
    let mut invalid = 0usize;
    let mut rest = sample;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(_) => break,
            Err(error) => {
                let valid = error.valid_up_to();
                match error.error_len() {
                    Some(len) => {
                        invalid += len;
                        rest = &rest[valid + len..];
                    }
                    None => break,
                }
            }
        }
    }
    (invalid as f64 / sample.len() as f64) > BINARY_INVALID_UTF8_RATIO
}

/// Compute the git working-tree status of `cwd`, returning an empty list when it is not a git
/// repository or git is unavailable.
fn git_status(cwd: &Path) -> Vec<FileStatus> {
    let Some(output) = run_git(cwd, &["status", "--porcelain"]) else {
        return Vec::new();
    };
    let numstat = numstat_map(cwd);
    let mut statuses = Vec::new();
    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim().to_owned();
        // Rename entries render as `old -> new`; report the new path.
        let path = path
            .rsplit_once(" -> ")
            .map(|(_, new)| new.to_owned())
            .unwrap_or(path);
        let status = classify_status(code);
        let (added, removed) = numstat.get(&path).copied().unwrap_or((0, 0));
        statuses.push(FileStatus {
            path: redaction::redact(&path),
            added,
            removed,
            status,
        });
    }
    statuses.sort_by(|left, right| left.path.cmp(&right.path));
    statuses
}

fn classify_status(code: &str) -> &'static str {
    if code.contains('D') {
        "deleted"
    } else if code.contains('A') || code.contains('?') {
        "added"
    } else {
        "modified"
    }
}

fn numstat_map(cwd: &Path) -> std::collections::HashMap<String, (u64, u64)> {
    let mut map = std::collections::HashMap::new();
    let Some(output) = run_git(cwd, &["diff", "--numstat", "HEAD"]) else {
        return map;
    };
    for line in output.lines() {
        let mut fields = line.split('\t');
        let added = fields.next().and_then(|value| value.parse::<u64>().ok());
        let removed = fields.next().and_then(|value| value.parse::<u64>().ok());
        let path = fields.next().map(str::to_owned);
        if let (Some(added), Some(removed), Some(path)) = (added, removed, path) {
            map.insert(path, (added, removed));
        }
    }
    map
}

fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}
