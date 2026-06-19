//! TUI checkpoint bridge: adapts the `tools` pre-mutation recorder seam to a checkpoints turn
//! recorder, mirroring the headless `cortex run` bridge.
//!
//! The shadow checkpoint store and its undo/redo mechanics live in the `checkpoints` crate; the TUI
//! opens the store for the working directory through [`checkpoints::ShadowStore::open_default`] (the
//! same shared, workspace-keyed location the headless surface uses) so a file-affecting turn driven
//! from the TUI can be reverted (`/undo`) and re-applied (`/redo`) on disk. This module only carries
//! the small adapter that lets the `tools` mutating tools snapshot a file's prior state without the
//! `tools` crate depending on `checkpoints`.

use std::path::Path;
use std::sync::Arc;

use checkpoints::TurnRecorder;

/// Bridges the `tools` crate's pre-mutation recorder seam to a [`checkpoints::TurnRecorder`], so the
/// mutating file tools record snapshots into the shadow store for the turn without `tools` depending
/// on `checkpoints`.
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
