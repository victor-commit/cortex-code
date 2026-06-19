//! Black-box tests for the headless checkpoint surface (`cortex undo` / `cortex redo`).
//!
//! These drive the real `cortex` binary's undo/redo path against a real git-backed shadow store,
//! WITHOUT needing an OpenRouter key. Recording a turn normally happens inside `cortex run` (an LLM
//! turn); to exercise the undo/redo surface deterministically and offline, the test prepares the
//! same shadow store the binary resolves and records turns through the public `checkpoints` API,
//! then invokes `cortex undo`/`cortex redo` and inspects the filesystem.
//!
//! The store location is NOT recomputed in the test: a first `cortex undo` (with empty history)
//! creates the shadow directory at the binary's resolved path, which the test then discovers and
//! records into — so the test and the binary provably address the same store.
//!
//! Covers VAL-CKPT-002/003 (byte-exact undo + redo), VAL-CKPT-005 (multi-undo walk-back),
//! VAL-CKPT-008 (create/delete reversal), and VAL-CKPT-010 (a fresh edit after undo discards the
//! redo branch) through the real CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use checkpoints::ShadowStore;

/// A hermetic temp directory unique to this test process.
fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-ckpt-cli-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// One isolated workspace + checkpoint base, plus helpers to run the binary and the store.
struct Harness {
    workspace: PathBuf,
    checkpoint_base: PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let base = scratch(tag);
        let workspace = base.join("workspace");
        let checkpoint_base = base.join("ckpt-base");
        std::fs::create_dir_all(&workspace).expect("create workspace");
        std::fs::create_dir_all(&checkpoint_base).expect("create checkpoint base");
        Self {
            workspace: workspace.canonicalize().expect("canonicalize workspace"),
            checkpoint_base,
        }
    }

    /// Run `cortex <args>` from the workspace with the checkpoint store base pointed at our temp dir
    /// and the environment cleared of anything that could redirect the store.
    fn cortex(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .current_dir(&self.workspace)
            .env("CORTEX_CHECKPOINT_DIR", &self.checkpoint_base)
            .env("HOME", &self.checkpoint_base)
            .env_remove("XDG_DATA_HOME")
            .env_remove("CORTEX_DATA_DIR");
        command.output().expect("run cortex")
    }

    /// Open the shadow store the binary uses, discovering its git directory (created on the first
    /// `cortex undo`) so the test never recomputes the binary's workspace-key algorithm.
    fn open_store(&self) -> ShadowStore {
        let git_dir = self.discover_git_dir();
        ShadowStore::open(&self.workspace, &git_dir).expect("open shadow store")
    }

    /// The single child directory of the checkpoint base is the workspace's shadow git directory.
    fn discover_git_dir(&self) -> PathBuf {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.checkpoint_base)
            .expect("read checkpoint base")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "expected exactly one shadow store dir, found {entries:?}"
        );
        entries.remove(0)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.workspace.join(rel)
    }

    fn write(&self, rel: &str, content: &str) {
        std::fs::write(self.path(rel), content).expect("write file");
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).expect("read file")
    }
}

/// Record one turn into `store`: snapshot `targets`' pre-images, run `mutate`, finalize.
fn record_turn(store: &ShadowStore, label: &str, targets: &[&Path], mutate: impl FnOnce()) {
    let recorder = store.begin_turn(label);
    for target in targets {
        recorder.record_pre_image(target);
    }
    mutate();
    recorder.finalize().expect("finalize turn");
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// VAL-CKPT-002 / VAL-CKPT-003: `cortex undo` restores byte-for-byte; `cortex redo` re-applies.
#[test]
fn cli_undo_and_redo_round_trip_bytes() {
    let h = Harness::new("round-trip");
    h.write("x.txt", "H0 original\n");
    let h0 = h.read("x.txt");

    // First undo creates the shadow store (empty history): a clean "nothing to undo".
    let first = h.cortex(&["undo"]);
    assert_success(&first);
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("nothing to undo"),
        "stdout: {}",
        String::from_utf8_lossy(&first.stdout)
    );

    // Record an edit turn through the same store the binary resolves.
    let store = h.open_store();
    let x = h.path("x.txt");
    record_turn(&store, "edit", &[x.as_path()], || {
        std::fs::write(&x, "H1 changed\n").unwrap();
    });
    let h1 = h.read("x.txt");
    assert_ne!(h0, h1);

    // `cortex undo` restores H0 exactly.
    let undo = h.cortex(&["undo"]);
    assert_success(&undo);
    assert_eq!(h.read("x.txt"), h0, "undo restores byte-identical H0");

    // `cortex redo` re-applies H1 exactly.
    let redo = h.cortex(&["redo"]);
    assert_success(&redo);
    assert_eq!(h.read("x.txt"), h1, "redo restores H1");
}

// VAL-CKPT-005: three editing turns, three `cortex undo`s, back to the original state.
#[test]
fn cli_multiple_undos_walk_back_to_origin() {
    let h = Harness::new("walk-back");
    h.write("x.txt", "v0");
    let v0 = h.read("x.txt");

    // Create the store, then record three sequential turns.
    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let x = h.path("x.txt");
    record_turn(&store, "t1", &[x.as_path()], || {
        std::fs::write(&x, "v1").unwrap()
    });
    record_turn(&store, "t2", &[x.as_path()], || {
        std::fs::write(&x, "v2").unwrap()
    });
    record_turn(&store, "t3", &[x.as_path()], || {
        std::fs::write(&x, "v3").unwrap()
    });
    assert_eq!(h.read("x.txt"), "v3");

    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), "v2");
    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), "v1");
    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), v0, "three undos return to the original");

    // A fourth undo is a clean no-op.
    let extra = h.cortex(&["undo"]);
    assert_success(&extra);
    assert!(String::from_utf8_lossy(&extra.stdout).contains("nothing to undo"));
}

// VAL-CKPT-008: `cortex undo` reverses a create (file removed) and a delete (file restored).
#[test]
fn cli_undo_reverses_create_and_delete() {
    let h = Harness::new("create-delete");
    h.write("old.txt", "ORIGINAL-OLD\n");
    let old_bytes = h.read("old.txt");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();

    // Turn 1: create new.txt.
    let new = h.path("new.txt");
    record_turn(&store, "create", &[new.as_path()], || {
        std::fs::write(&new, "fresh\n").unwrap();
    });
    // Turn 2: delete old.txt.
    let old = h.path("old.txt");
    record_turn(&store, "delete", &[old.as_path()], || {
        std::fs::remove_file(&old).unwrap();
    });
    assert!(!h.path("old.txt").exists());

    // Undo the delete: old.txt restored with its original bytes.
    assert_success(&h.cortex(&["undo"]));
    assert!(h.path("old.txt").exists());
    assert_eq!(h.read("old.txt"), old_bytes);

    // Undo the create: new.txt removed.
    assert_success(&h.cortex(&["undo"]));
    assert!(!h.path("new.txt").exists());
}

// VAL-CKPT-010: a fresh edit after an undo discards the redo branch; `cortex redo` is a no-op and
// the new edit's state is preserved.
#[test]
fn cli_new_edit_after_undo_discards_redo_branch() {
    let h = Harness::new("branch");
    h.write("x.txt", "H0");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let x = h.path("x.txt");

    record_turn(&store, "first", &[x.as_path()], || {
        std::fs::write(&x, "FIRST").unwrap();
    });
    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), "H0");

    // A fresh edit advances history along a new branch.
    record_turn(&store, "second", &[x.as_path()], || {
        std::fs::write(&x, "SECOND").unwrap();
    });

    // Redo must not resurrect the first edit; the new edit is preserved.
    let redo = h.cortex(&["redo"]);
    assert_success(&redo);
    assert!(String::from_utf8_lossy(&redo.stdout).contains("nothing to redo"));
    assert_eq!(h.read("x.txt"), "SECOND");
}

// The JSON output is a single strict, parseable document carrying the action, status, and paths.
#[test]
fn cli_undo_json_output_is_strict() {
    let h = Harness::new("json");
    h.write("x.txt", "H0");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let x = h.path("x.txt");
    record_turn(&store, "edit", &[x.as_path()], || {
        std::fs::write(&x, "H1").unwrap();
    });

    let undo = h.cortex(&["undo", "--format", "json"]);
    assert_success(&undo);
    let stdout = String::from_utf8_lossy(&undo.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("strict JSON");
    assert_eq!(value["action"], "undo");
    assert_eq!(value["status"], "restored");
    let paths = value["paths"].as_array().expect("paths array");
    assert!(paths.iter().any(|p| p == "x.txt"), "paths: {paths:?}");
    assert_eq!(h.read("x.txt"), "H0");
}
