//! Effect-based tests for snapshot diff/diffFull retrieval, per-patch (file/hunk) revert, and store
//! pruning on a real git-backed shadow store over real temp directories.
//!
//! Maps to the gap validation assertions:
//! - VAL-GAP-DATA-005: snapshot summary and full diffs are retrievable and include every changed file.
//! - VAL-GAP-DATA-006: a selected patch/hunk reverts independently of turn-level undo and is recorded.
//! - VAL-GAP-DATA-007: pruning drops old snapshots yet keeps recent undo/redo byte-exact.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use checkpoints::{RestoreOutcome, ShadowStore};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    workspace: PathBuf,
    shadow_git_dir: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "cortex-ckpt-maint-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    let workspace = base.join("workspace");
    let shadow_git_dir = base.join("shadow-store").join("shadow.git");
    fs::create_dir_all(&workspace).expect("create workspace");
    Fixture {
        workspace: workspace.canonicalize().expect("canonicalize workspace"),
        shadow_git_dir,
    }
}

impl Fixture {
    fn open(&self) -> ShadowStore {
        ShadowStore::open(&self.workspace, &self.shadow_git_dir).expect("open shadow store")
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.workspace.join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).expect("read file")
    }
}

/// Run one "turn": record the pre-images of `targets`, run the mutation, finalize the snapshot.
fn turn(store: &ShadowStore, label: &str, targets: &[&Path], mutate: impl FnOnce()) {
    let recorder = store.begin_turn(label);
    for target in targets {
        recorder.record_pre_image(target);
    }
    mutate();
    recorder.finalize().expect("finalize turn");
}

// VAL-GAP-DATA-005: the snapshot summary and full diff include every changed file, in git style.
#[test]
fn snapshot_summary_and_full_diff_cover_every_changed_file() {
    let fx = fixture("diff");
    let store = fx.open();
    // Seed two files that already exist (so one can be modified, one deleted).
    fx.write("keep.txt", "alpha\nbeta\ngamma\n");
    fx.write("gone.txt", "to be removed\n");

    let keep = fx.path("keep.txt");
    let gone = fx.path("gone.txt");
    let added = fx.path("added.txt");
    turn(
        &store,
        "sess",
        &[keep.as_path(), gone.as_path(), added.as_path()],
        || {
            fs::write(&keep, "alpha\nBETA\ngamma\n").unwrap();
            fs::remove_file(&gone).unwrap();
            fs::write(&added, "brand new\n").unwrap();
        },
    );

    // Summary: every changed file present, with correct status and line counts, and NO diff text.
    let summary = store.snapshot_diff(None, false).expect("summary diff");
    let paths: Vec<&str> = summary.iter().map(|change| change.path.as_str()).collect();
    assert!(
        paths.contains(&"keep.txt"),
        "summary lists keep.txt: {paths:?}"
    );
    assert!(
        paths.contains(&"gone.txt"),
        "summary lists gone.txt: {paths:?}"
    );
    assert!(
        paths.contains(&"added.txt"),
        "summary lists added.txt: {paths:?}"
    );

    let keep_change = summary.iter().find(|c| c.path == "keep.txt").unwrap();
    assert_eq!(keep_change.status, "modified");
    assert_eq!(keep_change.additions, 1, "one line changed (BETA)");
    assert_eq!(keep_change.deletions, 1, "one line changed (beta)");
    assert!(keep_change.diff.is_none(), "summary carries no diff text");

    let added_change = summary.iter().find(|c| c.path == "added.txt").unwrap();
    assert_eq!(added_change.status, "added");
    assert_eq!(added_change.additions, 1);
    assert_eq!(added_change.deletions, 0);

    let gone_change = summary.iter().find(|c| c.path == "gone.txt").unwrap();
    assert_eq!(gone_change.status, "deleted");
    assert_eq!(gone_change.deletions, 1);

    // Full diff: same files, now with git-style unified diff text referencing the real paths.
    let full = store.snapshot_diff(None, true).expect("full diff");
    assert_eq!(
        full.len(),
        summary.len(),
        "full and summary cover the same files"
    );
    let keep_full = full.iter().find(|c| c.path == "keep.txt").unwrap();
    let diff = keep_full.diff.as_deref().expect("full diff text");
    assert!(
        diff.contains("diff --git a/keep.txt b/keep.txt"),
        "diff: {diff}"
    );
    assert!(diff.contains("--- a/keep.txt"), "diff: {diff}");
    assert!(diff.contains("+++ b/keep.txt"), "diff: {diff}");
    assert!(
        diff.contains("-beta") && diff.contains("+BETA"),
        "diff: {diff}"
    );

    let added_full = full.iter().find(|c| c.path == "added.txt").unwrap();
    let added_diff = added_full.diff.as_deref().expect("added diff text");
    assert!(
        added_diff.contains("--- /dev/null"),
        "added diff: {added_diff}"
    );
    assert!(
        added_diff.contains("+++ b/added.txt"),
        "added diff: {added_diff}"
    );
}

// VAL-GAP-DATA-006: reverting one file (a patch) leaves the other file changed in the same turn
// untouched, and records the revert as a fresh, undoable checkpoint turn.
#[test]
fn patch_revert_targets_one_file_and_is_recorded() {
    let fx = fixture("revert-path");
    let store = fx.open();
    fx.write("a.txt", "A0\n");
    fx.write("b.txt", "B0\n");

    let a = fx.path("a.txt");
    let b = fx.path("b.txt");
    turn(&store, "sess", &[a.as_path(), b.as_path()], || {
        fs::write(&a, "A1\n").unwrap();
        fs::write(&b, "B1\n").unwrap();
    });

    let history_before = store.history().expect("history").total_turns;

    let outcome = store.revert_path(None, "a.txt").expect("revert a.txt");
    match outcome {
        RestoreOutcome::Restored { paths, .. } => assert_eq!(paths, vec!["a.txt".to_owned()]),
        other => panic!("expected a.txt revert, got {other:?}"),
    }

    // Only a.txt is rolled back; b.txt keeps its turn change.
    assert_eq!(
        fx.read("a.txt"),
        "A0\n",
        "a.txt reverted to its pre-turn bytes"
    );
    assert_eq!(
        fx.read("b.txt"),
        "B1\n",
        "b.txt is left untouched by the patch revert"
    );

    // The revert is recorded in checkpoint history (one new turn) and is itself undoable.
    let history_after = store.history().expect("history").total_turns;
    assert_eq!(
        history_after,
        history_before + 1,
        "revert recorded as a new turn"
    );
    assert!(store.history().expect("history").can_undo());

    // Undoing the revert restores a.txt's reverted-away change (the revert behaves like any turn).
    assert!(matches!(
        store.undo().expect("undo revert"),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(
        fx.read("a.txt"),
        "A1\n",
        "undo of the revert re-applies the change"
    );
    assert_eq!(fx.read("b.txt"), "B1\n", "b.txt still untouched");
}

// VAL-GAP-DATA-006: a patch revert refuses (conflict) when the file changed out-of-band, leaving the
// external content intact.
#[test]
fn patch_revert_detects_out_of_band_conflict() {
    let fx = fixture("revert-conflict");
    let store = fx.open();
    fx.write("x.txt", "H0\n");
    let x = fx.path("x.txt");
    turn(&store, "sess", &[x.as_path()], || {
        fs::write(&x, "H1\n").unwrap();
    });

    // External edit after the snapshot.
    let external = "H1\nEXTRA\n";
    fs::write(&x, external).unwrap();

    match store.revert_path(None, "x.txt").expect("revert") {
        RestoreOutcome::Conflict { conflicts, .. } => {
            assert_eq!(conflicts, vec!["x.txt".to_owned()]);
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(
        fx.read("x.txt"),
        external,
        "the external change is preserved"
    );
}

// VAL-GAP-DATA-006: reverting a single hunk rolls back only that hunk, leaving the file's other hunk
// (and the rest of the file) at the post-change content.
#[test]
fn hunk_revert_rolls_back_only_the_selected_hunk() {
    let fx = fixture("revert-hunk");
    let store = fx.open();
    // Two edits separated by enough unchanged context that git emits two distinct hunks.
    let pre = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nl15\nl16\nl17\nl18\nl19\nl20\n";
    let post = "l1\nl2\nCHANGED-A\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nCHANGED-B\nl16\nl17\nl18\nl19\nl20\n";
    fx.write("multi.txt", pre);
    let multi = fx.path("multi.txt");
    turn(&store, "sess", &[multi.as_path()], || {
        fs::write(&multi, post).unwrap();
    });

    let history_before = store.history().expect("history").total_turns;

    // Revert the FIRST hunk only.
    match store
        .revert_hunk(None, "multi.txt", 0)
        .expect("revert hunk 0")
    {
        RestoreOutcome::Restored { paths, .. } => assert_eq!(paths, vec!["multi.txt".to_owned()]),
        other => panic!("expected hunk revert, got {other:?}"),
    }

    let result = fx.read("multi.txt");
    assert!(
        result.contains("l3\n"),
        "first hunk rolled back to l3: {result}"
    );
    assert!(
        !result.contains("CHANGED-A"),
        "first hunk no longer shows CHANGED-A: {result}"
    );
    assert!(
        result.contains("CHANGED-B"),
        "second hunk is preserved at post content: {result}"
    );
    assert_eq!(
        store.history().expect("history").total_turns,
        history_before + 1,
        "the hunk revert is recorded as a new turn"
    );
}

// VAL-GAP-DATA-007: pruning drops the oldest applied turns yet keeps the recent undo turns and the
// redo branch byte-exact, while shrinking (or at least not growing) the object database.
#[test]
fn prune_drops_old_snapshots_but_keeps_recent_undo_redo() {
    let fx = fixture("prune");
    let store = fx.open();
    fx.write("x.txt", "v0\n");
    let x = fx.path("x.txt");

    // Five sequential edits: v0 -> v1 -> ... -> v5.
    for version in 1..=5 {
        let next = format!("v{version}\n");
        turn(&store, "sess", &[x.as_path()], || {
            fs::write(&x, &next).unwrap();
        });
    }
    assert_eq!(fx.read("x.txt"), "v5\n");
    assert_eq!(store.history().expect("history").total_turns, 5);

    // Keep only the two most recent undo turns; the three oldest are pruned.
    let outcome = store.prune(2).expect("prune");
    assert_eq!(outcome.pruned_turns, 3, "three oldest turns pruned");
    assert_eq!(outcome.remaining_turns, 2);
    assert_eq!(outcome.applied, 2);
    assert!(
        outcome.objects_after <= outcome.objects_before,
        "gc does not grow the object database: {} -> {}",
        outcome.objects_before,
        outcome.objects_after
    );

    // The two most recent turns still undo byte-exact (their blobs survived gc).
    assert!(matches!(
        store.undo().expect("undo 1"),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), "v4\n", "first undo restores v4 exactly");
    assert!(matches!(
        store.undo().expect("undo 2"),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), "v3\n", "second undo restores v3 exactly");

    // History beyond the retention window is gone (the third undo is a clean no-op).
    assert!(matches!(
        store.undo().expect("undo 3"),
        RestoreOutcome::Nothing
    ));
    assert_eq!(fx.read("x.txt"), "v3\n");

    // The redo branch is intact: redo re-applies v4 then v5 byte-exact.
    assert!(matches!(
        store.redo().expect("redo 1"),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), "v4\n");
    assert!(matches!(
        store.redo().expect("redo 2"),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), "v5\n");
}

// VAL-GAP-DATA-007: pruning with a retention window at or above the applied count is a safe no-op
// (nothing dropped, undo history fully intact).
#[test]
fn prune_keeps_everything_when_retention_covers_history() {
    let fx = fixture("prune-noop");
    let store = fx.open();
    fx.write("x.txt", "v0\n");
    let x = fx.path("x.txt");
    for version in 1..=3 {
        let next = format!("v{version}\n");
        turn(&store, "sess", &[x.as_path()], || {
            fs::write(&x, &next).unwrap();
        });
    }

    let outcome = store.prune(10).expect("prune");
    assert_eq!(outcome.pruned_turns, 0);
    assert_eq!(outcome.remaining_turns, 3);

    // All three turns still undo to the origin.
    for expected in ["v2\n", "v1\n", "v0\n"] {
        assert!(matches!(
            store.undo().expect("undo"),
            RestoreOutcome::Restored { .. }
        ));
        assert_eq!(fx.read("x.txt"), expected);
    }
}
