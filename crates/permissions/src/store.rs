//! Durable, cross-process persistence of saved ("always allow/deny") permission decisions.
//!
//! When an `ask` is answered with "always" — via the headless `--permission-allow-save` flag or the
//! TUI's "approve and remember" option — the choice is recorded in the session's
//! [`SavedDecisions`](crate::SavedDecisions) store. This module snapshots that store to disk so a
//! LATER, fully separate run (in EITHER surface) on the same project reuses the saved decision
//! without re-asking (VAL-CROSS-027/028): the decision survives the process exit and is shared
//! across surfaces.
//!
//! Saved decisions are **per project**: the store file lives OUTSIDE the project tree, under the
//! per-user data directory, keyed by a caller-supplied workspace key (the same key the shadow
//! checkpoint store uses — `checkpoints::workspace_key`), so two runs from the same directory
//! address the same file and two different projects never share decisions. The key is passed in
//! (rather than resolved here) so this crate stays decoupled from the checkpoint store; every
//! caller derives it from the canonical workspace path. Location precedence mirrors the database
//! and checkpoint resolvers:
//!
//! - `CORTEX_PERMISSION_DIR` — an explicit base directory for saved-decision stores (highest).
//! - `CORTEX_DATA_DIR` — `<CORTEX_DATA_DIR>/permissions`.
//! - `XDG_DATA_HOME` — `<XDG_DATA_HOME>/cortex/permissions`.
//! - `HOME` — `<HOME>/.local/share/cortex/permissions`.
//!
//! The file is plain JSON (an array of `{tool, pattern, decision}`), the [`SavedDecision`] serde
//! shape, so it is inspectable. A missing or unreadable/corrupt store is treated as "no saved
//! decisions" (the run proceeds; it never crashes on a bad cache file).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::SavedDecision;

/// The explicit saved-decision store base-directory override.
const ENV_PERMISSION_DIR: &str = "CORTEX_PERMISSION_DIR";

/// The subdirectory (under the resolved data base) holding per-project saved-decision stores.
const STORE_SUBDIR: &str = "permissions";
static STORE_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Inputs that determine the saved-decision store base directory, captured explicitly so
/// resolution is deterministic and unit-testable rather than reading the process environment
/// ambiently (mirrors the database/checkpoint path inputs).
#[derive(Debug, Clone, Default)]
pub struct StoreInputs {
    /// `CORTEX_PERMISSION_DIR`, when set and non-empty: an explicit base directory (highest).
    pub permission_dir: Option<PathBuf>,
    /// `CORTEX_DATA_DIR`, when set and non-empty: the data directory (`<dir>/permissions`).
    pub data_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when set and non-empty: the base data home for the default location.
    pub xdg_data_home: Option<PathBuf>,
    /// The user's `HOME`, used for the default `${HOME}/.local/share` data home.
    pub home: Option<PathBuf>,
}

impl StoreInputs {
    /// Populate the inputs from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            permission_dir: non_empty_var(ENV_PERMISSION_DIR),
            data_dir: non_empty_var("CORTEX_DATA_DIR"),
            xdg_data_home: non_empty_var("XDG_DATA_HOME"),
            home: non_empty_var("HOME"),
        }
    }

    /// The base directory under which per-workspace saved-decision stores live, by documented
    /// precedence, or `None` when no data location can be determined.
    #[must_use]
    fn base_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = self.permission_dir.as_ref() {
            return Some(dir.clone());
        }
        if let Some(dir) = self.data_dir.as_ref() {
            return Some(dir.join(STORE_SUBDIR));
        }
        if let Some(xdg) = self.xdg_data_home.as_ref() {
            return Some(xdg.join("cortex").join(STORE_SUBDIR));
        }
        self.home.as_ref().map(|home| {
            home.join(".local")
                .join("share")
                .join("cortex")
                .join(STORE_SUBDIR)
        })
    }
}

/// Resolve the saved-decision store file for `workspace_key` against explicit `inputs`, or `None`
/// when no data location can be determined.
#[must_use]
pub fn store_path_with(inputs: &StoreInputs, workspace_key: &str) -> Option<PathBuf> {
    inputs
        .base_dir()
        .map(|base| base.join(format!("{workspace_key}.json")))
}

/// Resolve the saved-decision store file for `workspace_key` against the process environment.
#[must_use]
fn store_path(workspace_key: &str) -> Option<PathBuf> {
    store_path_with(&StoreInputs::from_process(), workspace_key)
}

/// Read the saved decisions from a store file, in record order.
///
/// A missing file or one whose contents cannot be parsed yields an empty list (the run proceeds
/// ungated by saved decisions rather than failing on a bad/foreign cache file).
#[must_use]
pub fn read(path: &Path) -> Vec<SavedDecision> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write the saved decisions to a store file as JSON, creating the parent directory on first use.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the parent directory cannot be created or the file
/// cannot be written.
pub fn write(path: &Path, decisions: &[SavedDecision]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(decisions)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    atomic_write(path, &body)
}

/// Load the saved decisions for `workspace_key` from the process-resolved store, or an empty list
/// when no store exists yet or no data location can be determined.
#[must_use]
pub fn load_for(workspace_key: &str) -> Vec<SavedDecision> {
    store_path(workspace_key)
        .map(|path| read(&path))
        .unwrap_or_default()
}

/// Persist the saved decisions for `workspace_key` to the process-resolved store. A run with no
/// resolvable data location is a clean no-op (nothing to persist to), consistent with the
/// checkpoint store.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the resolved store file cannot be written.
pub fn save_for(workspace_key: &str, decisions: &[SavedDecision]) -> io::Result<()> {
    match store_path(workspace_key) {
        Some(path) => write(&path, decisions),
        None => Ok(()),
    }
}

/// Read an environment variable, treating an unset or empty value as absent.
fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Atomically replace `path` by writing `bytes` to a sibling temp file and renaming it in place.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(path, bytes, |temp, payload| std::fs::write(temp, payload))
}

fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    write_temp: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "permissions.json".to_owned());
    let unique = STORE_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        unique
    ));

    if let Err(err) = write_temp(&temp, bytes) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::Decision;

    /// A unique, freshly created empty temp directory for hermetic store tests.
    fn scratch_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortex-perm-store-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn explicit_permission_dir_wins_and_is_workspace_keyed() {
        let base = scratch_dir("base");
        let inputs = StoreInputs {
            permission_dir: Some(base.clone()),
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        let path = store_path_with(&inputs, "projkey").expect("path");
        assert!(path.starts_with(&base), "explicit dir wins: {path:?}");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("projkey.json")
        );
    }

    #[test]
    fn data_dir_override_places_store_under_permissions_subdir() {
        let inputs = StoreInputs {
            permission_dir: None,
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        let path = store_path_with(&inputs, "k").expect("path");
        assert!(
            path.starts_with(PathBuf::from("/data").join(STORE_SUBDIR)),
            "got: {path:?}"
        );
    }

    #[test]
    fn xdg_then_home_fallbacks_resolve() {
        let xdg = StoreInputs {
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
            ..StoreInputs::default()
        };
        assert!(
            store_path_with(&xdg, "k")
                .expect("xdg path")
                .starts_with(PathBuf::from("/xdg").join("cortex").join(STORE_SUBDIR))
        );

        let home = StoreInputs {
            home: Some(PathBuf::from("/home/u")),
            ..StoreInputs::default()
        };
        assert!(
            store_path_with(&home, "k")
                .expect("home path")
                .starts_with(PathBuf::from("/home/u").join(".local").join("share"))
        );
    }

    #[test]
    fn unresolvable_without_any_location() {
        let inputs = StoreInputs::default();
        assert!(store_path_with(&inputs, "k").is_none());
    }

    #[test]
    fn same_workspace_resolves_to_the_same_store_path() {
        let inputs = StoreInputs {
            permission_dir: Some(scratch_dir("stable-base")),
            ..StoreInputs::default()
        };
        assert_eq!(
            store_path_with(&inputs, "projkey"),
            store_path_with(&inputs, "projkey"),
            "the same workspace must always map to the same store file"
        );
    }

    #[test]
    fn missing_store_reads_as_empty() {
        let dir = scratch_dir("missing");
        assert!(read(&dir.join("absent.json")).is_empty());
    }

    #[test]
    fn corrupt_store_reads_as_empty_not_a_crash() {
        let dir = scratch_dir("corrupt");
        let path = dir.join("bad.json");
        std::fs::write(&path, b"{ this is not valid json ][").unwrap();
        assert!(
            read(&path).is_empty(),
            "a corrupt store file must degrade to no saved decisions"
        );
    }

    #[test]
    fn write_then_read_round_trips_decisions_in_order() {
        let dir = scratch_dir("round-trip");
        let path = dir.join("nested").join("store.json");
        let decisions = vec![
            SavedDecision::new("bash", "echo *", Decision::Allow),
            SavedDecision::new("write", "src/**", Decision::Deny),
        ];
        write(&path, &decisions).expect("write store");
        assert!(path.exists(), "write creates the store (and parent dirs)");
        assert_eq!(read(&path), decisions);
    }

    #[test]
    fn interrupted_atomic_write_leaves_the_previous_store_unchanged() {
        let dir = scratch_dir("atomic-fail");
        let path = dir.join("store.json");
        let initial = vec![SavedDecision::new("bash", "echo *", Decision::Allow)];
        write(&path, &initial).expect("seed store");
        let before = std::fs::read(&path).expect("read seeded bytes");

        let replacement =
            serde_json::to_vec_pretty(&vec![SavedDecision::new("write", "src/**", Decision::Deny)])
                .expect("encode replacement");
        let result = atomic_write_with(&path, &replacement, |temp, bytes| {
            let partial = &bytes[..bytes.len() / 2];
            std::fs::write(temp, partial)?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "simulated interrupted write",
            ))
        });

        assert!(result.is_err(), "the synthetic interruption should fail");
        assert_eq!(
            std::fs::read(&path).expect("read bytes after failure"),
            before,
            "the existing store file should remain byte-for-byte intact"
        );
        assert_eq!(read(&path), initial);
    }
}
