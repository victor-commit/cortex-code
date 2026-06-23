//! Black-box checks for `cortex replay` (VAL-FINISH-039..044).
//!
//! Replay must reconstruct real persisted transcripts, preserve order/roles, fail clearly on an
//! unknown session id, stay local-only under `CORTEX_NO_NET=1`, and emit strict machine JSON.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

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
            "cortex-replay-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create scratch HOME");
        Self {
            db_path: root.join("state/cortex.db"),
            root,
            home,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_with_env(args, &[])
    }

    fn command_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("CORTEX_DB_PATH", &self.db_path)
            .env_remove("OPENROUTER_API_KEY");
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        collect(spawn(self.command(args)))
    }

    fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        collect(spawn(self.command_with_env(args, env)))
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
            panic!("cortex replay did not terminate within {RUN_TIMEOUT:?}");
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
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
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

fn replay_json(sandbox: &Sandbox, session_id: &str) -> Value {
    let out = sandbox.run(&["replay", session_id, "--format", "json"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        out.stderr.is_empty(),
        "successful replay JSON should keep stderr empty: {}",
        stderr_str(&out)
    );
    serde_json::from_slice(&out.stdout).expect("replay JSON")
}

fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

fn strace_connect_trace(sandbox: &Sandbox, session_id: &str) -> String {
    let out = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "replay",
            session_id,
        ])
        .env_clear()
        .env("HOME", &sandbox.home)
        .env("XDG_CONFIG_HOME", sandbox.root.join(".config"))
        .env("CORTEX_DB_PATH", &sandbox.db_path)
        .env("CORTEX_NO_NET", "1")
        .output()
        .expect("spawn strace");
    assert_eq!(
        out.status.code(),
        Some(0),
        "replay under strace should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn count_connect(trace: &str) -> usize {
    trace.matches("connect(").count()
}

// VAL-FINISH-039.
#[test]
fn replay_existing_session_reconstructs_real_persisted_content() {
    let sandbox = Sandbox::new("existing");
    let session_id = new_session(&sandbox);
    append_message(
        &sandbox,
        &session_id,
        "user",
        "ZEBRA-NONCE-REPLAY-SESSION-CONTENT",
    );
    append_message(
        &sandbox,
        &session_id,
        "assistant",
        "assistant-echo-for-zebra-nonce",
    );

    let out = sandbox.run(&["replay", &session_id]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    let stdout = stdout_str(&out);
    assert!(
        stdout.contains("ZEBRA-NONCE-REPLAY-SESSION-CONTENT"),
        "replay must include real persisted user content: {stdout}"
    );
    assert!(
        stdout.contains("assistant-echo-for-zebra-nonce"),
        "replay must include real persisted assistant content: {stdout}"
    );
}

// VAL-FINISH-040.
#[test]
fn replay_is_not_a_constant_stub_and_reflects_each_sessions_messages() {
    let sandbox = Sandbox::new("stub");
    let session_a = new_session(&sandbox);
    append_message(&sandbox, &session_a, "user", "REPLAY-MARKER-A-USER");
    append_message(
        &sandbox,
        &session_a,
        "assistant",
        "REPLAY-MARKER-A-ASSISTANT",
    );

    let session_b = new_session(&sandbox);
    append_message(&sandbox, &session_b, "user", "REPLAY-MARKER-B-USER");
    append_message(
        &sandbox,
        &session_b,
        "assistant",
        "REPLAY-MARKER-B-ASSISTANT",
    );
    append_message(&sandbox, &session_b, "user", "REPLAY-MARKER-B-TAIL");

    let replay_a = sandbox.run(&["replay", &session_a]);
    let replay_b = sandbox.run(&["replay", &session_b]);
    assert_eq!(replay_a.status.code(), Some(0));
    assert_eq!(replay_b.status.code(), Some(0));

    let stdout_a = stdout_str(&replay_a);
    let stdout_b = stdout_str(&replay_b);
    assert_ne!(
        stdout_a, stdout_b,
        "distinct sessions must replay as distinct transcripts"
    );
    assert!(stdout_a.contains("REPLAY-MARKER-A-USER"));
    assert!(stdout_a.contains("REPLAY-MARKER-A-ASSISTANT"));
    assert!(!stdout_a.contains("REPLAY-MARKER-B-USER"));
    assert!(stdout_b.contains("REPLAY-MARKER-B-USER"));
    assert!(stdout_b.contains("REPLAY-MARKER-B-ASSISTANT"));
    assert!(!stdout_b.contains("REPLAY-MARKER-A-USER"));
    assert!(
        !stdout_a.to_lowercase().contains("open the tui")
            && !stdout_b.to_lowercase().contains("open the tui"),
        "replay must not be a display-only 'open the TUI' stub"
    );

    let json_a = replay_json(&sandbox, &session_a);
    let json_b = replay_json(&sandbox, &session_b);
    assert_eq!(json_a["messages"].as_array().expect("messages").len(), 2);
    assert_eq!(json_b["messages"].as_array().expect("messages").len(), 3);
}

// VAL-FINISH-041 + VAL-FINISH-044.
#[test]
fn replay_json_preserves_message_order_roles_and_text_fields() {
    let sandbox = Sandbox::new("order");
    let session_id = new_session(&sandbox);
    let sequence = [
        ("user", "ORDER-MARKER-1"),
        ("assistant", "ORDER-MARKER-2"),
        ("user", "ORDER-MARKER-3"),
        ("assistant", "ORDER-MARKER-4"),
    ];
    for (role, text) in sequence {
        append_message(&sandbox, &session_id, role, text);
    }

    let doc = replay_json(&sandbox, &session_id);
    let messages = doc["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), sequence.len());
    for (index, (role, text)) in sequence.iter().enumerate() {
        let entry = &messages[index];
        assert_eq!(entry["role"], *role, "role order mismatch at index {index}");
        assert_eq!(entry["text"], *text, "text order mismatch at index {index}");
    }
}

// VAL-FINISH-042.
#[test]
fn replay_unknown_session_fails_with_clear_error_and_no_stdout() {
    let sandbox = Sandbox::new("missing");
    let missing = "ses_missing_replay_123";
    let out = sandbox.run(&["replay", missing]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "replay of an unknown session must fail"
    );
    assert!(
        out.stdout.is_empty(),
        "unknown-session replay must not emit transcript stdout"
    );
    let stderr = stderr_str(&out);
    assert!(
        stderr.contains(missing),
        "stderr must name the missing session id: {stderr}"
    );
    assert!(
        stderr.to_lowercase().contains("no such session"),
        "stderr should clearly explain the session is missing: {stderr}"
    );
    let stderr_lower = stderr.to_lowercase();
    assert!(
        !stderr_lower.contains("panic") && !stderr_lower.contains("backtrace"),
        "error path must not panic or print a backtrace: {stderr}"
    );
}

// VAL-FINISH-043.
#[test]
fn replay_is_local_and_works_under_no_net_with_zero_connects() {
    let sandbox = Sandbox::new("none");
    let session_id = new_session(&sandbox);
    append_message(&sandbox, &session_id, "user", "NO-NET-REPLAY-MARKER");

    let out = sandbox.run_with_env(&["replay", &session_id], &[("CORTEX_NO_NET", "1")]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr_str(&out));
    assert!(
        stdout_str(&out).contains("NO-NET-REPLAY-MARKER"),
        "replay under CORTEX_NO_NET should still read persistence locally"
    );

    if !strace_usable() {
        eprintln!("skipping strace connect assertion: strace unavailable or ptrace restricted");
        return;
    }

    let trace = strace_connect_trace(&sandbox, &session_id);
    assert_eq!(
        count_connect(&trace),
        0,
        "replay should issue zero connect() syscalls under CORTEX_NO_NET=1:\n{trace}"
    );
}
