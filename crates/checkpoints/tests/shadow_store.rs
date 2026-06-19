//! Hardening tests proving the checkpoint store is an INDEPENDENT shadow-git store: it never touches
//! the project's own `.git`, lives outside the project working tree (no untracked pollution),
//! detects out-of-band conflicts on undo, leaves unrelated files untouched, works in a project with
//! no `.git`, and keeps a history that survives a store re-open (the on-disk basis of cross-process
//! durability).
//!
//! Each test maps to a checkpoint validation assertion:
//! VAL-CKPT-006 (never touches the project `.git`), VAL-CKPT-007 (out-of-band conflict detection),
//! VAL-CKPT-009 (read-only churn-free), VAL-CKPT-011 (durable across re-open), VAL-CKPT-012 (works
//! without a project `.git`), VAL-CKPT-013 (store lives outside the work tree), VAL-CKPT-014 (undo
//! restores only agent-changed files).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use checkpoints::{RestoreOutcome, ShadowStore};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh workspace plus a sibling shadow git directory that lives OUTSIDE the workspace tree, so
/// the store provably produces no in-tree pollution.
struct Fixture {
    workspace: PathBuf,
    shadow_git_dir: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "cortex-shadow-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    let workspace = base.join("workspace");
    // The shadow store lives under a sibling directory, never inside the workspace tree.
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

    /// Initialize the workspace as a real git repository with `tracked.txt` committed, returning the
    /// pristine HEAD, reflog, and porcelain status so a later comparison can prove the project's git
    /// state is untouched by checkpoint activity.
    fn init_git_project(&self) -> ProjectGitState {
        self.run_git(&["init", "-q"]);
        self.write("tracked.txt", "H0\n");
        self.run_git(&["add", "tracked.txt"]);
        self.run_git(&[
            "-c",
            "user.name=tester",
            "-c",
            "user.email=tester@example.com",
            "commit",
            "-q",
            "-m",
            "initial",
        ]);
        ProjectGitState {
            head: self.git_stdout(&["rev-parse", "HEAD"]),
            reflog: self.git_stdout(&["reflog"]),
            status: self.git_stdout(&["status", "--porcelain"]),
        }
    }

    /// Run a git command in the workspace, asserting success.
    fn run_git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Run a git query in the workspace, returning trimmed stdout.
    fn git_stdout(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

/// A snapshot of the project repository's observable git state.
struct ProjectGitState {
    head: String,
    reflog: String,
    status: String,
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

// VAL-CKPT-006 + VAL-CKPT-013: a full snapshot/undo/redo cycle in a REAL git project leaves the
// project's HEAD, reflog, and porcelain status byte-identical, and the shadow store lives outside
// the project working tree (no untracked snapshot artifacts surface inside the project).
#[test]
fn shadow_store_never_touches_the_project_git() {
    let fx = fixture("project-git");
    let before = fx.init_git_project();
    assert!(before.status.is_empty(), "the project starts clean");

    // The shadow store's git directory must resolve OUTSIDE the project working tree (and its .git).
    let store = fx.open();
    assert!(
        !store.git_dir().starts_with(&fx.workspace),
        "shadow git dir {} must live outside the project working tree {}",
        store.git_dir().display(),
        fx.workspace.display()
    );

    // A turn edits the tracked file: recording the snapshot uses object-level shadow git only.
    let tracked = fx.path("tracked.txt");
    turn(&store, "edit", &[tracked.as_path()], || {
        fs::write(&tracked, "H1 agent edit\n").unwrap();
    });
    assert_eq!(
        fx.git_stdout(&["rev-parse", "HEAD"]),
        before.head,
        "recording a snapshot must not move the project HEAD"
    );
    assert_eq!(
        fx.git_stdout(&["reflog"]),
        before.reflog,
        "recording a snapshot must not write the project reflog"
    );

    // Walk the full undo/redo cycle, returning the tree to its committed state.
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("tracked.txt"), "H0\n");
    assert!(matches!(
        store.redo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("tracked.txt"), "H1 agent edit\n");
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));

    // VAL-CKPT-006: HEAD, reflog, and status are all identical to before any checkpoint activity.
    assert_eq!(
        fx.git_stdout(&["rev-parse", "HEAD"]),
        before.head,
        "HEAD is unchanged by checkpoint/undo/redo activity"
    );
    assert_eq!(
        fx.git_stdout(&["reflog"]),
        before.reflog,
        "the reflog is unchanged by checkpoint/undo/redo activity"
    );
    assert_eq!(
        fx.git_stdout(&["status", "--porcelain"]),
        before.status,
        "the porcelain status is unchanged (no shadow-store pollution)"
    );

    // VAL-CKPT-013: the project contains no untracked shadow-store artifacts, and the store path is
    // outside the work tree.
    assert!(
        fx.git_stdout(&["status", "--porcelain"]).is_empty(),
        "no untracked shadow-store files surface inside the project"
    );
    assert!(
        !store.git_dir().exists() || !store.git_dir().starts_with(&fx.workspace),
        "the shadow store never lives inside the project tree"
    );
}

// VAL-CKPT-007: if a file is modified out-of-band after the snapshot, undo detects the conflict and
// refuses rather than silently discarding the external change.
#[test]
fn undo_detects_out_of_band_conflict_and_preserves_external_changes() {
    let fx = fixture("conflict");
    let store = fx.open();
    fx.write("x.txt", "H0\n");

    let x = fx.path("x.txt");
    turn(&store, "agent-edit", &[x.as_path()], || {
        fs::write(&x, "H1 agent edit\n").unwrap();
    });

    // An external process appends to X after the agent's snapshot was finalized.
    let external = "H1 agent edit\nEXTERNAL APPEND\n";
    fs::write(&x, external).unwrap();

    // Undo must surface a conflict naming the affected path, not a blind overwrite.
    match store.undo().unwrap() {
        RestoreOutcome::Conflict { conflicts, .. } => {
            assert_eq!(conflicts, vec!["x.txt".to_owned()]);
        }
        other => panic!("expected an out-of-band conflict, got {other:?}"),
    }

    // The external append survives untouched (no silent destructive restore).
    assert_eq!(
        fx.read("x.txt"),
        external,
        "the out-of-band change must not be clobbered"
    );

    // The history cursor did not advance: the turn is still undoable once the conflict is resolved.
    assert!(
        store.history().unwrap().can_undo(),
        "a refused undo leaves the cursor in place"
    );
}

// VAL-CKPT-009: a sequence of read-only turns (no pre-images recorded) leaves the checkpoint store
// count unchanged from its initial value.
#[test]
fn read_only_turns_create_no_checkpoint_churn() {
    let fx = fixture("read-only");
    let store = fx.open();
    fx.write("x.txt", "untouched");

    let initial = store.history().expect("history").total_turns;

    // Several read-only turns: each begins and finalizes WITHOUT recording any pre-image, exactly as
    // read/glob/grep/read-only bash do.
    for label in ["read", "grep", "glob"] {
        let recorder = store.begin_turn(label);
        assert!(
            recorder.finalize().expect("finalize").is_none(),
            "a read-only turn records no snapshot"
        );
    }

    assert_eq!(
        store.history().expect("history").total_turns,
        initial,
        "the checkpoint store count is unchanged across read-only turns"
    );
    assert_eq!(fx.read("x.txt"), "untouched");
}

// VAL-CKPT-011: the history is durable on disk — a turn recorded through one store handle is undone
// by a SEPARATE handle opened from the same shadow directory (the in-process analogue of a later
// process re-opening the store), restoring the pre-edit bytes byte-identically.
#[test]
fn history_is_durable_across_a_separate_store_handle() {
    let fx = fixture("durable");
    let h0 = "H0 durable content\nsecond line\n";
    fx.write("x.txt", h0);

    // Handle A records an edit, then is dropped (modelling process A exiting).
    {
        let store_a = fx.open();
        let x = fx.path("x.txt");
        turn(&store_a, "process-a", &[x.as_path()], || {
            fs::write(&x, "H1 changed by A\n").unwrap();
        });
        assert_eq!(fx.read("x.txt"), "H1 changed by A\n");
    }

    // Handle B (a fresh open, as a later process would do) sees the durable history and undoes it.
    let store_b = fx.open();
    assert!(
        store_b.history().unwrap().can_undo(),
        "the recorded history survives the original handle being dropped"
    );
    assert!(matches!(
        store_b.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(
        fx.read("x.txt"),
        h0,
        "a separate handle restores the pre-edit bytes byte-identically"
    );
}

// VAL-CKPT-012: checkpointing works in a directory that is NOT a git repository, and the working
// directory still contains no `.git` afterwards (the shadow store is fully independent).
#[test]
fn checkpointing_works_without_a_project_git() {
    let fx = fixture("no-git");
    assert!(
        !fx.workspace.join(".git").exists(),
        "the scratch workspace is not a git repository"
    );

    let store = fx.open();
    let h0 = "H0 no-git\n";
    fx.write("x.txt", h0);

    let x = fx.path("x.txt");
    turn(&store, "edit", &[x.as_path()], || {
        fs::write(&x, "H1 no-git\n").unwrap();
    });
    assert_eq!(fx.read("x.txt"), "H1 no-git\n");

    // Undo succeeds with no missing-repository error and restores the prior bytes.
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(
        fx.read("x.txt"),
        h0,
        "undo restores H0 with no project .git"
    );

    // The working tree still contains no `.git`: the store never created one in the workspace.
    assert!(
        !fx.workspace.join(".git").exists(),
        "no .git is created inside the working tree"
    );
}

// VAL-CKPT-014: undo reverts exactly the files the turn changed, leaving an unrelated sibling file
// (never touched by the agent) byte-identical.
#[test]
fn undo_restores_only_agent_changed_files() {
    let fx = fixture("unrelated");
    let store = fx.open();
    let hu = "UNRELATED CONTENT\nthat the agent never touches\n";
    let h0 = "A H0\n";
    fx.write("U.txt", hu);
    fx.write("A.txt", h0);

    // A turn edits only A.txt.
    let a = fx.path("A.txt");
    turn(&store, "edit-A", &[a.as_path()], || {
        fs::write(&a, "A H1\n").unwrap();
    });
    assert_eq!(fx.read("A.txt"), "A H1\n");
    assert_eq!(
        fx.read("U.txt"),
        hu,
        "the unrelated file is not in the turn"
    );

    // Undo restores A.txt to H0 while leaving U.txt byte-identical.
    assert!(matches!(
        store.undo().unwrap(),
        RestoreOutcome::Restored { .. }
    ));
    assert_eq!(fx.read("A.txt"), h0, "A.txt is restored to H0");
    assert_eq!(
        fx.read("U.txt"),
        hu,
        "the untouched sibling U.txt remains byte-identical"
    );
}
