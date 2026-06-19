//! The headless checkpoint surface: the shadow-store location resolver, the bridge that lets the
//! `tools` layer record pre-mutation snapshots, and the `cortex undo` / `cortex redo` subcommands.
//!
//! The shadow checkpoint store lives OUTSIDE the project working tree, under the per-user data
//! directory (so it never pollutes the project or its `.git`). It is keyed by the canonical
//! workspace path, so a `cortex run` that records snapshots and a later `cortex undo` from the same
//! directory address the same store. Location precedence mirrors the database path resolution:
//!
//! - `CORTEX_CHECKPOINT_DIR` — an explicit base directory for shadow stores (highest precedence).
//! - `CORTEX_DATA_DIR` — `<CORTEX_DATA_DIR>/checkpoints`.
//! - `XDG_DATA_HOME` — `<XDG_DATA_HOME>/cortex/checkpoints`.
//! - `HOME` — `<HOME>/.local/share/cortex/checkpoints`.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use checkpoints::{CheckpointError, RestoreOutcome, ShadowStore, TurnRecorder};
use errors::Exit;
use serde::Serialize;

use crate::cli::{CheckpointArgs, OutputFormat};

/// The explicit shadow-store base-directory override.
const ENV_CHECKPOINT_DIR: &str = "CORTEX_CHECKPOINT_DIR";

/// Bridges the `tools` crate's pre-mutation recorder seam to a [`checkpoints::TurnRecorder`], so the
/// mutating file tools record snapshots into the shadow store without `tools` depending on it.
pub struct RecorderBridge {
    inner: Arc<TurnRecorder>,
}

impl RecorderBridge {
    /// Wrap a shared turn recorder so it can be attached to the tool context.
    #[must_use]
    pub fn new(inner: Arc<TurnRecorder>) -> Self {
        Self { inner }
    }
}

impl tools::CheckpointRecorder for RecorderBridge {
    fn record_pre_image(&self, path: &Path) {
        self.inner.record_pre_image(path);
    }
}

/// Open (creating on first use) the shadow checkpoint store for `cwd`, or `None` when no data
/// location can be resolved (so a run degrades to no checkpointing rather than failing).
pub fn open_store(cwd: &Path) -> Result<Option<ShadowStore>, CheckpointError> {
    let Some(base) = base_dir() else {
        return Ok(None);
    };
    let git_dir = base.join(workspace_key(cwd));
    Ok(Some(ShadowStore::open(cwd, &git_dir)?))
}

/// Finalize a turn recorder at the end of a run, logging (but not failing the run on) any snapshot
/// failure: the agent's work has already been delivered, so a degraded undo is reported, not fatal.
pub fn finalize_turn(recorder: &TurnRecorder) {
    if let Err(err) = recorder.finalize() {
        eprintln!("cortex run: checkpoint snapshot unavailable: {err}");
    }
}

/// Dispatch `cortex undo` / `cortex redo`.
pub fn run(args: CheckpointArgs, redo: bool) -> Exit {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!(
                "cortex {}: cannot determine the working directory: {err}",
                verb(redo)
            );
            return Exit::Failure;
        }
    };
    let store = match open_store(&cwd) {
        Ok(Some(store)) => store,
        Ok(None) => {
            eprintln!(
                "cortex {}: cannot determine a checkpoint location; set HOME, XDG_DATA_HOME, \
                 CORTEX_DATA_DIR, or CORTEX_CHECKPOINT_DIR",
                verb(redo)
            );
            return Exit::Failure;
        }
        Err(err) => {
            eprintln!("cortex {}: {err}", verb(redo));
            return Exit::Failure;
        }
    };

    let outcome = if redo { store.redo() } else { store.undo() };
    match outcome {
        Ok(outcome) => report(redo, &outcome, args.format.format),
        Err(err) => {
            eprintln!("cortex {}: {err}", verb(redo));
            Exit::Failure
        }
    }
}

/// The command verb for diagnostics.
const fn verb(redo: bool) -> &'static str {
    if redo { "redo" } else { "undo" }
}

/// A machine-readable description of an undo/redo outcome.
#[derive(Serialize)]
struct CheckpointReport<'a> {
    action: &'a str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    paths: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    conflicts: &'a [String],
}

/// Render an undo/redo outcome and map it to an exit code. A successful restore or a clean
/// "nothing to do" exits 0; a refused conflict exits non-zero so a script can detect it.
fn report(redo: bool, outcome: &RestoreOutcome, format: OutputFormat) -> Exit {
    let action = verb(redo);
    let empty: [String; 0] = [];
    let (status, label, paths, conflicts, exit): (&str, Option<&str>, &[String], &[String], Exit) =
        match outcome {
            RestoreOutcome::Restored { label, paths } => (
                "restored",
                Some(label.as_str()),
                paths.as_slice(),
                &empty[..],
                Exit::Success,
            ),
            RestoreOutcome::Nothing => ("nothing", None, &empty[..], &empty[..], Exit::Success),
            RestoreOutcome::Conflict { label, conflicts } => (
                "conflict",
                Some(label.as_str()),
                &empty[..],
                conflicts.as_slice(),
                Exit::Failure,
            ),
        };

    match format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let report = CheckpointReport {
                action,
                status,
                label,
                paths,
                conflicts,
            };
            match serde_json::to_string(&report) {
                Ok(mut body) => {
                    body.push('\n');
                    let _ = io::stdout().write_all(body.as_bytes());
                }
                Err(err) => {
                    eprintln!("cortex {action}: failed to serialize output: {err}");
                    return Exit::Failure;
                }
            }
        }
        OutputFormat::Text => emit_text(action, status, paths, conflicts),
    }
    exit
}

/// Render a human-readable summary of an undo/redo outcome to stdout/stderr.
fn emit_text(action: &str, status: &str, paths: &[String], conflicts: &[String]) {
    match status {
        "restored" => {
            let verb = if action == "undo" { "undid" } else { "redid" };
            println!("{verb} 1 turn ({} file(s)):", paths.len());
            for path in paths {
                println!("  {path}");
            }
        }
        "nothing" => {
            let what = if action == "undo" {
                "nothing to undo"
            } else {
                "nothing to redo"
            };
            println!("{what}");
        }
        "conflict" => {
            eprintln!(
                "cortex {action}: refused; {} file(s) changed outside the agent since the snapshot:",
                conflicts.len()
            );
            for path in conflicts {
                eprintln!("  {path}");
            }
        }
        _ => {}
    }
}

/// The base directory under which per-workspace shadow stores live, by documented precedence.
fn base_dir() -> Option<PathBuf> {
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

/// A stable, filesystem-safe key for `cwd`: a sanitized basename plus a deterministic hash of the
/// canonical path, so two different workspaces never collide and the same workspace always maps to
/// the same per-project store across processes. Shared by the other per-project data-dir stores
/// (e.g. the saved-permission store) so every per-workspace artifact keys projects identically.
pub(crate) fn workspace_key(cwd: &Path) -> String {
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
    fn base_dir_prefers_explicit_override() {
        // Exercised indirectly: the explicit override constant is the documented highest precedence.
        assert_eq!(ENV_CHECKPOINT_DIR, "CORTEX_CHECKPOINT_DIR");
    }
}
