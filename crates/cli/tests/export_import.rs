//! Black-box checks for `cortex export` / `cortex import` round-trip behavior.
//! Coverage: portable artifact emission, role/content/order round-trip preservation, clear error
//! handling for missing/malformed input, and secret redaction in exported artifacts.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use redaction::PLACEHOLDER;
use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    db_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-export-import-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create scratch HOME");
        let db_path = root.join("state/cortex.db");
        Self {
            root,
            home,
            db_path,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("CORTEX_DB_PATH", &self.db_path)
            .env("CORTEX_NO_NET", "1");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        collect(spawn(self.command(args)))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spawn(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.spawn().expect("spawn cortex binary")
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

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {RUN_TIMEOUT:?}");
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

fn new_session(sandbox: &Sandbox) -> String {
    let out = sandbox.run(&["session", "new", "--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "session new failed: {}",
        stderr_str(&out)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).expect("session JSON");
    doc["id"].as_str().expect("session id").to_owned()
}

fn append_message(sandbox: &Sandbox, session_id: &str, role: &str, content: &str) {
    let out = sandbox.run(&[
        "session", "message", session_id, content, "--role", role, "--format", "json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "session message failed: {}",
        stderr_str(&out)
    );
}

fn parse_artifact(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("artifact JSON")
}

fn normalized_messages(doc: &Value) -> Vec<(String, String)> {
    doc["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .map(|msg| {
            (
                msg["role"].as_str().expect("role").to_owned(),
                msg["content"].as_str().expect("content").to_owned(),
            )
        })
        .collect()
}

// VAL-FINISH-025: export emits a non-empty, parseable artifact that includes session id + marker.
#[test]
fn export_emits_portable_artifact_with_session_and_marker_text() {
    let sandbox = Sandbox::new("export");
    let session_id = new_session(&sandbox);
    append_message(
        &sandbox,
        &session_id,
        "user",
        "marker-export-import-hello-from-session",
    );

    let out = sandbox.run(&["export", &session_id]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "export failed: {}",
        stderr_str(&out)
    );
    assert!(
        !out.stdout.is_empty(),
        "export must emit a non-empty artifact on stdout"
    );
    assert!(
        out.stderr.is_empty(),
        "successful export should not emit stderr diagnostics: {}",
        stderr_str(&out)
    );

    let artifact = parse_artifact(&out);
    assert_eq!(artifact["version"], 1);
    assert_eq!(artifact["session"]["id"], session_id);
    let contains_marker = artifact["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .any(|m| m["content"] == "marker-export-import-hello-from-session");
    assert!(
        contains_marker,
        "artifact must contain the marker message text"
    );
}

// VAL-FINISH-026 + VAL-FINISH-027: export -> import -> re-export preserves role/content/order.
#[test]
fn import_round_trip_preserves_message_content_roles_and_order() {
    let source = Sandbox::new("source");
    let session_id = new_session(&source);
    append_message(&source, &session_id, "user", "marker-round-trip-user");
    append_message(
        &source,
        &session_id,
        "assistant",
        "marker-round-trip-assistant",
    );
    append_message(&source, &session_id, "user", "marker-round-trip-third");

    let export_out = source.run(&["export", &session_id]);
    assert_eq!(
        export_out.status.code(),
        Some(0),
        "source export failed: {}",
        stderr_str(&export_out)
    );
    let artifact_path = source.path("session-export.json");
    fs::write(&artifact_path, &export_out.stdout).expect("write artifact file");

    let target = Sandbox::new("target");
    let import_out = target.run(&["import", artifact_path.to_str().expect("utf8 path")]);
    assert_eq!(
        import_out.status.code(),
        Some(0),
        "import failed: {}",
        stderr_str(&import_out)
    );
    assert!(
        stdout_str(&import_out).contains(&session_id),
        "successful import should report the imported session id"
    );

    let reexport_out = target.run(&["export", &session_id]);
    assert_eq!(
        reexport_out.status.code(),
        Some(0),
        "re-export after import failed: {}",
        stderr_str(&reexport_out)
    );

    let original = parse_artifact(&export_out);
    let round_tripped = parse_artifact(&reexport_out);
    let original_messages = normalized_messages(&original);
    let round_tripped_messages = normalized_messages(&round_tripped);
    assert_eq!(
        round_tripped_messages, original_messages,
        "normalized role/content message arrays must be identical after round-trip"
    );
    assert_eq!(
        round_tripped_messages.len(),
        original_messages.len(),
        "message count must be preserved exactly"
    );
}

// VAL-FINISH-029: missing session export fails clearly, no stdout artifact, no panic.
#[test]
fn export_of_missing_session_fails_cleanly() {
    let sandbox = Sandbox::new("missing-export");
    let out = sandbox.run(&["export", "ses_no_such_session"]);
    assert_ne!(out.status.code(), Some(0), "missing export must fail");
    assert!(
        out.stdout.is_empty(),
        "missing export must not emit an artifact on stdout"
    );
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("ses_no_such_session"),
        "error should name the missing session id: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "no panic output: {stderr}");
}

// VAL-FINISH-030: malformed artifact import fails cleanly and leaves DB usable.
#[test]
fn import_of_malformed_artifact_fails_without_corrupting_db() {
    let sandbox = Sandbox::new("bad-import");
    let bad_path = sandbox.path("bad-artifact.json");
    fs::write(&bad_path, "{ not valid json ").expect("write malformed artifact");

    let out = sandbox.run(&["import", bad_path.to_str().expect("utf8 path")]);
    assert_ne!(out.status.code(), Some(0), "malformed import must fail");
    assert!(
        out.stdout.is_empty(),
        "malformed import must not emit success text on stdout"
    );
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains("invalid") || stderr.contains("unparseable"),
        "error should clearly describe malformed/unparseable artifact: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "no panic output: {stderr}");

    let follow_up = sandbox.run(&["session", "list", "--format", "json"]);
    assert_eq!(
        follow_up.status.code(),
        Some(0),
        "database should remain usable after malformed import failure: {}",
        stderr_str(&follow_up)
    );
    let listed: Value = serde_json::from_slice(&follow_up.stdout).expect("strict list JSON");
    assert!(
        listed["sessions"].is_array(),
        "session list JSON must remain valid after malformed import failure"
    );
}

// VAL-FINISH-031: exported artifacts never expose secret literals.
#[test]
fn export_redacts_secrets_from_artifact_content() {
    let sandbox = Sandbox::new("redaction");
    let session_id = new_session(&sandbox);

    let body = "cafebabe".repeat(8);
    let secret = format!("sk-{}-{}", "or-v1", body);
    let content = format!("do not leak this key: {secret}");
    append_message(&sandbox, &session_id, "user", &content);

    let mut cmd = sandbox.command(&["export", &session_id]);
    cmd.env("OPENROUTER_API_KEY", &secret);
    let out = collect(spawn(cmd));
    assert_eq!(
        out.status.code(),
        Some(0),
        "export with secret-bearing content should still succeed: {}",
        stderr_str(&out)
    );

    let artifact = stdout_str(&out);
    assert!(
        !artifact.contains(&secret),
        "artifact must not contain the literal secret value"
    );
    assert!(
        artifact.contains(PLACEHOLDER),
        "artifact should contain the canonical redaction placeholder when secret-like content appears"
    );
}
