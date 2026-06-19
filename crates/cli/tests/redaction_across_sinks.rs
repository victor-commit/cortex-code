//! Black-box proof that secret redaction is wired into every (network-free) sink (VAL-SKEL-031,
//! 043, 044, 046, 047).
//!
//! A single fake-but-realistic OpenRouter key is pushed through the config dump, persisted session
//! storage, and the verbose log stream. Each test runs the real binary with a cleared environment
//! in an isolated scratch tree and inspects the produced bytes: the raw key must never appear, the
//! canonical placeholder must, and non-secret data alongside it must survive verbatim.
//!
//! NOTE (M0→M1 transition): the M0 `cortex run` echo sink was a fourth redaction surface here. The
//! echo path has been superseded by a real OpenRouter turn, so the run-echo redaction sub-checks
//! were removed; redaction of model output on the live run path is proven elsewhere (VAL-LLM-024).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// A fake but realistically shaped OpenRouter API key (`sk-or-v1-` + 64 hex), assembled from
/// fragments so no contiguous key-shaped literal lives in source (secret scanners flag those).
fn secret_key() -> String {
    let body = "deadbeef".repeat(8);
    format!("sk-{}-{}", "or-v1", body)
}

/// The single canonical placeholder every sink must emit for a redaction.
const PLACEHOLDER: &str = "[REDACTED]";

/// A self-cleaning scratch tree: `<root>/home` is HOME and `<root>/home/proj` the project root.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-redaction-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("home/proj")).expect("create scratch tree");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn project(&self) -> PathBuf {
        self.root.join("home/proj")
    }

    fn write_project_config(&self, contents: &str) {
        fs::write(self.project().join("cortex.toml"), contents).expect("write project config");
    }

    /// Run the binary from the project directory with a cleared environment plus `env`, the given
    /// args, and an optional stdin body.
    fn run(&self, env: &[(&str, &str)], args: &[&str], stdin: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.env("HOME", self.home());
        for (key, value) in env {
            command.env(key, value);
        }
        command.current_dir(self.project());
        command.args(args);
        collect(spawn(command, stdin))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spawn(mut command: Command, stdin: Option<&str>) -> Child {
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command.spawn().expect("spawn cortex binary");
    if let Some(body) = stdin {
        use std::io::Write;
        child
            .stdin
            .take()
            .expect("stdin handle")
            .write_all(body.as_bytes())
            .expect("write stdin");
    }
    child
}

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

    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("wait on child") {
            let stdout = out_thread.join().expect("join stdout");
            let stderr = err_thread.join().expect("join stderr");
            return Output {
                status,
                stdout,
                stderr,
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex invocation timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Recursively collect the paths of every file under `dir`.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// Whether any byte window in `haystack` equals `needle` (a raw `strings`/`grep`-style scan).
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

// VAL-SKEL-031 + VAL-SKEL-045 + VAL-SKEL-047: a secret resolved into config via `{env:}` is
// scrubbed in the config dump, while a non-secret `{env:}` value and a plain key stay verbatim.
#[test]
fn config_dump_scrubs_resolved_secret_but_keeps_non_secrets() {
    let scratch = Scratch::new();
    scratch.write_project_config(
        "model = \"google/gemini-2.5-flash\"\n\
         [auth]\n\
         key = \"{env:OPENROUTER_API_KEY}\"\n\
         note = \"{env:PLAIN_NOTE}\"\n",
    );

    let secret = secret_key();
    let output = scratch.run(
        &[
            ("OPENROUTER_API_KEY", secret.as_str()),
            ("PLAIN_NOTE", "just-a-note"),
        ],
        &["config", "--format", "json"],
        None,
    );
    assert_eq!(output.status.code(), Some(0), "config must exit 0");
    assert!(
        !contains_bytes(&output.stdout, secret.as_bytes()),
        "raw secret must be absent from the config dump"
    );

    let report: Value = serde_json::from_slice(&output.stdout).expect("strict JSON dump");
    assert_eq!(
        report["auth"]["key"], PLACEHOLDER,
        "the resolved secret is reported as the canonical placeholder"
    );
    assert_eq!(
        report["auth"]["note"], "just-a-note",
        "a non-secret {{env:}} value is reported verbatim (targeted redaction)"
    );
    assert_eq!(
        report["model"], "google/gemini-2.5-flash",
        "a plain non-secret value is reported verbatim"
    );
}

// VAL-SKEL-044 + VAL-SKEL-047: a secret pushed through persisted storage never lands in the DB
// file (or any on-disk artifact); the placeholder is stored where the value was and adjacent
// non-secret text survives.
#[test]
fn persisted_data_is_scrubbed_on_disk() {
    let scratch = Scratch::new();
    let db_path = scratch.root.join("data/cortex.db");
    let db_env = [("CORTEX_DB_PATH", db_path.to_str().unwrap())];

    let created = scratch.run(&db_env, &["session", "new", "--format", "json"], None);
    assert_eq!(created.status.code(), Some(0), "session new must exit 0");
    let session: Value = serde_json::from_slice(&created.stdout).expect("session JSON");
    let id = session["id"].as_str().expect("session id");

    let secret = secret_key();
    let content = format!("token {secret} kept");
    let appended = scratch.run(
        &db_env,
        &["session", "message", id, &content, "--format", "json"],
        None,
    );
    assert_eq!(
        appended.status.code(),
        Some(0),
        "message create must exit 0"
    );

    // No on-disk artifact anywhere under the scratch root may contain the raw secret.
    for path in files_under(&scratch.root) {
        let bytes = fs::read(&path).expect("read artifact");
        assert!(
            !contains_bytes(&bytes, secret.as_bytes()),
            "raw secret leaked into {}",
            path.display()
        );
    }

    let db_bytes = fs::read(&db_path).expect("read db file");
    assert!(
        contains_bytes(&db_bytes, PLACEHOLDER.as_bytes()),
        "placeholder must be present where the secret value was"
    );
    assert!(
        contains_bytes(&db_bytes, b"token ") && contains_bytes(&db_bytes, b" kept"),
        "non-secret content around the secret must be persisted verbatim"
    );

    // Reading it back through the tool shows the placeholder, not the secret.
    let shown = scratch.run(&db_env, &["session", "show", id, "--format", "json"], None);
    let detail: Value = serde_json::from_slice(&shown.stdout).expect("detail JSON");
    let stored = detail["messages"][0]["content"]
        .as_str()
        .expect("message content");
    assert_eq!(stored, "token [REDACTED] kept");
}

// VAL-SKEL-043: a secret present in the environment is never emitted into the verbose log stream
// on stderr.
#[test]
fn verbose_logs_never_emit_the_secret() {
    let scratch = Scratch::new();
    scratch.write_project_config(
        "[auth]\n\
         key = \"{env:OPENROUTER_API_KEY}\"\n",
    );
    let secret = secret_key();
    let output = scratch.run(
        &[("OPENROUTER_API_KEY", secret.as_str())],
        &["config", "-vv", "--format", "json"],
        None,
    );
    assert!(
        !contains_bytes(&output.stderr, secret.as_bytes()),
        "raw secret must be absent from verbose logs: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !contains_bytes(&output.stdout, secret.as_bytes()),
        "raw secret must be absent from the config dump"
    );
}

// VAL-SKEL-046: the placeholder is one identical canonical token across the config dump and
// persisted storage for the same secret.
#[test]
fn placeholder_is_canonical_across_every_sink() {
    let scratch = Scratch::new();
    scratch.write_project_config(
        "[auth]\n\
         key = \"{env:OPENROUTER_API_KEY}\"\n",
    );
    let db_path = scratch.root.join("data/cortex.db");
    let db = db_path.to_str().unwrap();
    let secret = secret_key();

    let config = scratch.run(
        &[("OPENROUTER_API_KEY", secret.as_str())],
        &["config", "--format", "json"],
        None,
    );
    let config_report: Value = serde_json::from_slice(&config.stdout).expect("config JSON");
    let from_config = config_report["auth"]["key"].as_str().expect("config key");

    let created = scratch.run(
        &[("CORTEX_DB_PATH", db)],
        &["session", "new", "--format", "json"],
        None,
    );
    let session: Value = serde_json::from_slice(&created.stdout).expect("session JSON");
    let id = session["id"].as_str().expect("session id");
    let appended = scratch.run(
        &[("CORTEX_DB_PATH", db)],
        &["session", "message", id, &secret, "--format", "json"],
        None,
    );
    let message: Value = serde_json::from_slice(&appended.stdout).expect("message JSON");
    let from_db = message["content"].as_str().expect("message content");

    assert_eq!(from_config, PLACEHOLDER);
    assert_eq!(from_db, PLACEHOLDER);
    assert_eq!(from_config, from_db, "config and persistence agree");
}
