//! The headless snapshot surface: `cortex snapshot diff` / `revert` / `prune`.
//!
//! These commands operate on the same workspace-keyed shadow checkpoint store that powers
//! `cortex undo` / `cortex redo` (resolved through [`crate::checkpoint::open_store`]), exposing the
//! finer-grained review and maintenance operations the store provides: net diff/diffFull retrieval,
//! per-file (and per-hunk) revert that leaves the rest of a turn intact, and pruning that shrinks
//! the store without breaking recent undo/redo. Diagnostics go to stderr; machine output
//! (`--format json`/`--jsonl`) is strict JSON on stdout with no human chatter interleaved.

use std::io::{self, Write};

use checkpoints::{FileChange, PruneOutcome, RestoreOutcome, ShadowStore};
use errors::Exit;
use serde::Serialize;

use crate::checkpoint::open_store;
use crate::cli::{
    OutputFormat, SnapshotArgs, SnapshotCommand, SnapshotDiffArgs, SnapshotPruneArgs,
    SnapshotRevertArgs,
};

/// Dispatch a `cortex snapshot` invocation.
pub fn run(args: SnapshotArgs) -> Exit {
    let store = match resolve_store() {
        Ok(store) => store,
        Err(exit) => return exit,
    };
    match args.command {
        SnapshotCommand::Diff(args) => diff(&store, args),
        SnapshotCommand::Revert(args) => revert(&store, args),
        SnapshotCommand::Prune(args) => prune(&store, args),
    }
}

/// Resolve and open the workspace's shadow checkpoint store, mapping every failure to a clear
/// stderr diagnostic and a non-zero exit.
fn resolve_store() -> Result<ShadowStore, Exit> {
    let cwd = std::env::current_dir().map_err(|err| {
        eprintln!("cortex snapshot: cannot determine the working directory: {err}");
        Exit::Failure
    })?;
    match open_store(&cwd) {
        Ok(Some(store)) => Ok(store),
        Ok(None) => {
            eprintln!(
                "cortex snapshot: cannot determine a checkpoint location; set HOME, XDG_DATA_HOME, \
                 CORTEX_DATA_DIR, or CORTEX_CHECKPOINT_DIR"
            );
            Err(Exit::Failure)
        }
        Err(err) => {
            eprintln!("cortex snapshot: {err}");
            Err(Exit::Failure)
        }
    }
}

/// `cortex snapshot diff` — report the net file changes captured by the agent's snapshots.
fn diff(store: &ShadowStore, args: SnapshotDiffArgs) -> Exit {
    let changes = match store.snapshot_diff(args.session.as_deref(), args.full) {
        Ok(changes) => changes,
        Err(err) => {
            eprintln!("cortex snapshot diff: {err}");
            return Exit::Failure;
        }
    };

    match args.format.format {
        OutputFormat::Json => {
            let report = DiffReport {
                session: args.session.as_deref(),
                files: &changes,
            };
            emit_json(&report)
        }
        OutputFormat::Jsonl => emit_jsonl(&changes),
        OutputFormat::Text => {
            emit_diff_text(&changes, args.full);
            Exit::Success
        }
    }
}

/// `cortex snapshot revert` — revert a single file or hunk, recorded as a fresh snapshot.
fn revert(store: &ShadowStore, args: SnapshotRevertArgs) -> Exit {
    let session = args.session.as_deref();
    let outcome = match args.hunk {
        Some(index) => store.revert_hunk(session, &args.path, index),
        None => store.revert_path(session, &args.path),
    };
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("cortex snapshot revert: {err}");
            return Exit::Failure;
        }
    };
    report_revert(&outcome, args.format.format)
}

/// `cortex snapshot prune` — drop old snapshots and garbage-collect the store.
fn prune(store: &ShadowStore, args: SnapshotPruneArgs) -> Exit {
    let outcome = match store.prune(args.keep) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("cortex snapshot prune: {err}");
            return Exit::Failure;
        }
    };
    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&PruneReport::from(&outcome)),
        OutputFormat::Text => {
            emit_prune_text(&outcome);
            Exit::Success
        }
    }
}

/// The machine-readable `snapshot diff` document.
#[derive(Serialize)]
struct DiffReport<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<&'a str>,
    files: &'a [FileChange],
}

/// The machine-readable `snapshot prune` document.
#[derive(Serialize)]
struct PruneReport {
    pruned_turns: usize,
    remaining_turns: usize,
    applied: usize,
    total_before: usize,
    objects_before: usize,
    objects_after: usize,
    bytes_before: u64,
    bytes_after: u64,
}

impl From<&PruneOutcome> for PruneReport {
    fn from(outcome: &PruneOutcome) -> Self {
        Self {
            pruned_turns: outcome.pruned_turns,
            remaining_turns: outcome.remaining_turns,
            applied: outcome.applied,
            total_before: outcome.total_before,
            objects_before: outcome.objects_before,
            objects_after: outcome.objects_after,
            bytes_before: outcome.bytes_before,
            bytes_after: outcome.bytes_after,
        }
    }
}

/// The machine-readable `snapshot revert` document.
#[derive(Serialize)]
struct RevertReport<'a> {
    action: &'a str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    paths: &'a [String],
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    conflicts: &'a [String],
}

/// Render a revert outcome to the requested format and map it to an exit code (a refused conflict
/// exits non-zero so a script can detect it).
fn report_revert(outcome: &RestoreOutcome, format: OutputFormat) -> Exit {
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
            let report = RevertReport {
                action: "revert",
                status,
                label,
                paths,
                conflicts,
            };
            if emit_json(&report) == Exit::Failure {
                return Exit::Failure;
            }
        }
        OutputFormat::Text => emit_revert_text(status, paths, conflicts),
    }
    exit
}

/// Print the human-readable diff: a per-file summary line, plus the full diff text when requested.
fn emit_diff_text(changes: &[FileChange], full: bool) {
    if changes.is_empty() {
        println!("no snapshot changes");
        return;
    }
    for change in changes {
        println!(
            "{:<9} {} (+{} -{})",
            change.status, change.path, change.additions, change.deletions
        );
        if full && let Some(diff) = &change.diff {
            print!("{diff}");
            if !diff.ends_with('\n') {
                println!();
            }
        }
    }
}

/// Print a human-readable prune summary.
fn emit_prune_text(outcome: &PruneOutcome) {
    println!(
        "pruned {} old snapshot(s); {} remaining ({} undoable)",
        outcome.pruned_turns, outcome.remaining_turns, outcome.applied
    );
    println!(
        "object database: {} -> {} files, {} -> {} bytes",
        outcome.objects_before, outcome.objects_after, outcome.bytes_before, outcome.bytes_after
    );
}

/// Print a human-readable revert outcome.
fn emit_revert_text(status: &str, paths: &[String], conflicts: &[String]) {
    match status {
        "restored" => {
            println!("reverted {} file(s):", paths.len());
            for path in paths {
                println!("  {path}");
            }
        }
        "nothing" => println!("nothing to revert"),
        "conflict" => {
            eprintln!(
                "cortex snapshot revert: refused; {} file(s) changed outside the agent since the \
                 snapshot:",
                conflicts.len()
            );
            for path in conflicts {
                eprintln!("  {path}");
            }
        }
        _ => {}
    }
}

/// Serialize `value` as a single strict JSON document to stdout.
fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            let _ = io::stdout().write_all(body.as_bytes());
            Exit::Success
        }
        Err(err) => {
            eprintln!("cortex snapshot: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

/// Serialize each file change as its own JSON line (newline-delimited JSON).
fn emit_jsonl(changes: &[FileChange]) -> Exit {
    let mut out = io::stdout().lock();
    for change in changes {
        match serde_json::to_string(change) {
            Ok(mut body) => {
                body.push('\n');
                let _ = out.write_all(body.as_bytes());
            }
            Err(err) => {
                eprintln!("cortex snapshot: failed to serialize output: {err}");
                return Exit::Failure;
            }
        }
    }
    Exit::Success
}
