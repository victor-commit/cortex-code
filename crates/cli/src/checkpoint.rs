//! The headless checkpoint surface: the bridge that lets the `tools` layer record pre-mutation
//! snapshots, and the `cortex undo` / `cortex redo` subcommands.
//!
//! The shadow checkpoint store lives OUTSIDE the project working tree, under the per-user data
//! directory (so it never pollutes the project or its `.git`). It is keyed by the canonical
//! workspace path, so a `cortex run` that records snapshots and a later `cortex undo` from the same
//! directory address the same store. The location precedence (and the workspace key) live in the
//! `checkpoints` crate ([`ShadowStore::open_default`] / [`checkpoints::workspace_key`]) so every
//! surface — headless and the TUI — resolves one shared store.

use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use checkpoints::{CheckpointError, RestoreOutcome, ShadowStore, TurnRecorder};
use errors::Exit;
use serde::Serialize;

use crate::cli::{CheckpointArgs, OutputFormat};

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
/// location can be resolved (so a run degrades to no checkpointing rather than failing). Delegates
/// to [`ShadowStore::open_default`] so headless and the TUI resolve one shared store.
pub fn open_store(cwd: &Path) -> Result<Option<ShadowStore>, CheckpointError> {
    ShadowStore::open_default(cwd)
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

/// A stable, filesystem-safe per-workspace key for `cwd`, shared by the other per-project data-dir
/// stores (e.g. the saved-permission store) so every per-workspace artifact keys projects
/// identically. Delegates to [`checkpoints::workspace_key`], the single source of truth.
pub(crate) fn workspace_key(cwd: &Path) -> String {
    checkpoints::workspace_key(cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_key_is_stable_for_the_same_path() {
        let dir = std::env::temp_dir();
        assert_eq!(workspace_key(&dir), workspace_key(&dir));
    }
}
