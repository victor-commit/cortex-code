//! Black-box tests for the headless snapshot surface (`cortex snapshot diff` / `revert` / `prune`).
//!
//! These drive the real `cortex` binary against a real git-backed shadow store WITHOUT an OpenRouter
//! key. As in `checkpoint_cli.rs`, recording a turn normally happens inside an LLM `cortex run`; to
//! exercise the snapshot surface deterministically and offline, the test prepares the SAME shadow
//! store the binary resolves (discovered after a first `cortex undo` creates it) and records turns
//! through the public `checkpoints` API, then invokes `cortex snapshot ...` and inspects stdout and
//! the filesystem.
//!
//! Covers VAL-GAP-DATA-005 (diff + diffFull retrievable), VAL-GAP-DATA-006 (per-patch revert
//! independent of turn-level undo), and VAL-GAP-DATA-007 (prune keeps recent undo/redo) through the
//! real CLI.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

use checkpoints::ShadowStore;

fn scratch(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-snap-cli-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

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

// VAL-GAP-DATA-005: `cortex snapshot diff` (summary) and `--full` (unified diff) list every changed
// file, with full output in `git diff` style.
#[test]
fn snapshot_diff_summary_and_full_list_changed_files() {
    let h = Harness::new("diff");
    h.write("keep.txt", "alpha\nbeta\n");
    h.write("gone.txt", "remove me\n");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let keep = h.path("keep.txt");
    let gone = h.path("gone.txt");
    let added = h.path("added.txt");
    record_turn(
        &store,
        "sess",
        &[keep.as_path(), gone.as_path(), added.as_path()],
        || {
            std::fs::write(&keep, "alpha\nBETA\n").unwrap();
            std::fs::remove_file(&gone).unwrap();
            std::fs::write(&added, "new file\n").unwrap();
        },
    );

    // Summary as strict JSON: every changed file with status + counts, and no diff text.
    let summary = h.cortex(&["snapshot", "diff", "--format", "json"]);
    assert_success(&summary);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&summary.stdout).trim()).expect("strict JSON");
    let files = value["files"].as_array().expect("files array");
    let names: Vec<&str> = files.iter().filter_map(|f| f["path"].as_str()).collect();
    assert!(names.contains(&"keep.txt"), "files: {names:?}");
    assert!(names.contains(&"gone.txt"), "files: {names:?}");
    assert!(names.contains(&"added.txt"), "files: {names:?}");
    let keep_entry = files.iter().find(|f| f["path"] == "keep.txt").unwrap();
    assert_eq!(keep_entry["status"], "modified");
    assert_eq!(keep_entry["additions"], 1);
    assert_eq!(keep_entry["deletions"], 1);
    assert!(keep_entry.get("diff").is_none(), "summary omits diff text");

    // Full diff: git-style unified text referencing the real path.
    let full = h.cortex(&["snapshot", "diff", "--full", "--format", "json"]);
    assert_success(&full);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&full.stdout).trim()).expect("strict JSON");
    let keep_entry = value["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "keep.txt")
        .unwrap();
    let diff = keep_entry["diff"].as_str().expect("full diff text");
    assert!(diff.contains("diff --git a/keep.txt b/keep.txt"), "{diff}");
    assert!(diff.contains("-beta") && diff.contains("+BETA"), "{diff}");

    // Human text output also lists the files.
    let text = h.cortex(&["snapshot", "diff"]);
    assert_success(&text);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("keep.txt"), "text: {stdout}");
    assert!(stdout.contains("added.txt"), "text: {stdout}");
}

// VAL-GAP-DATA-006: `cortex snapshot revert --path` reverts one file (a patch) without touching the
// other file changed in the same turn, and records the action (a later `cortex undo` re-applies it).
#[test]
fn snapshot_revert_path_is_independent_of_other_files() {
    let h = Harness::new("revert");
    h.write("a.txt", "A0\n");
    h.write("b.txt", "B0\n");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let a = h.path("a.txt");
    let b = h.path("b.txt");
    record_turn(&store, "sess", &[a.as_path(), b.as_path()], || {
        std::fs::write(&a, "A1\n").unwrap();
        std::fs::write(&b, "B1\n").unwrap();
    });

    let revert = h.cortex(&["snapshot", "revert", "--path", "a.txt", "--format", "json"]);
    assert_success(&revert);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&revert.stdout).trim()).expect("strict JSON");
    assert_eq!(value["status"], "restored");
    assert!(
        value["paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "a.txt"),
        "paths: {value:?}"
    );

    // a.txt rolled back; b.txt left at its turn change.
    assert_eq!(h.read("a.txt"), "A0\n");
    assert_eq!(h.read("b.txt"), "B1\n");

    // The revert was recorded as a fresh undoable snapshot: `cortex undo` re-applies the change.
    let undo = h.cortex(&["undo"]);
    assert_success(&undo);
    assert_eq!(h.read("a.txt"), "A1\n", "undo of the revert re-applies A1");
    assert_eq!(h.read("b.txt"), "B1\n");
}

// VAL-GAP-DATA-006: `cortex snapshot revert --hunk` reverts only the selected hunk of a file.
#[test]
fn snapshot_revert_hunk_targets_one_hunk() {
    let h = Harness::new("hunk");
    let pre = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nl15\nl16\nl17\nl18\nl19\nl20\n";
    let post = "l1\nl2\nCHANGED-A\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nCHANGED-B\nl16\nl17\nl18\nl19\nl20\n";
    h.write("multi.txt", pre);

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let multi = h.path("multi.txt");
    record_turn(&store, "sess", &[multi.as_path()], || {
        std::fs::write(&multi, post).unwrap();
    });

    let revert = h.cortex(&["snapshot", "revert", "--path", "multi.txt", "--hunk", "0"]);
    assert_success(&revert);

    let result = h.read("multi.txt");
    assert!(result.contains("l3\n"), "hunk 0 rolled back: {result}");
    assert!(!result.contains("CHANGED-A"), "hunk 0 reverted: {result}");
    assert!(result.contains("CHANGED-B"), "hunk 1 preserved: {result}");
}

// VAL-GAP-DATA-006: a conflicting revert (file changed out-of-band) exits non-zero and preserves the
// external change.
#[test]
fn snapshot_revert_conflict_exits_nonzero() {
    let h = Harness::new("revert-conflict");
    h.write("x.txt", "H0\n");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let x = h.path("x.txt");
    record_turn(&store, "sess", &[x.as_path()], || {
        std::fs::write(&x, "H1\n").unwrap();
    });
    let external = "H1\nEXTERNAL\n";
    std::fs::write(&x, external).unwrap();

    let revert = h.cortex(&["snapshot", "revert", "--path", "x.txt", "--format", "json"]);
    assert!(
        !revert.status.success(),
        "an out-of-band conflict must exit non-zero"
    );
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&revert.stdout).trim()).expect("strict JSON");
    assert_eq!(value["status"], "conflict");
    assert_eq!(h.read("x.txt"), external, "external change preserved");
}

// VAL-GAP-DATA-007: `cortex snapshot prune` drops old snapshots yet keeps the recent undo turns
// byte-exact.
#[test]
fn snapshot_prune_keeps_recent_undo() {
    let h = Harness::new("prune");
    h.write("x.txt", "v0\n");

    assert_success(&h.cortex(&["undo"]));
    let store = h.open_store();
    let x = h.path("x.txt");
    for version in 1..=5 {
        let next = format!("v{version}\n");
        record_turn(&store, "sess", &[x.as_path()], || {
            std::fs::write(&x, &next).unwrap();
        });
    }
    assert_eq!(h.read("x.txt"), "v5\n");

    let prune = h.cortex(&["snapshot", "prune", "--keep", "2", "--format", "json"]);
    assert_success(&prune);
    let value: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&prune.stdout).trim()).expect("strict JSON");
    assert_eq!(value["pruned_turns"], 3);
    assert_eq!(value["remaining_turns"], 2);

    // The two most recent turns still undo byte-exact through a separate `cortex undo` process.
    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), "v4\n");
    assert_success(&h.cortex(&["undo"]));
    assert_eq!(h.read("x.txt"), "v3\n");
    // History beyond the retention window is gone.
    let extra = h.cortex(&["undo"]);
    assert_success(&extra);
    assert!(String::from_utf8_lossy(&extra.stdout).contains("nothing to undo"));
}
