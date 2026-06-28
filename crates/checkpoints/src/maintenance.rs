//! Snapshot maintenance: diff/diffFull retrieval, per-patch (file/hunk) revert, and store pruning.
//!
//! These operations extend the turn-granular undo/redo core ([`crate::store`]) with the finer-grained
//! review and cleanup surface a long-running agent session needs:
//!
//! - [`ShadowStore::snapshot_diff`] reports the net change of the applied turns in scope as either a
//!   per-file **summary** (status + added/removed line counts) or a **full** unified diff, in
//!   `git diff` style with real workspace-relative paths.
//! - [`ShadowStore::revert_path`] / [`ShadowStore::revert_hunk`] roll back a single file (a "patch")
//!   or a single hunk of one file WITHOUT touching the other files changed in the same turn. The
//!   revert is recorded as a fresh checkpoint turn, so it is itself undoable and shows up in history.
//! - [`ShadowStore::prune`] drops the oldest applied turns beyond a retention window and runs the
//!   shadow store's `git gc`, shrinking the object database while keeping the most recent undo turns
//!   and every redoable turn byte-exact.

use serde::{Deserialize, Serialize};

use crate::error::CheckpointError;
use crate::store::{FileImage, RestoreOutcome, ShadowStore, read_file_image, write_staged_image};

/// One file's net change in a snapshot diff: its status, line counts, and (optionally) the full
/// unified diff text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    /// Workspace-relative path, `/`-separated.
    pub path: String,
    /// One of `added`, `modified`, or `deleted`.
    pub status: String,
    /// Lines added (`+`) across the net change.
    pub additions: usize,
    /// Lines removed (`-`) across the net change.
    pub deletions: usize,
    /// The full unified diff for the file, present only when a full diff was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

/// The result of a [`ShadowStore::prune`] pass over the snapshot store.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PruneOutcome {
    /// How many old turns were dropped from the front of the history.
    pub pruned_turns: usize,
    /// How many turns remain after pruning.
    pub remaining_turns: usize,
    /// The undo cursor (applied turn count) after pruning.
    pub applied: usize,
    /// The turn count before pruning.
    pub total_before: usize,
    /// Loose+packed object files in the shadow object database before the pass.
    pub objects_before: usize,
    /// Object files after the pass (after `git gc`).
    pub objects_after: usize,
    /// Object-database byte size before the pass.
    pub bytes_before: u64,
    /// Object-database byte size after the pass.
    pub bytes_after: u64,
}

/// A single parsed hunk of a unified diff, retaining both sides for reversal.
struct Hunk {
    /// 1-based starting line of the hunk in the NEW (post) file.
    new_start: usize,
    /// Number of NEW-file lines the hunk spans.
    new_count: usize,
    /// The OLD (pre) side lines: context plus removed lines, in order.
    old_lines: Vec<String>,
    /// The NEW (post) side lines: context plus added lines, in order.
    new_lines: Vec<String>,
}

impl ShadowStore {
    /// Report the net change of the applied turns in scope as per-file [`FileChange`] entries.
    ///
    /// `label` selects the scope: `Some(id)` restricts to turns recorded under that session id,
    /// `None` spans every applied turn. With `full = false` each entry carries only its status and
    /// added/removed line counts (a summary); with `full = true` each entry also carries the full
    /// unified diff text (`git diff` style with the real workspace-relative path). Files whose net
    /// content is unchanged across the scope are omitted.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal or object database cannot be read.
    pub fn snapshot_diff(
        &self,
        label: Option<&str>,
        full: bool,
    ) -> Result<Vec<FileChange>, CheckpointError> {
        let net = self.collect_net_changes(label)?;
        if net.order.is_empty() {
            return Ok(Vec::new());
        }
        let empty_oid = self.store_blob(b"")?;
        let mut out = Vec::with_capacity(net.order.len());
        for path in net.order {
            let pre = net.pre.get(&path).cloned().flatten();
            let post = net.post.get(&path).cloned().flatten();
            if pre.as_ref().map(|image| &image.oid) == post.as_ref().map(|image| &image.oid) {
                continue;
            }
            let status = change_status(pre.as_ref(), post.as_ref());
            let pre_oid = pre.map_or_else(|| empty_oid.clone(), |image| image.oid);
            let post_oid = post.map_or_else(|| empty_oid.clone(), |image| image.oid);
            let raw = self
                .git()
                .run(&["diff", "--no-color", &pre_oid, &post_oid])?;
            let body = String::from_utf8_lossy(&raw).into_owned();
            let (additions, deletions) = count_changes(&body);
            let diff = full.then(|| render_unified(&path, status, &body));
            out.push(FileChange {
                path,
                status: status.to_owned(),
                additions,
                deletions,
                diff,
            });
        }
        Ok(out)
    }

    /// Revert one file (a "patch") to its pre-change bytes WITHOUT touching the other files changed
    /// in the same turn(s), recording the revert as a fresh undoable checkpoint turn.
    ///
    /// `label` selects the scope as in [`Self::snapshot_diff`]. The file is restored to the earliest
    /// pre-image the scope recorded for it (removing a file the scope created). If the file was
    /// modified out-of-band since the snapshot (its current bytes no longer match the recorded
    /// post-image) the revert is refused as a [`RestoreOutcome::Conflict`] and nothing is written.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal or object database cannot be read/written.
    pub fn revert_path(
        &self,
        label: Option<&str>,
        rel_path: &str,
    ) -> Result<RestoreOutcome, CheckpointError> {
        let net = self.collect_net_changes(label)?;
        if !net.order.iter().any(|path| path == rel_path) {
            return Ok(RestoreOutcome::Nothing);
        }
        let pre = net.pre.get(rel_path).cloned().flatten();
        let post = net.post.get(rel_path).cloned().flatten();
        if pre.as_ref().map(|image| &image.oid) == post.as_ref().map(|image| &image.oid) {
            return Ok(RestoreOutcome::Nothing);
        }

        let turn_label = revert_turn_label(label, rel_path);
        let absolute = self.workspace_root().join(rel_path);
        let current = read_file_image(&absolute)?;
        if !self.current_matches_expected(current.as_ref(), post.as_ref())? {
            return Ok(RestoreOutcome::Conflict {
                label: turn_label,
                conflicts: vec![rel_path.to_owned()],
            });
        }

        let target = match &pre {
            Some(image) => Some((self.read_blob(&image.oid)?, image.executable)),
            None => None,
        };
        self.record_single_file_revert(&turn_label, rel_path, &absolute, target.as_ref())?;
        Ok(RestoreOutcome::Restored {
            label: turn_label,
            paths: vec![rel_path.to_owned()],
        })
    }

    /// Revert a single hunk (0-based `hunk_index`) of one modified file, leaving that file's other
    /// hunks and every other file in the turn untouched, recorded as a fresh undoable turn.
    ///
    /// Only modified text files are supported (both a pre- and post-image exist); use
    /// [`Self::revert_path`] for files the scope added or deleted wholesale. The current on-disk
    /// bytes must still match the recorded post-image (otherwise the hunk line numbers cannot be
    /// trusted), or the revert is refused as a [`RestoreOutcome::Conflict`].
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the scope has no recorded modification for the path, the
    /// hunk index is out of range, the hunk no longer aligns with the file, or the store cannot be
    /// read/written.
    pub fn revert_hunk(
        &self,
        label: Option<&str>,
        rel_path: &str,
        hunk_index: usize,
    ) -> Result<RestoreOutcome, CheckpointError> {
        let net = self.collect_net_changes(label)?;
        if !net.order.iter().any(|path| path == rel_path) {
            return Ok(RestoreOutcome::Nothing);
        }
        let pre = net.pre.get(rel_path).cloned().flatten();
        let post = net.post.get(rel_path).cloned().flatten();
        let (Some(pre_image), Some(post_image)) = (pre, post) else {
            return Err(CheckpointError::new(format!(
                "hunk revert needs a modified file; {rel_path} was added or deleted (use a path revert)"
            )));
        };
        if pre_image.oid == post_image.oid {
            return Ok(RestoreOutcome::Nothing);
        }

        let turn_label = revert_turn_label(label, rel_path);
        let absolute = self.workspace_root().join(rel_path);
        let current = read_file_image(&absolute)?;
        let post_bytes = self.read_blob(&post_image.oid)?;
        let aligned = current
            .as_ref()
            .is_some_and(|(bytes, _)| bytes.as_slice() == post_bytes.as_slice());
        if !aligned {
            return Ok(RestoreOutcome::Conflict {
                label: turn_label,
                conflicts: vec![rel_path.to_owned()],
            });
        }
        let executable = current
            .as_ref()
            .map_or(post_image.executable, |(_, executable)| *executable);

        let raw = self
            .git()
            .run(&["diff", "--no-color", &pre_image.oid, &post_image.oid])?;
        let diff = String::from_utf8_lossy(&raw).into_owned();
        let hunks = parse_hunks(&diff)?;
        let total = hunks.len();
        let hunk = hunks.get(hunk_index).ok_or_else(|| {
            CheckpointError::new(format!(
                "hunk index {hunk_index} is out of range for {rel_path} ({total} hunk(s))"
            ))
        })?;
        let target_bytes = revert_one_hunk(&post_bytes, hunk)?;
        if target_bytes == post_bytes {
            return Ok(RestoreOutcome::Nothing);
        }

        self.record_single_file_revert(
            &turn_label,
            rel_path,
            &absolute,
            Some(&(target_bytes, executable)),
        )?;
        Ok(RestoreOutcome::Restored {
            label: turn_label,
            paths: vec![rel_path.to_owned()],
        })
    }

    /// Drop the oldest applied turns beyond a retention window and garbage-collect the shadow object
    /// database, keeping the most recent `keep_applied` undo turns and every redoable turn intact.
    ///
    /// Only turns BEFORE the undo cursor (already-applied history) are eligible for pruning, and at
    /// most `applied - keep_applied` of them (the oldest) are dropped; redoable turns ahead of the
    /// cursor are never touched. After dropping the journal entries, the anchor is rebuilt over only
    /// the surviving blobs and `git gc --prune=now` removes the now-unreachable objects. Recent
    /// undo/redo therefore keeps working byte-exact while storage shrinks.
    ///
    /// # Errors
    /// Returns a [`CheckpointError`] when the journal cannot be read/written or `git gc` fails.
    pub fn prune(&self, keep_applied: usize) -> Result<PruneOutcome, CheckpointError> {
        let (objects_before, bytes_before) = self.object_db_stats();
        let mut journal = self.load_journal()?;
        let total_before = journal.turns.len();
        let drop_count = journal.applied.saturating_sub(keep_applied);
        if drop_count > 0 {
            journal.turns.drain(0..drop_count);
            journal.applied -= drop_count;
            self.save_journal(&journal)?;
            self.reanchor(&journal)?;
        }
        // Expire ref logs first: they would otherwise pin abandoned anchor commits and keep their
        // unique blobs reachable, defeating the gc. Best-effort (a store with logs disabled has
        // none to expire).
        let _ = self
            .git()
            .run(&["reflog", "expire", "--all", "--expire=now"]);
        self.git().run(&["gc", "--prune=now", "--quiet"])?;
        let (objects_after, bytes_after) = self.object_db_stats();
        Ok(PruneOutcome {
            pruned_turns: drop_count,
            remaining_turns: journal.turns.len(),
            applied: journal.applied,
            total_before,
            objects_before,
            objects_after,
            bytes_before,
            bytes_after,
        })
    }

    /// Record a single-file revert as a fresh turn: snapshot the file's current (post) bytes as the
    /// new turn's pre-image, write the revert target, and finalize the snapshot.
    fn record_single_file_revert(
        &self,
        turn_label: &str,
        rel_path: &str,
        absolute: &std::path::Path,
        target: Option<&(Vec<u8>, bool)>,
    ) -> Result<(), CheckpointError> {
        let recorder = self.begin_turn(turn_label.to_owned());
        recorder.record_pre_image(absolute);
        write_staged_image(absolute, rel_path, target)?;
        recorder.finalize()?;
        Ok(())
    }

    /// Count object files and their total bytes under the shadow object database (`objects/`).
    fn object_db_stats(&self) -> (usize, u64) {
        let mut count = 0;
        let mut bytes = 0;
        let mut stack = vec![self.git_dir().join("objects")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    stack.push(entry.path());
                } else if file_type.is_file() {
                    count += 1;
                    if let Ok(metadata) = entry.metadata() {
                        bytes += metadata.len();
                    }
                }
            }
        }
        (count, bytes)
    }
}

/// Classify a net change from its pre/post presence.
fn change_status(pre: Option<&FileImage>, post: Option<&FileImage>) -> &'static str {
    match (pre, post) {
        (None, Some(_)) => "added",
        (Some(_), None) => "deleted",
        _ => "modified",
    }
}

/// The label for a revert turn: the scope's session id when present, else a generic marker.
fn revert_turn_label(label: Option<&str>, rel_path: &str) -> String {
    match label {
        Some(label) => label.to_owned(),
        None => format!("snapshot-revert:{rel_path}"),
    }
}

/// Count added/removed lines in a unified diff body (only counting within hunks, so the `---`/`+++`
/// file headers are excluded).
fn count_changes(diff: &str) -> (usize, usize) {
    let mut additions = 0;
    let mut deletions = 0;
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        if !in_hunk {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

/// Re-render a `git diff <oid> <oid>` body with real workspace paths in its `git diff` header, so
/// the output reads like `git diff` / `git diff --no-index` rather than naming blob object ids.
fn render_unified(path: &str, status: &str, body: &str) -> String {
    let hunks_at = if body.starts_with("@@") {
        Some(0)
    } else {
        body.find("\n@@").map(|index| index + 1)
    };
    let Some(start) = hunks_at else {
        // No textual hunks (e.g. a binary file): pass git's own description through unchanged.
        return body.to_owned();
    };
    let hunks = &body[start..];
    let old = if status == "added" {
        "/dev/null".to_owned()
    } else {
        format!("a/{path}")
    };
    let new = if status == "deleted" {
        "/dev/null".to_owned()
    } else {
        format!("b/{path}")
    };
    format!("diff --git a/{path} b/{path}\n--- {old}\n+++ {new}\n{hunks}")
}

/// Parse the hunks of a single-file unified diff.
fn parse_hunks(diff: &str) -> Result<Vec<Hunk>, CheckpointError> {
    let mut hunks = Vec::new();
    let mut lines = diff.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.starts_with("@@") {
            continue;
        }
        let (new_start, new_count) = parse_hunk_header(line)?;
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        while let Some(peeked) = lines.peek() {
            if peeked.starts_with("@@") {
                break;
            }
            let body = lines.next().unwrap_or_default();
            if let Some(rest) = body.strip_prefix(' ') {
                old_lines.push(rest.to_owned());
                new_lines.push(rest.to_owned());
            } else if let Some(rest) = body.strip_prefix('-') {
                old_lines.push(rest.to_owned());
            } else if let Some(rest) = body.strip_prefix('+') {
                new_lines.push(rest.to_owned());
            }
            // A `\ No newline at end of file` marker (or any other line) carries no content to map.
        }
        hunks.push(Hunk {
            new_start,
            new_count,
            old_lines,
            new_lines,
        });
    }
    Ok(hunks)
}

/// Parse a `@@ -a,b +c,d @@` header, returning the NEW-side start line and line count.
fn parse_hunk_header(line: &str) -> Result<(usize, usize), CheckpointError> {
    let plus = line
        .split_whitespace()
        .find(|token| token.starts_with('+'))
        .ok_or_else(|| CheckpointError::new(format!("malformed hunk header: {line}")))?;
    let mut numbers = plus[1..].split(',');
    let start = numbers
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| CheckpointError::new(format!("malformed hunk header: {line}")))?;
    let count = match numbers.next() {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| CheckpointError::new(format!("malformed hunk header: {line}")))?,
        None => 1,
    };
    Ok((start, count))
}

/// Produce the bytes of `post` with the single `hunk` rolled back to its pre-change lines, leaving
/// every other line of `post` (including other hunks' regions) intact.
fn revert_one_hunk(post: &[u8], hunk: &Hunk) -> Result<Vec<u8>, CheckpointError> {
    let text = String::from_utf8_lossy(post);
    let trailing_newline = text.ends_with('\n');
    let mut lines: Vec<String> = if text.is_empty() {
        Vec::new()
    } else {
        let mut split: Vec<String> = text.split('\n').map(str::to_owned).collect();
        if trailing_newline {
            split.pop();
        }
        split
    };

    let start_index = if hunk.new_count == 0 {
        hunk.new_start
    } else {
        hunk.new_start.saturating_sub(1)
    };
    let end_index = start_index + hunk.new_count;
    if end_index > lines.len() {
        return Err(CheckpointError::new(
            "hunk does not align with the current file content; revert refused",
        ));
    }
    if lines[start_index..end_index] != hunk.new_lines[..] {
        return Err(CheckpointError::new(
            "hunk context does not match the current file content; revert refused",
        ));
    }

    let mut reverted: Vec<String> = Vec::with_capacity(lines.len());
    reverted.extend_from_slice(&lines[..start_index]);
    reverted.extend(hunk.old_lines.iter().cloned());
    reverted.extend_from_slice(&lines[end_index..]);
    lines = reverted;

    let mut joined = lines.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    Ok(joined.into_bytes())
}
