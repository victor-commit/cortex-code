//! The default on-disk location for per-workspace shadow stores.
//!
//! The shadow store lives OUTSIDE the project working tree, under the per-user data directory, so it
//! never pollutes the project or its `.git`. Its directory is keyed by the canonical workspace path,
//! so the same workspace always maps to the same store across processes and surfaces — the headless
//! `cortex run`/`cortex undo` and the interactive TUI address one shared store. Location precedence
//! (highest first):
//!
//! - `CORTEX_CHECKPOINT_DIR` — an explicit base directory for shadow stores.
//! - `CORTEX_DATA_DIR` — `<CORTEX_DATA_DIR>/checkpoints`.
//! - `XDG_DATA_HOME` — `<XDG_DATA_HOME>/cortex/checkpoints`.
//! - `HOME` — `<HOME>/.local/share/cortex/checkpoints`.

use std::path::{Path, PathBuf};

use crate::error::CheckpointError;
use crate::store::ShadowStore;

/// The explicit shadow-store base-directory override.
const ENV_CHECKPOINT_DIR: &str = "CORTEX_CHECKPOINT_DIR";

impl ShadowStore {
    /// Open (creating on first use) the shadow store at this workspace's default location, or `None`
    /// when no data location can be resolved (so a surface degrades to no checkpointing rather than
    /// failing). The location is keyed by the canonical workspace path (see [`workspace_key`]), so a
    /// `cortex run` that records snapshots and a later `cortex undo` — or the TUI's `/undo` — from
    /// the same directory address the same store.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when a base location resolves but the shadow store cannot be
    /// opened (e.g. the workspace root cannot be canonicalized or the git directory cannot be
    /// created).
    pub fn open_default(cwd: &Path) -> Result<Option<ShadowStore>, CheckpointError> {
        let Some(base) = default_base_dir() else {
            return Ok(None);
        };
        let git_dir = base.join(workspace_key(cwd));
        Ok(Some(ShadowStore::open(cwd, &git_dir)?))
    }
}

/// A stable, filesystem-safe key for `cwd`: a sanitized basename plus a deterministic hash of the
/// canonical path, so two different workspaces never collide and the same workspace always maps to
/// the same per-project store across processes. Shared by the other per-project data-dir stores
/// (e.g. the saved-permission store) so every per-workspace artifact keys projects identically.
#[must_use]
pub fn workspace_key(cwd: &Path) -> String {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let basename = canonical
        .file_name()
        .map(|name| sanitize(&name.to_string_lossy()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "root".to_owned());
    format!(
        "{basename}-{:016x}",
        fnv1a64(canonical.to_string_lossy().as_bytes())
    )
}

/// The base directory under which per-workspace shadow stores live, by documented precedence.
fn default_base_dir() -> Option<PathBuf> {
    if let Some(dir) = non_empty_var(ENV_CHECKPOINT_DIR) {
        return Some(dir);
    }
    if let Some(dir) = non_empty_var("CORTEX_DATA_DIR") {
        return Some(dir.join("checkpoints"));
    }
    if let Some(xdg) = non_empty_var("XDG_DATA_HOME") {
        return Some(xdg.join("cortex").join("checkpoints"));
    }
    non_empty_var("HOME").map(|home| {
        home.join(".local")
            .join("share")
            .join("cortex")
            .join("checkpoints")
    })
}

/// Replace any character that is not alphanumeric, `-`, `_`, or `.` with `_`, so the key is a single
/// safe path component.
fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A small, dependency-free, deterministic 64-bit FNV-1a hash (stable across processes and builds).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Read an environment variable, treating an unset or empty value as absent.
fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_unsafe_characters() {
        assert_eq!(sanitize("my proj/v2"), "my_proj_v2");
        assert_eq!(sanitize("ok-name_1.2"), "ok-name_1.2");
    }

    #[test]
    fn fnv_is_deterministic_and_distinguishes_inputs() {
        assert_eq!(fnv1a64(b"/a/b"), fnv1a64(b"/a/b"));
        assert_ne!(fnv1a64(b"/a/b"), fnv1a64(b"/a/c"));
    }

    #[test]
    fn workspace_key_is_stable_for_the_same_path() {
        let dir = std::env::temp_dir();
        assert_eq!(workspace_key(&dir), workspace_key(&dir));
    }

    #[test]
    fn workspace_key_distinguishes_different_paths() {
        let a = std::env::temp_dir().join("cortex-key-a");
        let b = std::env::temp_dir().join("cortex-key-b");
        assert_ne!(workspace_key(&a), workspace_key(&b));
    }

    #[test]
    fn base_dir_constant_is_the_documented_override() {
        assert_eq!(ENV_CHECKPOINT_DIR, "CORTEX_CHECKPOINT_DIR");
    }
}
