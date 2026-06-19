//! The per-turn recorder that captures pre-mutation images and finalizes a turn snapshot.
//!
//! A recorder is created via [`ShadowStore::begin_turn`](crate::ShadowStore::begin_turn) and shared
//! with the tool layer for the duration of one turn. Each mutating tool calls
//! [`TurnRecorder::record_pre_image`] with the file it is about to change BEFORE the bytes hit the
//! disk; the recorder captures the file's current state the first time it sees a given path
//! (first-write-wins within the turn). When the turn ends the caller invokes
//! [`TurnRecorder::finalize`], which records each touched file's post-mutation state and appends the
//! complete snapshot to the store's durable journal.
//!
//! Capture failures never block a tool mutation: a failure is remembered and surfaced at
//! [`TurnRecorder::finalize`], so the tool's write still proceeds (the turn simply finalizes
//! degraded, without an undoable snapshot).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use crate::error::CheckpointError;
use crate::store::{FileImage, RecordedTurn, ShadowStore, TurnFileRecord};

/// Records the pre-images of one turn and finalizes them into a snapshot.
///
/// Interior mutability lets the recorder be shared (behind an `Arc`) with the tool context while the
/// caller retains the ability to finalize it once the turn completes.
#[derive(Debug)]
pub struct TurnRecorder {
    store: ShadowStore,
    label: String,
    state: Mutex<RecorderState>,
}

/// The accumulating pre-image state of an in-flight turn.
#[derive(Debug, Default)]
struct RecorderState {
    /// rel_path -> pre-image (`None` = the file was absent before the turn touched it).
    pre_images: BTreeMap<String, Option<FileImage>>,
    /// The first capture failure, if any; finalization surfaces it instead of a partial snapshot.
    failure: Option<CheckpointError>,
}

impl TurnRecorder {
    /// Build a recorder for `store` labelled with `label`.
    pub(crate) fn new(store: ShadowStore, label: String) -> Self {
        Self {
            store,
            label,
            state: Mutex::new(RecorderState::default()),
        }
    }

    /// Capture the pre-image of `path` (the first time the turn touches it). MUST be called before
    /// the mutation's bytes hit the disk. Infallible by contract: a capture failure is remembered
    /// and degrades the whole turn at [`Self::finalize`].
    pub fn record_pre_image(&self, path: &Path) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.failure.is_some() {
            return;
        }
        let rel = match self.store.relativize(path) {
            Ok(rel) => rel,
            Err(error) => {
                state.failure = Some(error);
                return;
            }
        };
        if state.pre_images.contains_key(&rel) {
            return;
        }
        match self.store.capture_image(path) {
            Ok(image) => {
                state.pre_images.insert(rel, image);
            }
            Err(error) => state.failure = Some(error),
        }
    }

    /// Whether at least one pre-image was captured (i.e. the turn mutated a file).
    #[must_use]
    pub fn has_mutations(&self) -> bool {
        self.state
            .lock()
            .map(|state| !state.pre_images.is_empty())
            .unwrap_or(false)
    }

    /// Whether a capture failed, so the turn will finalize degraded (no snapshot).
    #[must_use]
    pub fn is_degraded(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.failure.is_some())
            .unwrap_or(true)
    }

    /// Finalize the turn: record each touched file's post-mutation state and append the snapshot to
    /// the store's journal. Returns `Ok(None)` when the turn mutated no files (nothing to record),
    /// so read-only turns create no checkpoint churn.
    ///
    /// # Errors
    /// Returns the remembered capture failure, or any failure recording post-images or persisting
    /// the snapshot.
    pub fn finalize(&self) -> Result<Option<RecordedTurn>, CheckpointError> {
        let (pre_images, failure) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| CheckpointError::new("recorder state poisoned"))?;
            (std::mem::take(&mut state.pre_images), state.failure.take())
        };

        if let Some(failure) = failure {
            return Err(failure);
        }
        if pre_images.is_empty() {
            return Ok(None);
        }

        let mut files = Vec::with_capacity(pre_images.len());
        for (rel_path, pre) in pre_images {
            let absolute = self.store.workspace_root().join(&rel_path);
            let post = self.store.capture_image(&absolute)?;
            files.push(TurnFileRecord {
                rel_path,
                pre,
                post,
            });
        }

        let turn = RecordedTurn {
            label: self.label.clone(),
            files,
        };
        self.store.append_turn(turn.clone())?;
        Ok(Some(turn))
    }
}
