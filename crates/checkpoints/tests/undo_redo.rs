//! Effect-based tests for the shadow checkpoint store: real git-backed snapshots over real temp
//! directories, asserting byte-exact undo/redo at turn granularity. Each test maps to a checkpoint
//! validation assertion (VAL-CKPT-001..005, 008, 010).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use checkpoints::{RestoreOutcome, ShadowStore};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh, canonicalized workspace directory plus a sibling shadow git directory that lives
/// OUTSIDE the workspace tree (so the store produces no in-tree pollution).
struct Fixture {
    workspace: PathBuf,
    shadow_git_dir: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base =
        std::env::temp_dir().join(format!("cortex-ckpt-{tag}-{}-{unique}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let workspace = base.join("workspace");
    let shadow_git_dir = base.join("shadow.git");
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

// VAL-CKPT-001: a snapshot capturing the pre-mutation state is recorded BEFORE the write, so it can
// restore the prior bytes.
#[test]
fn snapshot_records_pre_mutation_state() {
    let fx = fixture("pre-snapshot");
    let store = fx.open();
    fx.write("x.txt", "BEFORE");

    let x = fx.path("x.txt");
    turn(&store, "t1", &[x.as_path()], || {
        fs::write(&x, "AFTER").unwrap();
    });

    // A snapshot exists for the turn.
    let history = store.history().expect("history");
    assert_eq!(history.total_turns, 1);
    assert!(history.can_undo());

    // The recorded snapshot equals the file's state immediately before the write: undo restores it.
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), "BEFORE");
}

// VAL-CKPT-002 / VAL-CKPT-003: undo restores byte-for-byte; a subsequent redo re-applies exactly.
#[test]
fn undo_then_redo_round_trips_bytes() {
    let fx = fixture("undo-redo");
    let store = fx.open();
    fx.write("x.txt", "H0-content\nline two\n");
    let h0 = fx.read("x.txt");

    let x = fx.path("x.txt");
    turn(&store, "edit", &[x.as_path()], || {
        fs::write(&x, "H1-different\n").unwrap();
    });
    let h1 = fx.read("x.txt");
    assert_ne!(h0, h1);

    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), h0, "undo returns X to byte-identical H0");

    assert!(matches!(
        store.redo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("x.txt"), h1, "redo returns X to H1");
}

// VAL-CKPT-004: a turn that writes A and edits B is undone as one unit, while a file C changed in an
// earlier turn is unchanged.
#[test]
fn turn_granular_undo_restores_all_turn_files_and_leaves_prior_turns() {
    let fx = fixture("turn-granular");
    let store = fx.open();
    fx.write("c.txt", "C0");
    fx.write("b.txt", "B0");

    // Turn 1: change C only.
    let c = fx.path("c.txt");
    turn(&store, "turn-1", &[c.as_path()], || {
        fs::write(&c, "C1").unwrap();
    });

    // Turn 2: write a brand-new A and edit B together.
    let a = fx.path("a.txt");
    let b = fx.path("b.txt");
    turn(&store, "turn-2", &[a.as_path(), b.as_path()], || {
        fs::write(&a, "A-new").unwrap();
        fs::write(&b, "B1").unwrap();
    });
    assert_eq!(fx.read("a.txt"), "A-new");
    assert_eq!(fx.read("b.txt"), "B1");
    assert_eq!(fx.read("c.txt"), "C1");

    // One undo reverts the whole second turn: A removed, B back to B0; C (earlier turn) untouched.
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert!(
        !fx.path("a.txt").exists(),
        "A (created this turn) is removed"
    );
    assert_eq!(fx.read("b.txt"), "B0", "B reverts to its pre-turn state");
    assert_eq!(fx.read("c.txt"), "C1", "the earlier turn's file is intact");
}

// VAL-CKPT-005: three sequential editing turns followed by three undos return the tree to its
// original pre-edit state, matching each intermediate boundary along the way.
#[test]
fn three_turns_three_undos_walk_back_to_origin() {
    let fx = fixture("multi-undo");
    let store = fx.open();
    fx.write("x.txt", "v0");
    let boundary0 = fx.read("x.txt");

    let x = fx.path("x.txt");
    turn(&store, "t1", &[x.as_path()], || {
        fs::write(&x, "v1").unwrap()
    });
    let boundary1 = fx.read("x.txt");
    turn(&store, "t2", &[x.as_path()], || {
        fs::write(&x, "v2").unwrap()
    });
    let boundary2 = fx.read("x.txt");
    turn(&store, "t3", &[x.as_path()], || {
        fs::write(&x, "v3").unwrap()
    });
    assert_eq!(fx.read("x.txt"), "v3");

    // Each undo restores the exact prior boundary, in reverse chronological order.
    store.undo().unwrap();
    assert_eq!(fx.read("x.txt"), boundary2);
    store.undo().unwrap();
    assert_eq!(fx.read("x.txt"), boundary1);
    store.undo().unwrap();
    assert_eq!(
        fx.read("x.txt"),
        boundary0,
        "back to the original pre-edit state"
    );

    // No further undo is possible.
    assert!(matches!(store.undo().unwrap(), RestoreOutcome::Nothing));
}

// VAL-CKPT-008: undo reverses both a create and a delete.
#[test]
fn undo_reverses_create_and_delete() {
    let fx = fixture("create-delete");
    let store = fx.open();
    fx.write("old.txt", "ORIGINAL-OLD\n");
    let old_bytes = fx.read("old.txt");

    // Turn A: a write that creates new.txt.
    let new = fx.path("new.txt");
    turn(&store, "create", &[new.as_path()], || {
        fs::write(&new, "freshly created\n").unwrap();
    });
    assert!(fx.path("new.txt").exists());

    // Turn B: a patch-style delete of old.txt (pre-image captured before removal).
    let old = fx.path("old.txt");
    turn(&store, "delete", &[old.as_path()], || {
        fs::remove_file(&old).unwrap();
    });
    assert!(!fx.path("old.txt").exists());

    // Undo the delete: old.txt is restored with its original bytes.
    store.undo().unwrap();
    assert!(fx.path("old.txt").exists(), "deleted file restored by undo");
    assert_eq!(
        fx.read("old.txt"),
        old_bytes,
        "restored with original bytes"
    );

    // Undo the create: new.txt is removed.
    store.undo().unwrap();
    assert!(!fx.path("new.txt").exists(), "created file removed by undo");
}

// VAL-CKPT-010: a fresh edit after an undo discards the redo branch; redo does not resurrect the
// abandoned change.
#[test]
fn new_edit_after_undo_discards_redo_branch() {
    let fx = fixture("branch-advance");
    let store = fx.open();
    fx.write("x.txt", "H0");

    let x = fx.path("x.txt");
    turn(&store, "first-edit", &[x.as_path()], || {
        fs::write(&x, "FIRST-EDIT").unwrap();
    });

    // Undo back to H0.
    store.undo().unwrap();
    assert_eq!(fx.read("x.txt"), "H0");

    // A fresh edit advances history along a new branch.
    turn(&store, "new-edit", &[x.as_path()], || {
        fs::write(&x, "NEW-EDIT").unwrap();
    });
    assert_eq!(fx.read("x.txt"), "NEW-EDIT");

    // Redo must NOT resurrect the first edit's content; the new edit's state is preserved.
    assert!(matches!(store.redo().unwrap(), RestoreOutcome::Nothing));
    assert_eq!(fx.read("x.txt"), "NEW-EDIT");
}

// A turn that records no pre-image (a read-only turn) creates no checkpoint churn.
#[test]
fn read_only_turn_records_no_snapshot() {
    let fx = fixture("read-only");
    let store = fx.open();
    fx.write("x.txt", "untouched");

    let recorder = store.begin_turn("read-only");
    // No record_pre_image calls (e.g. read/grep/glob only).
    assert!(recorder.finalize().expect("finalize").is_none());

    let history = store.history().expect("history");
    assert_eq!(
        history.total_turns, 0,
        "no snapshot recorded for a read-only turn"
    );
}

// Durability across processes is backed by the on-disk journal + object database: a store reopened
// from the same shadow directory sees the recorded history and can undo it.
#[test]
fn history_is_durable_across_reopen() {
    let fx = fixture("durable");
    {
        let store = fx.open();
        fx.write("x.txt", "H0");
        let x = fx.path("x.txt");
        turn(&store, "t1", &[x.as_path()], || {
            fs::write(&x, "H1").unwrap()
        });
    }
    // A separate store handle (as a later process would open) restores the change.
    let reopened = fx.open();
    assert!(reopened.history().unwrap().can_undo());
    reopened.undo().unwrap();
    assert_eq!(fx.read("x.txt"), "H0");
}
