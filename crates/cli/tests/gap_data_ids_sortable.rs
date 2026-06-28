//! VAL-GAP-DATA-008: generated IDs are time-sortable and prefix-typed where documented.
//!
//! High-volume creation through the `cortex session` CLI surface mints session, message, and
//! (persisted) message-part ids on the shared store. These black-box checks assert each id carries
//! its documented prefix and that a plain lexicographic sort of the ids reproduces their creation
//! order within the run.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("cortex-ids-{label}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn db_path(&self) -> PathBuf {
        self.root.join("cortex.db")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        command.env("CORTEX_DB_PATH", self.db_path());
        command.env("CORTEX_NO_NET", "1");
        command.args(args);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = self.command(args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let child: Child = command.spawn().expect("spawn cortex binary");
        let output = child.wait_with_output().expect("collect output");
        assert!(
            output.status.success(),
            "command {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn parse_stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout was not JSON ({err}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_sorts_in_creation_order(ids: &[String], kind: &str) {
    assert!(
        ids.len() >= 50,
        "expected a high-volume {kind} fixture, got {}",
        ids.len()
    );
    let mut sorted = ids.to_vec();
    sorted.sort();
    assert_eq!(
        ids, &sorted,
        "lexicographic sort of {kind} ids must equal their creation order"
    );
    let mut unique = ids.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "{kind} ids must be unique");
}

fn assert_prefixed_hex(id: &str, prefix: &str) {
    let body = id
        .strip_prefix(&format!("{prefix}_"))
        .unwrap_or_else(|| panic!("id `{id}` must start with `{prefix}_`"));
    assert_eq!(body.len(), 32, "id `{id}` must carry a 32-hex body");
    assert!(
        body.chars().all(|c| c.is_ascii_hexdigit()),
        "id `{id}` body must be hex"
    );
}

#[test]
fn high_volume_session_ids_are_prefixed_and_sort_by_creation_order() {
    let scratch = Scratch::new("sessions");
    let mut ids = Vec::new();
    for index in 0..120 {
        let created = scratch.run(&[
            "session",
            "new",
            "--title",
            &format!("session-{index:04}"),
            "--format",
            "json",
        ]);
        let id = parse_stdout_json(&created)["id"]
            .as_str()
            .expect("session id")
            .to_owned();
        assert_prefixed_hex(&id, "ses");
        ids.push(id);
    }
    assert_sorts_in_creation_order(&ids, "session");
}

#[test]
fn high_volume_message_and_part_ids_are_prefixed_and_sort_by_creation_order() {
    let scratch = Scratch::new("messages");
    let created = scratch.run(&["session", "new", "--title", "msg-host", "--format", "json"]);
    let session_id = parse_stdout_json(&created)["id"]
        .as_str()
        .expect("session id")
        .to_owned();

    let mut message_ids = Vec::new();
    let mut part_ids = Vec::new();
    for index in 0..120 {
        let stored = scratch.run(&[
            "session",
            "message",
            &session_id,
            &format!("content-{index:04}"),
            "--role",
            "user",
            "--format",
            "json",
        ]);
        let message = parse_stdout_json(&stored);
        let id = message["id"].as_str().expect("message id").to_owned();
        assert_prefixed_hex(&id, "msg");
        for part in message["parts"].as_array().into_iter().flatten() {
            let part_id = part["id"].as_str().expect("part id").to_owned();
            assert!(
                part_id.starts_with(&format!("{id}_p")),
                "persisted part id `{part_id}` must derive from its message id `{id}`"
            );
            part_ids.push(part_id);
        }
        message_ids.push(id);
    }

    assert_sorts_in_creation_order(&message_ids, "message");
    assert_sorts_in_creation_order(&part_ids, "part");
}
