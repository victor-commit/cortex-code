//! Black-box checks for the `cortex session` persistence path (VAL-SKEL-037..042, 068).
//!
//! Each test builds an isolated scratch tree with its own HOME and database location, runs the
//! real binary with a cleared environment, and inspects both the CLI output and the on-disk
//! SQLite file. Durability is proven by creating data in one process and reading it back in a
//! separate, later process.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// A self-cleaning scratch tree: `<root>/home` is HOME.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-session-cli-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// The default DB path derived from this scratch HOME (no override).
    fn default_db(&self) -> PathBuf {
        self.home().join(".local/share/cortex/cortex.db")
    }

    fn command(&self, env: &[(&str, &str)], args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        for (key, value) in env {
            command.env(key, value);
        }
        command.arg("session");
        command.args(args);
        command
    }

    /// Run a `cortex session` invocation to completion.
    fn run(&self, env: &[(&str, &str)], args: &[&str]) -> Output {
        collect(spawn(self.command(env, args)))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Spawn a command with piped, non-inherited streams.
fn spawn(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.spawn().expect("spawn cortex binary")
}

/// Drain and wait for a spawned child with a hard timeout.
fn collect(mut child: Child) -> Output {
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex session did not terminate within {RUN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn stdout_str(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf8 stdout")
}

fn stderr_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Open a DB file read-only and return its user table names.
fn table_names(db_path: &PathBuf) -> Vec<String> {
    let conn = rusqlite::Connection::open(db_path).expect("open db file");
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare");
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    names
}

fn integrity_ok(db_path: &PathBuf) -> bool {
    let conn = rusqlite::Connection::open(db_path).expect("open db file");
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity check");
    result == "ok"
}

fn migration_row_count(db_path: &PathBuf) -> i64 {
    let conn = rusqlite::Connection::open(db_path).expect("open db file");
    conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
        .expect("count migrations")
}

// VAL-SKEL-037: first run against a fresh path creates a valid SQLite DB with expected tables.
#[test]
fn fresh_run_creates_valid_db_with_expected_tables() {
    let scratch = Scratch::new();
    let db = scratch.path("custom/cortex.db");
    assert!(!db.exists(), "precondition: DB path is fresh");

    let out = scratch.run(&[("CORTEX_DB_PATH", db.to_str().unwrap())], &["new"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(db.exists(), "DB file is created on first use");

    let tables = table_names(&db);
    for expected in ["sessions", "messages", "schema_migrations"] {
        assert!(tables.contains(&expected.to_owned()), "tables: {tables:?}");
    }
    assert!(integrity_ok(&db), "fresh DB passes integrity_check");

    // The new session id is printed on stdout with nothing on stderr.
    assert!(stdout_str(&out).trim().starts_with("ses_"));
    assert!(out.stderr.is_empty(), "clean run has empty stderr");
}

// VAL-SKEL-038: a session created by invocation A is returned by a later invocation B.
#[test]
fn session_is_retrievable_after_process_exit() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    let env = [("CORTEX_DB_PATH", db.to_str().unwrap())];

    let created = scratch.run(&env, &["new", "--format", "json"]);
    assert_eq!(created.status.code(), Some(0));
    let created_json: Value = serde_json::from_slice(&created.stdout).expect("json");
    let id = created_json["id"].as_str().expect("id").to_owned();

    // A new process lists/shows the session created by the prior, fully-exited process.
    let listed = scratch.run(&env, &["list", "--format", "json"]);
    assert_eq!(listed.status.code(), Some(0));
    let listed_json: Value = serde_json::from_slice(&listed.stdout).expect("json");
    let ids: Vec<&str> = listed_json["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&id.as_str()), "session {id} survives restart");

    let shown = scratch.run(&env, &["show", &id, "--format", "json"]);
    assert_eq!(shown.status.code(), Some(0));
    let shown_json: Value = serde_json::from_slice(&shown.stdout).expect("json");
    assert_eq!(shown_json["session"]["id"], id);
}

// VAL-SKEL-039: a stored message (with content) is retrievable in a later invocation.
#[test]
fn message_content_is_retrievable_after_process_exit() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    let env = [("CORTEX_DB_PATH", db.to_str().unwrap())];

    let created = scratch.run(&env, &["new", "--format", "json"]);
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let stored = scratch.run(
        &env,
        &["message", &id, "remember marmalade", "--format", "json"],
    );
    assert_eq!(
        stored.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&stored)
    );

    // A separate later process reads the message content back.
    let shown = scratch.run(&env, &["show", &id, "--format", "json"]);
    assert_eq!(shown.status.code(), Some(0));
    let shown_json: Value = serde_json::from_slice(&shown.stdout).expect("json");
    let messages = shown_json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "remember marmalade");
    assert_eq!(messages[0]["role"], "user");
}

// VAL-SKEL-040: repeated runs against the migrated DB stay green and keep prior rows.
#[test]
fn repeated_runs_are_idempotent_and_preserve_rows() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    let env = [("CORTEX_DB_PATH", db.to_str().unwrap())];

    for _ in 0..4 {
        let out = scratch.run(&env, &["new"]);
        assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    }

    assert_eq!(
        migration_row_count(&db),
        db::latest_version(),
        "each migration applied exactly once"
    );
    assert!(integrity_ok(&db));

    let listed = scratch.run(&env, &["list", "--format", "json"]);
    let listed_json: Value = serde_json::from_slice(&listed.stdout).expect("json");
    assert_eq!(
        listed_json["sessions"].as_array().unwrap().len(),
        4,
        "every created session survives repeated runs"
    );
}

// VAL-SKEL-041: the override creates the DB only at the new path; the default is untouched.
#[test]
fn override_creates_db_only_at_override_path() {
    let scratch = Scratch::new();
    let override_db = scratch.path("relocated/cortex.db");

    let out = scratch.run(
        &[("CORTEX_DB_PATH", override_db.to_str().unwrap())],
        &["new"],
    );
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(override_db.exists(), "DB created at the override path");
    assert!(
        !scratch.default_db().exists(),
        "the default path is left untouched when an override is used"
    );
}

// VAL-SKEL-041 (default): with no override the DB lands at the documented default location.
#[test]
fn default_path_is_used_without_override() {
    let scratch = Scratch::new();
    let out = scratch.run(&[], &["new"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        scratch.default_db().exists(),
        "DB created at the default data-dir location"
    );
}

// VAL-SKEL-041 (data-dir override): CORTEX_DATA_DIR relocates the data directory.
#[test]
fn data_dir_override_relocates_the_db() {
    let scratch = Scratch::new();
    let data_dir = scratch.path("alt-data");
    let out = scratch.run(&[("CORTEX_DATA_DIR", data_dir.to_str().unwrap())], &["new"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(data_dir.join("cortex.db").exists());
    assert!(!scratch.default_db().exists(), "default left untouched");
}

// VAL-SKEL-042 (junk file): a non-SQLite file yields a clear stderr error and non-zero exit.
#[test]
fn junk_db_file_yields_clear_error_no_panic() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    fs::write(&db, b"definitely not a sqlite database\n".repeat(8)).expect("write junk");

    let out = scratch.run(&[("CORTEX_DB_PATH", db.to_str().unwrap())], &["new"]);
    assert_ne!(out.status.code(), Some(0), "junk DB must fail");
    assert!(out.stdout.is_empty(), "stdout stays empty on error");
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("not a valid SQLite database"),
        "clear DB diagnostic, got: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "no panic backtrace: {stderr}");
}

// VAL-SKEL-042 (unwritable path): an uncreatable path yields a clear error, non-zero exit.
#[test]
fn unwritable_db_path_yields_clear_error_no_panic() {
    let scratch = Scratch::new();
    let blocker = scratch.path("a-file");
    fs::write(&blocker, b"i am a file").expect("write blocker");
    let db = blocker.join("cortex.db"); // parent is a regular file

    let out = scratch.run(&[("CORTEX_DB_PATH", db.to_str().unwrap())], &["new"]);
    assert_ne!(out.status.code(), Some(0), "unwritable path must fail");
    assert!(out.stdout.is_empty(), "stdout stays empty on error");
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("cannot open database"),
        "clear DB diagnostic, got: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "no panic backtrace: {stderr}");
}

// Showing a non-existent session is a clear error, not a panic.
#[test]
fn show_unknown_session_errors_cleanly() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    let out = scratch.run(
        &[("CORTEX_DB_PATH", db.to_str().unwrap())],
        &["show", "ses_nope"],
    );
    assert_ne!(out.status.code(), Some(0));
    let stderr = stderr_str(&out);
    assert!(stderr.contains("no such session"), "got: {stderr}");
    assert!(!stderr.contains("panicked"));
}

// VAL-SKEL-068: two simultaneous first-use processes on one fresh path each create/migrate the
// DB without corruption, double-applied migrations, or a panic.
#[test]
fn concurrent_first_use_is_safe() {
    let scratch = Scratch::new();
    let db = scratch.path("cortex.db");
    assert!(!db.exists(), "precondition: fresh path");
    let db_str = db.to_str().unwrap();

    // Spawn both processes before draining either, so their first-use migrations race.
    let a = spawn(scratch.command(&[("CORTEX_DB_PATH", db_str)], &["new"]));
    let b = spawn(scratch.command(&[("CORTEX_DB_PATH", db_str)], &["new"]));
    let out_a = collect(a);
    let out_b = collect(b);

    for (label, out) in [("A", &out_a), ("B", &out_b)] {
        assert_eq!(
            out.status.code(),
            Some(0),
            "process {label} must exit 0; stderr: {}",
            stderr_str(out)
        );
        assert!(
            !stderr_str(out).contains("panicked"),
            "process {label} must not panic: {}",
            stderr_str(out)
        );
    }

    // After both exit: a single valid DB at the expected version with intact schema.
    assert!(integrity_ok(&db), "DB passes integrity_check");
    assert_eq!(
        migration_row_count(&db),
        db::latest_version(),
        "each migration applied exactly once despite the race"
    );
    let tables = table_names(&db);
    for expected in ["sessions", "messages", "schema_migrations"] {
        assert!(tables.contains(&expected.to_owned()), "tables: {tables:?}");
    }

    let listed = scratch.run(&[("CORTEX_DB_PATH", db_str)], &["list", "--format", "json"]);
    let listed_json: Value = serde_json::from_slice(&listed.stdout).expect("json");
    assert_eq!(
        listed_json["sessions"].as_array().unwrap().len(),
        2,
        "both concurrent sessions are present, none lost"
    );
}
