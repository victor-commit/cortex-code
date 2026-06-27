//! The shadow checkpoint store: a git-backed snapshot store that powers turn-granular undo/redo.
//!
//! The store owns a standalone git directory (the "shadow" repository) whose nominal work tree is
//! the workspace root. The project's own `.git` is never touched: every git command addresses the
//! shadow directory through `--git-dir`, and the shadow directory itself lives wherever the caller
//! places it (the CLI puts it under the data directory, outside the project tree, so it produces no
//! untracked pollution).
//!
//! A *turn* is the unit of undo: all file mutations a single message/turn produces are captured
//! together and reverted together. Each touched file is stored as a pair of images — the bytes
//! before the turn first touched it (`pre`) and the bytes after the turn finished (`post`) — realized
//! as git blobs in the shadow object database. An absent file is represented by `None`, so creation
//! (`pre = None`) and deletion (`post = None`) are reversed exactly.
//!
//! The recorded turns form a linear history with a cursor (`applied`): undo walks the cursor back
//! (restoring each turn's `pre` images), redo walks it forward (restoring `post` images). Recording
//! a fresh turn while the cursor is behind the end truncates the abandoned redo branch, so a new
//! edit after an undo permanently supersedes what was undone.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::error::CheckpointError;
use crate::git::Git;
use crate::recorder::TurnRecorder;

/// The journal file name inside the shadow git directory.
const JOURNAL_FILE: &str = "cortex-journal.json";

/// The ref keeping every journal-referenced blob reachable in the shadow object database.
const ANCHOR_REF: &str = "refs/heads/cortex-snapshots";
static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static RESTORE_FAIL_AFTER_STEP: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(usize::MAX);

/// One stored image of a file: the git blob object id of its content plus its executable bit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileImage {
    /// The git blob object id holding the file's exact bytes.
    pub oid: String,
    /// `true` when the file carried the executable bit at capture time.
    pub executable: bool,
}

/// The pre/post images of one file touched during a turn. `None` means the file was absent at that
/// point (a creation has `pre == None`, a deletion has `post == None`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TurnFileRecord {
    /// Path relative to the workspace root, `/`-separated.
    pub rel_path: String,
    /// The image before the turn first touched the file.
    pub pre: Option<FileImage>,
    /// The image after the turn completed.
    pub post: Option<FileImage>,
}

/// A finalized turn snapshot: a human label plus the per-file pre/post images it can restore.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecordedTurn {
    /// A human-facing label for the turn (e.g. the session id), informational only.
    pub label: String,
    /// The per-file pre/post images captured for the turn.
    pub files: Vec<TurnFileRecord>,
}

/// The durable on-disk record of recorded turns and the undo/redo cursor.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Journal {
    /// The recorded turns in chronological order.
    pub turns: Vec<RecordedTurn>,
    /// How many leading turns are currently applied to the working tree. Undo decrements it, redo
    /// increments it, and recording a new turn truncates everything past it before appending.
    pub applied: usize,
}

impl Journal {
    /// Every distinct blob object id the journal references (across all turns' pre and post images),
    /// used to keep them reachable in the shadow object database.
    fn referenced_oids(&self) -> BTreeSet<String> {
        let mut oids = BTreeSet::new();
        for turn in &self.turns {
            for file in &turn.files {
                if let Some(image) = &file.pre {
                    oids.insert(image.oid.clone());
                }
                if let Some(image) = &file.post {
                    oids.insert(image.oid.clone());
                }
            }
        }
        oids
    }
}

/// Which direction a restore walks the timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreDirection {
    /// Undo: the working tree is expected to hold each file's `post` image; it is rewritten to the
    /// `pre` image.
    Undo,
    /// Redo: the working tree is expected to hold each file's `pre` image; it is rewritten to the
    /// `post` image.
    Redo,
}

/// The result of an undo or redo request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestoreOutcome {
    /// A turn was restored: the affected relative paths, in sorted order.
    Restored {
        /// A label identifying the restored turn.
        label: String,
        /// The relative paths whose bytes were rewritten (or removed) to the target image.
        paths: Vec<String>,
    },
    /// There was nothing to undo (cursor at the start) or redo (cursor at the end).
    Nothing,
    /// The restore was refused because one or more files were modified out-of-band since the
    /// snapshot, so restoring would clobber unrelated changes. No file was rewritten.
    Conflict {
        /// A label identifying the turn that could not be restored.
        label: String,
        /// The relative paths whose current content does not match the expected snapshot image.
        conflicts: Vec<String>,
    },
}

/// One staged restore operation, carrying both the current bytes (for rollback) and the target
/// bytes to apply.
struct StagedRestoreEntry {
    rel_path: String,
    absolute: PathBuf,
    before: Option<(Vec<u8>, bool)>,
    target: Option<(Vec<u8>, bool)>,
}

/// A summary of the store's current history, for inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistorySummary {
    /// The total number of recorded turns (applied plus redoable).
    pub total_turns: usize,
    /// How many turns are currently applied (the undo cursor).
    pub applied: usize,
}

impl HistorySummary {
    /// Whether an undo would restore a turn.
    #[must_use]
    pub const fn can_undo(&self) -> bool {
        self.applied > 0
    }

    /// Whether a redo would re-apply a turn.
    #[must_use]
    pub const fn can_redo(&self) -> bool {
        self.applied < self.total_turns
    }
}

/// A computed file-level diff across a session's recorded turns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    /// Workspace-relative path, `/`-separated.
    pub path: String,
    /// One of `added`, `modified`, or `deleted`.
    pub status: String,
    /// Unified diff between the session's pre-image and post-image of the file.
    pub diff: String,
}

/// A handle on the shadow checkpoint store for one workspace.
#[derive(Clone, Debug)]
pub struct ShadowStore {
    workspace_root: PathBuf,
    git: Git,
}

impl ShadowStore {
    /// Open (creating on first use) the shadow store whose git directory is `git_dir` and whose
    /// snapshotted work tree is `workspace_root`. The project's own `.git` is never touched.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the workspace root cannot be canonicalized or the shadow
    /// git directory cannot be created/initialized.
    pub fn open(workspace_root: &Path, git_dir: &Path) -> Result<Self, CheckpointError> {
        let workspace_root = workspace_root
            .canonicalize()
            .map_err(|error| CheckpointError::new(format!("workspace root: {error}")))?;
        let git = Git::new(git_dir.to_path_buf(), workspace_root.clone());
        let store = Self {
            workspace_root,
            git,
        };
        if !store.git.git_dir().join("HEAD").is_file() {
            fs::create_dir_all(store.git.git_dir())
                .map_err(|error| CheckpointError::new(format!("create shadow dir: {error}")))?;
            store.git.run(&["init", "--quiet"])?;
        }
        Ok(store)
    }

    /// The workspace root this store snapshots.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// The shadow git directory location.
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        self.git.git_dir()
    }

    /// Begin recording a turn under `label`. Pre-images are captured through the returned recorder
    /// as the turn's tools are about to mutate files; [`TurnRecorder::finalize`] then records the
    /// post-images and appends the snapshot.
    #[must_use]
    pub fn begin_turn(&self, label: impl Into<String>) -> TurnRecorder {
        TurnRecorder::new(self.clone(), label.into())
    }

    /// A summary of the recorded history and the undo/redo cursor.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal cannot be read.
    pub fn history(&self) -> Result<HistorySummary, CheckpointError> {
        let journal = self.load_journal()?;
        Ok(HistorySummary {
            total_turns: journal.turns.len(),
            applied: journal.applied,
        })
    }

    /// Compute the net file diffs produced by every applied turn recorded under `label`.
    ///
    /// For each file the session touched, the earliest pre-image and the latest post-image are
    /// compared; files whose content is unchanged across the whole session are omitted. The diff is
    /// the unified diff between those two images, so it reflects exactly what the session changed on
    /// disk. Only applied turns (those still in effect, ahead of the undo cursor) are considered.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal or object database cannot be read.
    pub fn session_file_diffs(&self, label: &str) -> Result<Vec<FileDiff>, CheckpointError> {
        let journal = self.load_journal()?;
        let mut order: Vec<String> = Vec::new();
        let mut pre: BTreeMap<String, Option<FileImage>> = BTreeMap::new();
        let mut post: BTreeMap<String, Option<FileImage>> = BTreeMap::new();
        for turn in journal.turns.iter().take(journal.applied) {
            if turn.label != label {
                continue;
            }
            for file in &turn.files {
                if !pre.contains_key(&file.rel_path) {
                    pre.insert(file.rel_path.clone(), file.pre.clone());
                    order.push(file.rel_path.clone());
                }
                post.insert(file.rel_path.clone(), file.post.clone());
            }
        }
        if order.is_empty() {
            return Ok(Vec::new());
        }
        let empty_oid = self.store_blob(b"")?;
        let mut diffs = Vec::with_capacity(order.len());
        for path in order {
            let pre_image = pre.get(&path).cloned().flatten();
            let post_image = post.get(&path).cloned().flatten();
            if pre_image.as_ref().map(|image| &image.oid)
                == post_image.as_ref().map(|image| &image.oid)
            {
                continue;
            }
            let status = match (&pre_image, &post_image) {
                (None, Some(_)) => "added",
                (Some(_), None) => "deleted",
                _ => "modified",
            };
            let pre_oid = pre_image.map_or_else(|| empty_oid.clone(), |image| image.oid);
            let post_oid = post_image.map_or_else(|| empty_oid.clone(), |image| image.oid);
            let raw = self.git.run(&["diff", "--no-color", &pre_oid, &post_oid])?;
            diffs.push(FileDiff {
                path,
                status: status.to_owned(),
                diff: String::from_utf8_lossy(&raw).into_owned(),
            });
        }
        Ok(diffs)
    }

    /// Undo the most recently applied turn, restoring every file it changed to its pre-turn bytes
    /// (deleting files the turn created). All-or-nothing: if any file was modified out-of-band since
    /// the snapshot the undo is refused and nothing is rewritten.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal or the object database cannot be read/written.
    pub fn undo(&self) -> Result<RestoreOutcome, CheckpointError> {
        let mut journal = self.load_journal()?;
        if journal.applied == 0 {
            return Ok(RestoreOutcome::Nothing);
        }
        let index = journal.applied - 1;
        let turn = journal.turns[index].clone();
        let outcome = self.restore(&turn, RestoreDirection::Undo)?;
        if let RestoreOutcome::Restored { .. } = &outcome {
            journal.applied = index;
            self.save_journal(&journal)?;
        }
        Ok(outcome)
    }

    /// Redo the next undone turn, re-applying every file change it made (recreating files it
    /// created, re-deleting files it deleted). All-or-nothing, mirroring [`Self::undo`].
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal or the object database cannot be read/written.
    pub fn redo(&self) -> Result<RestoreOutcome, CheckpointError> {
        let mut journal = self.load_journal()?;
        if journal.applied >= journal.turns.len() {
            return Ok(RestoreOutcome::Nothing);
        }
        let index = journal.applied;
        let turn = journal.turns[index].clone();
        let outcome = self.restore(&turn, RestoreDirection::Redo)?;
        if let RestoreOutcome::Restored { .. } = &outcome {
            journal.applied = index + 1;
            self.save_journal(&journal)?;
        }
        Ok(outcome)
    }

    /// Restore one turn in `direction`, refusing the whole operation if any file was modified
    /// out-of-band (conflict detection, all-or-nothing).
    fn restore(
        &self,
        turn: &RecordedTurn,
        direction: RestoreDirection,
    ) -> Result<RestoreOutcome, CheckpointError> {
        let mut staged = Vec::with_capacity(turn.files.len());
        let mut conflicts = Vec::new();
        for file in &turn.files {
            let absolute = self.workspace_root.join(&file.rel_path);
            let current = read_file_image(&absolute)?;
            let expected = match direction {
                RestoreDirection::Undo => &file.post,
                RestoreDirection::Redo => &file.pre,
            };
            if !self.current_matches_expected(current.as_ref(), expected.as_ref())? {
                conflicts.push(file.rel_path.clone());
                continue;
            }
            let target = match direction {
                RestoreDirection::Undo => &file.pre,
                RestoreDirection::Redo => &file.post,
            };
            let target = match target {
                Some(image) => Some((self.read_blob(&image.oid)?, image.executable)),
                None => None,
            };
            staged.push(StagedRestoreEntry {
                rel_path: file.rel_path.clone(),
                absolute,
                before: current,
                target,
            });
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            return Ok(RestoreOutcome::Conflict {
                label: turn.label.clone(),
                conflicts,
            });
        }

        let mut paths = Vec::with_capacity(staged.len());
        let mut applied = Vec::with_capacity(staged.len());
        for (index, entry) in staged.iter().enumerate() {
            let write_result = maybe_fail_restore_step(index + 1, &entry.rel_path).and_then(|()| {
                write_staged_image(&entry.absolute, &entry.rel_path, entry.target.as_ref())
            });
            if let Err(error) = write_result {
                return match self.rollback_staged_restore(&staged, &applied) {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(CheckpointError::new(format!(
                        "{}; rollback failed: {}",
                        error.detail(),
                        rollback_error.detail()
                    ))),
                };
            }
            applied.push(index);
            paths.push(entry.rel_path.clone());
        }
        paths.sort();
        Ok(RestoreOutcome::Restored {
            label: turn.label.clone(),
            paths,
        })
    }

    /// Whether the current on-disk bytes match the expected snapshot image.
    fn current_matches_expected(
        &self,
        current: Option<&(Vec<u8>, bool)>,
        expected: Option<&FileImage>,
    ) -> Result<bool, CheckpointError> {
        match (current, expected) {
            (None, None) => Ok(true),
            (None, Some(_)) | (Some(_), None) => Ok(false),
            (Some((bytes, _)), Some(expected)) => Ok(self.hash_bytes(bytes)? == expected.oid),
        }
    }

    /// Best-effort rollback for a failed multi-file restore: restore each already-written file to
    /// the bytes observed before this restore started, in reverse order.
    fn rollback_staged_restore(
        &self,
        staged: &[StagedRestoreEntry],
        applied_indices: &[usize],
    ) -> Result<(), CheckpointError> {
        for index in applied_indices.iter().copied().rev() {
            let entry = &staged[index];
            write_staged_image(&entry.absolute, &entry.rel_path, entry.before.as_ref())?;
        }
        Ok(())
    }

    /// Store `bytes` as a blob in the shadow object database, returning its object id.
    pub(crate) fn store_blob(&self, bytes: &[u8]) -> Result<String, CheckpointError> {
        Ok(String::from_utf8_lossy(
            &self
                .git
                .run_stdin(&["hash-object", "-w", "--stdin"], bytes)?,
        )
        .trim()
        .to_owned())
    }

    /// Compute the object id `bytes` would hash to (without writing it).
    fn hash_bytes(&self, bytes: &[u8]) -> Result<String, CheckpointError> {
        Ok(
            String::from_utf8_lossy(&self.git.run_stdin(&["hash-object", "--stdin"], bytes)?)
                .trim()
                .to_owned(),
        )
    }

    /// Read a blob's bytes back from the shadow object database.
    fn read_blob(&self, oid: &str) -> Result<Vec<u8>, CheckpointError> {
        self.git.run(&["cat-file", "blob", oid])
    }

    /// Capture the workspace-relative path of `path`, resolving it against the workspace root and
    /// tolerating a symlinked ancestor (e.g. a symlinked temp dir) so the relative key is stable.
    pub(crate) fn relativize(&self, path: &Path) -> Result<String, CheckpointError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };
        let relative = match absolute.strip_prefix(&self.workspace_root) {
            Ok(relative) => relative.to_path_buf(),
            Err(_) => self.relativize_via_canonical_parent(&absolute)?,
        };
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if normalized.is_empty() {
            return Err(CheckpointError::new("empty relative path"));
        }
        Ok(normalized)
    }

    /// Fallback relativization for a path whose ancestor (e.g. the temp root) is a symlink: resolve
    /// the existing parent to its canonical form, then strip the workspace root.
    fn relativize_via_canonical_parent(&self, absolute: &Path) -> Result<PathBuf, CheckpointError> {
        let parent = absolute
            .parent()
            .ok_or_else(|| CheckpointError::new("path has no parent"))?;
        let file_name = absolute
            .file_name()
            .ok_or_else(|| CheckpointError::new("path has no file name"))?;
        let canonical_parent = parent.canonicalize().map_err(|error| {
            CheckpointError::new(format!("canonicalize {}: {error}", parent.display()))
        })?;
        canonical_parent
            .join(file_name)
            .strip_prefix(&self.workspace_root)
            .map(Path::to_path_buf)
            .map_err(|_| {
                CheckpointError::new(format!(
                    "path {} is outside the workspace root",
                    absolute.display()
                ))
            })
    }

    /// Capture the current on-disk image of `path` (its bytes stored as a blob), or `None` when the
    /// file is absent.
    pub(crate) fn capture_image(&self, path: &Path) -> Result<Option<FileImage>, CheckpointError> {
        match read_file_image(path)? {
            None => Ok(None),
            Some((bytes, executable)) => Ok(Some(FileImage {
                oid: self.store_blob(&bytes)?,
                executable,
            })),
        }
    }

    /// Append `turn` to the journal, truncating any abandoned redo branch first, advancing the
    /// cursor, and keeping all referenced blobs reachable.
    pub(crate) fn append_turn(&self, turn: RecordedTurn) -> Result<(), CheckpointError> {
        let mut journal = self.load_journal()?;
        journal.turns.truncate(journal.applied);
        journal.turns.push(turn);
        journal.applied = journal.turns.len();
        self.save_journal(&journal)?;
        self.reanchor(&journal)?;
        Ok(())
    }

    /// The path of the journal file inside the shadow git directory.
    fn journal_path(&self) -> PathBuf {
        self.git.git_dir().join(JOURNAL_FILE)
    }

    /// Load the journal, treating an absent file as an empty history.
    fn load_journal(&self) -> Result<Journal, CheckpointError> {
        let path = self.journal_path();
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| CheckpointError::new(format!("read journal: {error}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Journal::default()),
            Err(error) => Err(CheckpointError::new(format!("read journal: {error}"))),
        }
    }

    /// Persist the journal atomically (write to a temp file, then rename over the target).
    fn save_journal(&self, journal: &Journal) -> Result<(), CheckpointError> {
        let path = self.journal_path();
        let bytes = serde_json::to_vec_pretty(journal)
            .map_err(|error| CheckpointError::new(format!("encode journal: {error}")))?;
        atomic_write_file(&path, &bytes, false)
            .map_err(|error| CheckpointError::new(format!("write journal: {error}")))?;
        Ok(())
    }

    /// Rebuild the anchor commit so every blob the journal references stays reachable in the shadow
    /// object database (so `cat-file` succeeds across processes and survives a future `git gc`).
    fn reanchor(&self, journal: &Journal) -> Result<(), CheckpointError> {
        let oids = journal.referenced_oids();
        let index_path = self
            .git
            .git_dir()
            .join(format!("cortex-anchor-index-{}", std::process::id()));
        let index = index_path.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", index.as_str())];

        let result = self.build_anchor(&oids, &env);
        let _ = fs::remove_file(&index_path);
        result
    }

    /// Build the anchor tree/commit from `oids` using a dedicated index, then point the anchor ref
    /// at it. Each blob is recorded in a flat tree keyed by its own object id.
    fn build_anchor(
        &self,
        oids: &BTreeSet<String>,
        env: &[(&str, &str)],
    ) -> Result<(), CheckpointError> {
        self.git.run_with(&["read-tree", "--empty"], env, None)?;
        for oid in oids {
            let cacheinfo = format!("100644,{oid},{oid}");
            self.git.run_with(
                &["update-index", "--add", "--cacheinfo", cacheinfo.as_str()],
                env,
                None,
            )?;
        }
        let tree = String::from_utf8_lossy(&self.git.run_with(&["write-tree"], env, None)?)
            .trim()
            .to_owned();
        let identity = [
            ("GIT_AUTHOR_NAME", "cortex"),
            ("GIT_AUTHOR_EMAIL", "cortex@localhost"),
            ("GIT_COMMITTER_NAME", "cortex"),
            ("GIT_COMMITTER_EMAIL", "cortex@localhost"),
        ];
        let commit = String::from_utf8_lossy(&self.git.run_with(
            &[
                "commit-tree",
                tree.as_str(),
                "-m",
                "cortex checkpoint anchor",
            ],
            &identity,
            None,
        )?)
        .trim()
        .to_owned();
        self.git.run(&["update-ref", ANCHOR_REF, commit.as_str()])?;
        Ok(())
    }
}

/// Write `image` to `absolute` (deleting the file when `image` is `None`), atomically per file.
fn write_staged_image(
    absolute: &Path,
    rel_path: &str,
    image: Option<&(Vec<u8>, bool)>,
) -> Result<(), CheckpointError> {
    match image {
        None => {
            if absolute.exists() {
                fs::remove_file(absolute)
                    .map_err(|error| CheckpointError::new(format!("delete {rel_path}: {error}")))?;
            }
            Ok(())
        }
        Some((bytes, executable)) => {
            if let Some(parent) = absolute.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    CheckpointError::new(format!("restore parents of {rel_path}: {error}"))
                })?;
            }
            atomic_write_file(absolute, bytes, *executable)
                .map_err(|error| CheckpointError::new(format!("restore {rel_path}: {error}")))
        }
    }
}

/// Atomically replace `path` by writing to a sibling temp file and renaming it in place.
fn atomic_write_file(path: &Path, bytes: &[u8], executable: bool) -> io::Result<()> {
    atomic_write_file_with(path, bytes, executable, |temp, payload| {
        fs::write(temp, payload)
    })
}

fn atomic_write_file_with(
    path: &Path,
    bytes: &[u8],
    executable: bool,
    write_temp: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let unique = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(
        ".{file_name}.cortex-atomic-{}.{}.tmp",
        std::process::id(),
        unique
    ));

    if let Err(error) = write_temp(&temp, bytes) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if executable { 0o755 } else { 0o644 };
        let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(mode));
    }
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
fn maybe_fail_restore_step(step: usize, rel_path: &str) -> Result<(), CheckpointError> {
    if RESTORE_FAIL_AFTER_STEP.load(std::sync::atomic::Ordering::Relaxed) == step {
        return Err(CheckpointError::new(format!(
            "simulated restore failure at step {step} for {rel_path}"
        )));
    }
    Ok(())
}

#[cfg(not(test))]
fn maybe_fail_restore_step(_step: usize, _rel_path: &str) -> Result<(), CheckpointError> {
    Ok(())
}

/// Read a file's bytes and executable bit; `Ok(None)` when the file is absent.
fn read_file_image(path: &Path) -> Result<Option<(Vec<u8>, bool)>, CheckpointError> {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CheckpointError::new(format!(
            "stat {}: {error}",
            path.display()
        ))),
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(CheckpointError::new(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
            let executable = file_is_executable(&metadata);
            let bytes = fs::read(path).map_err(|error| {
                CheckpointError::new(format!("read {}: {error}", path.display()))
            })?;
            Ok(Some((bytes, executable)))
        }
    }
}

/// Whether `metadata` carries the executable bit (always `false` off Unix).
fn file_is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-checkpoint-store-{tag}-{}-{}",
            std::process::id(),
            ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    struct RestoreFailReset;

    impl Drop for RestoreFailReset {
        fn drop(&mut self) {
            RESTORE_FAIL_AFTER_STEP.store(usize::MAX, Ordering::Relaxed);
        }
    }

    #[test]
    fn interrupted_atomic_journal_write_leaves_previous_bytes_intact() {
        let dir = scratch_dir("atomic");
        let path = dir.join("journal.json");
        fs::write(&path, b"{\"turns\":[]}\n").expect("seed journal");
        let before = fs::read(&path).expect("read seeded bytes");

        let result = atomic_write_file_with(&path, b"{\"corrupt\":true}", false, |temp, bytes| {
            fs::write(temp, &bytes[..bytes.len() / 2])?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "simulated interrupted write",
            ))
        });
        assert!(result.is_err(), "the synthetic interruption should fail");
        assert_eq!(fs::read(&path).expect("read journal"), before);
    }

    #[test]
    fn undo_rolls_back_if_restore_fails_midway() {
        let dir = scratch_dir("restore-rollback");
        let workspace = dir.join("workspace");
        let shadow = dir.join("shadow.git");
        fs::create_dir_all(&workspace).expect("create workspace");
        let store = ShadowStore::open(&workspace, &shadow).expect("open shadow store");

        let a = workspace.join("a.txt");
        let b = workspace.join("b.txt");
        fs::write(&a, "A0\n").unwrap();
        fs::write(&b, "B0\n").unwrap();

        let recorder = store.begin_turn("turn-1");
        recorder.record_pre_image(&a);
        recorder.record_pre_image(&b);
        fs::write(&a, "A1\n").unwrap();
        fs::write(&b, "B1\n").unwrap();
        recorder.finalize().expect("finalize");

        RESTORE_FAIL_AFTER_STEP.store(2, Ordering::Relaxed);
        let _reset = RestoreFailReset;
        let result = store.undo();
        assert!(
            result.is_err(),
            "simulated mid-restore failure should error"
        );

        // Undo failed, so both files remain at the pre-undo tree state (no half-applied restore).
        assert_eq!(fs::read_to_string(&a).unwrap(), "A1\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "B1\n");
        assert!(
            store.history().expect("history").can_undo(),
            "a failed restore must leave the undo cursor unchanged"
        );
    }
}
