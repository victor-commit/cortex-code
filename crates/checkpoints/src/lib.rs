//! Shadow-git checkpoint store powering turn-granular undo/redo (architecture §3.4).
//!
//! The agent's mutating file tools (`write`/`edit`/`apply_patch`, and downstream formatters) record
//! a pre-mutation snapshot BEFORE they change the working tree. The snapshots are grouped by
//! message/turn: undoing reverts every file a single turn changed as one unit, and redoing
//! re-applies it exactly. Snapshots are stored as git blobs in a standalone "shadow" git directory
//! that never touches the project's own `.git`.
//!
//! The crate is intentionally decoupled from the tool layer: the `tools` crate defines a small
//! recorder trait its mutating tools call, and a surface (the CLI) bridges that trait to the
//! [`TurnRecorder`] returned by [`ShadowStore::begin_turn`]. This crate owns only the storage and
//! the undo/redo mechanics.
//!
//! ## Lifecycle
//! 1. [`ShadowStore::open`] opens (or creates) the shadow store for a workspace.
//! 2. [`ShadowStore::begin_turn`] starts a [`TurnRecorder`] for the turn.
//! 3. Each mutating tool calls [`TurnRecorder::record_pre_image`] before writing.
//! 4. [`TurnRecorder::finalize`] records the post-images and appends the snapshot.
//! 5. [`ShadowStore::undo`] / [`ShadowStore::redo`] walk the recorded history.

mod error;
mod git;
mod location;
mod recorder;
mod store;

pub use error::CheckpointError;
pub use location::workspace_key;
pub use recorder::TurnRecorder;
pub use store::{
    FileDiff, FileImage, HistorySummary, RecordedTurn, RestoreDirection, RestoreOutcome,
    ShadowStore, TurnFileRecord,
};
