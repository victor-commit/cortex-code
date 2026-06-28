//! Black-box coverage for the storage gap `gap-storage-typed-parts-project-account`.
//!
//! These tests exercise the durable multi-directory project model and the durable account records
//! through the shipped `cortex` binary only (no library access), proving:
//!
//! - VAL-GAP-DATA-003: a project spans multiple directories without cwd-hash collisions, the
//!   project list shows both directories, search spans both, and a session keeps its project
//!   identity across a process restart.
//! - VAL-GAP-DATA-002: account state persists durably across a restart with restrictive file
//!   permissions on the secret sink and no raw secret in any output or the database file.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-storage-gap-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        let data_dir = root.join("data");
        fs::create_dir_all(&home).expect("create HOME");
        fs::create_dir_all(&data_dir).expect("create data dir");
        Self {
            root,
            home,
            data_dir,
        }
    }

    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        fs::create_dir_all(&path).expect("create directory");
        path
    }

    fn db_path(&self) -> PathBuf {
        self.data_dir.join("cortex.db")
    }

    fn auth_file(&self) -> PathBuf {
        self.data_dir.join("auth.json")
    }

    fn command_in(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("CORTEX_DATA_DIR", &self.data_dir)
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_AUTH_FILE");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(mut command: Command, stdin_text: Option<&str>) -> Output {
    command.stdin(if stdin_text.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child: Child = command.spawn().expect("spawn cortex");
    if let Some(text) = stdin_text {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(text.as_bytes()).expect("write stdin");
        stdin.flush().expect("flush stdin");
    }
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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
            panic!("cortex did not terminate within {RUN_TIMEOUT:?}");
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
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_str(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn no_panic(stderr: &str) {
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("RUST_BACKTRACE"),
        "must not panic: {stderr}"
    );
}

fn fake_key(tag: &str) -> String {
    format!("sk-{}-{}{}", "or-v1", tag, "abcd1234".repeat(6))
}

// VAL-GAP-DATA-003: a project spans two directories, the list shows both, search spans both, and
// the project identity is the same regardless of which directory it is queried from (no cwd-hash
// collision). Every command is a fresh process, so durability across restarts is inherent.
#[test]
fn project_spans_multiple_directories_and_search_covers_both() {
    let sandbox = Sandbox::new("multi-dir");
    let dir_a = sandbox.mkdir("alpha");
    let dir_b = sandbox.mkdir("beta");
    let token_a = "TOKEN_ALPHA_4242";
    let token_b = "TOKEN_BETA_9090";
    fs::write(dir_a.join("a.txt"), format!("{token_a}\n")).expect("write a fixture");
    fs::write(dir_b.join("b.txt"), format!("{token_b}\n")).expect("write b fixture");

    // First use in directory A creates the project anchored there.
    let current = run(sandbox.command_in(&dir_a, &["project", "current"]), None);
    assert_eq!(
        current.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&current)
    );
    no_panic(&stderr_str(&current));
    assert!(
        stdout_str(&current).contains("alpha"),
        "project current should report directory A: {}",
        stdout_str(&current)
    );

    // Widen the project to a second directory.
    let add = run(
        sandbox.command_in(&dir_a, &["project", "add", dir_b.to_str().unwrap()]),
        None,
    );
    assert_eq!(add.status.code(), Some(0), "stderr: {}", stderr_str(&add));

    // The project list shows BOTH directories (fresh process == post-restart).
    let list = run(
        sandbox.command_in(&dir_a, &["project", "list", "--format", "json"]),
        None,
    );
    assert_eq!(list.status.code(), Some(0), "stderr: {}", stderr_str(&list));
    let doc: Value = serde_json::from_slice(&list.stdout).expect("strict project list JSON");
    let projects = doc["projects"].as_array().expect("projects array");
    assert_eq!(projects.len(), 1, "exactly one project: {doc:?}");
    let directories: Vec<String> = projects[0]["directories"]
        .as_array()
        .expect("directories array")
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        directories.iter().any(|dir| dir.contains("alpha"))
            && directories.iter().any(|dir| dir.contains("beta")),
        "project list should show both directories: {directories:?}"
    );
    let project_id_from_a = projects[0]["id"].as_str().expect("project id").to_owned();

    // Querying from directory B resolves to the SAME project (no cwd-hash collision).
    let current_from_b = run(
        sandbox.command_in(&dir_b, &["project", "current", "--format", "json"]),
        None,
    );
    assert_eq!(
        current_from_b.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&current_from_b)
    );
    let doc_b: Value =
        serde_json::from_slice(&current_from_b.stdout).expect("strict project current JSON");
    assert_eq!(
        doc_b["id"].as_str(),
        Some(project_id_from_a.as_str()),
        "directory B must resolve to the same project as directory A: {doc_b:?}"
    );

    // Search from A spans BOTH directories: a token that only exists in B is found.
    let search = run(sandbox.command_in(&dir_a, &["search", token_b]), None);
    assert_eq!(
        search.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&search)
    );
    assert!(
        stdout_str(&search).contains(token_b),
        "search from A should span B and find its token: {}",
        stdout_str(&search)
    );
    // And the token in A is still found from A.
    let search_a = run(sandbox.command_in(&dir_a, &["search", token_a]), None);
    assert_eq!(search_a.status.code(), Some(0));
    assert!(
        stdout_str(&search_a).contains(token_a),
        "search from A should still find A's own token: {}",
        stdout_str(&search_a)
    );
}

// VAL-GAP-DATA-003: a session created in a project directory keeps its project identity across a
// process restart (a separate `cortex` invocation re-reads it from the durable store).
#[test]
fn session_retains_project_identity_across_restart() {
    let sandbox = Sandbox::new("session-identity");
    let project_dir = sandbox.mkdir("workspace");

    let new = run(sandbox.command_in(&project_dir, &["session", "new"]), None);
    assert_eq!(new.status.code(), Some(0), "stderr: {}", stderr_str(&new));
    let session_id = stdout_str(&new).trim().to_owned();
    assert!(!session_id.is_empty(), "session new should print an id");

    // A fresh process (restart) lists the project's sessions and still sees the linked session.
    let sessions = run(
        sandbox.command_in(&project_dir, &["project", "sessions"]),
        None,
    );
    assert_eq!(
        sessions.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&sessions)
    );
    assert!(
        stdout_str(&sessions)
            .lines()
            .any(|line| line.trim() == session_id),
        "the project should retain its session across restart: {} (want {session_id})",
        stdout_str(&sessions)
    );
}

// VAL-GAP-DATA-002: account state persists durably across a restart, the secret sink is 0600, and
// no raw secret leaks into any output or the database file.
#[test]
fn account_records_persist_with_redaction_and_safe_permissions() {
    let sandbox = Sandbox::new("account");
    let project = sandbox.mkdir("project");
    let key = fake_key("acct");

    let login = run(
        sandbox.command_in(
            &project,
            &[
                "auth",
                "login",
                "openrouter",
                "--api-key",
                &key,
                "--format",
                "json",
            ],
        ),
        None,
    );
    assert_eq!(
        login.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&login)
    );
    no_panic(&stderr_str(&login));
    assert!(
        !stdout_str(&login).contains(&key) && !stderr_str(&login).contains(&key),
        "login output must never echo the raw key"
    );

    // The secret sink exists and is 0600.
    let auth_file = sandbox.auth_file();
    assert!(auth_file.exists(), "login must create the auth store");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&auth_file)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "auth store must be 0600");
    }

    // A fresh process (restart) still reports the durable account record AND the reusable
    // credential, with no raw secret in the output.
    let list = run(
        sandbox.command_in(&project, &["auth", "list", "--format", "json"]),
        None,
    );
    assert_eq!(list.status.code(), Some(0), "stderr: {}", stderr_str(&list));
    let doc: Value = serde_json::from_slice(&list.stdout).expect("strict auth list JSON");
    assert!(
        doc["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .any(|provider| provider["id"] == "openrouter"),
        "credential should be reused after restart: {doc:?}"
    );
    assert!(
        doc["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .any(|account| account["provider"] == "openrouter"),
        "durable account record should persist after restart: {doc:?}"
    );
    assert!(
        !stdout_str(&list).contains(&key),
        "auth list must never print the raw key"
    );

    // The database file holds account metadata but never the raw secret.
    let db_bytes = fs::read(sandbox.db_path()).expect("read db file");
    assert!(
        !String::from_utf8_lossy(&db_bytes).contains(&key),
        "the database file must never contain the raw secret"
    );

    // Logout removes the durable account record too.
    let logout = run(
        sandbox.command_in(&project, &["auth", "logout", "openrouter"]),
        None,
    );
    assert_eq!(
        logout.status.code(),
        Some(0),
        "stderr: {}",
        stderr_str(&logout)
    );
    let list_after = run(
        sandbox.command_in(&project, &["auth", "list", "--format", "json"]),
        None,
    );
    assert_eq!(list_after.status.code(), Some(0));
    let doc_after: Value =
        serde_json::from_slice(&list_after.stdout).expect("strict auth list JSON");
    assert!(
        doc_after["accounts"]
            .as_array()
            .expect("accounts array")
            .iter()
            .all(|account| account["provider"] != "openrouter"),
        "account record should be gone after logout: {doc_after:?}"
    );
}
