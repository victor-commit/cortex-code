//! Live, end-to-end proof that the durable `cortex run` lifecycle is hardened (architecture §4).
//! NO mocks — every test here drives a real OpenRouter turn (default `google/gemini-2.5-flash`).
//!
//! Gated on `OPENROUTER_API_KEY`; each test skips cleanly when it is absent so the offline
//! `cargo test --workspace` gate stays green. Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to the feature's fulfilled assertions:
//! - VAL-RUN-003: the reported step/iteration count in `--jsonl`/`--format json` is `<= 25`.
//! - VAL-RUN-004: a tiny idle budget aborts a turn within a bounded window with a transport
//!   diagnostic on stderr and a non-zero exit (fail-closed), not a hang.
//! - VAL-RUN-005: SIGINT during streaming exits cleanly (no panic/backtrace, no orphan) promptly.
//! - VAL-RUN-006: a session survives an interrupted turn and a later input is still answered.
//! - VAL-RUN-013: interrupting a `--format json` run leaves stdout that is valid JSON or empty.
//!
//! Process hygiene: every child is launched in its own process group and signalled by its exact
//! PID; the group is checked for survivors after exit so no orphan is left behind.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The default model the mission targets.
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A unique scratch directory for hermetic per-test state (an isolated `HOME` + database).
fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-run-robust-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `cortex` command carrying the live key and a clean, non-verbose environment so stdout stays
/// pure machine/prose output. Ambient robustness knobs are cleared so a test sets them explicitly.
fn cortex(key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR")
        .env_remove("CORTEX_STREAM_IDLE_TIMEOUT_MS");
    c
}

/// Spawn the binary and collect its output, enforcing `timeout` as a hang guard. The exact child
/// PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
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
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {timeout:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

/// Send SIGINT to an exact PID via `kill -INT`. Returns whether the signal was delivered (a process
/// that already exited yields a non-success `kill`, which is not a failure for the delay path).
fn send_sigint(pid: u32) -> bool {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Wait for a child to exit within `timeout`, returning its status. Returns `None` (after a
/// force-kill) if the child did not exit on its own in time — i.e. the interrupt did not work.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Whether a process group still has live members. `Some(true)`/`Some(false)` from `pgrep -g`;
/// `None` when `pgrep` is unavailable (the orphan check is then skipped).
fn group_has_survivors(pgid: u32) -> Option<bool> {
    let out = Command::new("pgrep")
        .args(["-g", &pgid.to_string()])
        .output()
        .ok()?;
    match out.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// The outcome of interrupting a streamed run with SIGINT.
struct Interrupted {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pgid: u32,
}

/// Spawn `command` in its own process group with piped streams and a null stdin.
fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn cortex binary")
}

/// Drain a child's stdout/stderr on background threads, returning a first-byte signal for stdout
/// and join handles for the collected bytes.
#[allow(clippy::type_complexity)]
fn drain_streams(
    child: &mut Child,
) -> (
    mpsc::Receiver<()>,
    std::thread::JoinHandle<Vec<u8>>,
    std::thread::JoinHandle<Vec<u8>>,
) {
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let (tx, rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            match out.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let first = buf.is_empty();
                    buf.extend_from_slice(&chunk[..n]);
                    if first {
                        let _ = tx.send(());
                    }
                }
                Err(_) => break,
            }
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    (rx, out_thread, err_thread)
}

/// Spawn `command`, wait until the first stdout byte arrives (bounded by `first_output`), send
/// SIGINT to the exact PID, then wait up to `exit_within` for a clean exit.
fn interrupt_after_first_output(
    command: Command,
    first_output: Duration,
    exit_within: Duration,
) -> Interrupted {
    let mut child = spawn_group(command);
    let pid = child.id();
    let pgid = pid;
    let (rx, out_thread, err_thread) = drain_streams(&mut child);

    let saw_output = rx.recv_timeout(first_output).is_ok();
    assert!(
        saw_output,
        "expected streamed output before interrupting (pid {pid})"
    );
    assert!(
        send_sigint(pid),
        "kill -INT must reach the live child {pid}"
    );

    let status = wait_for_exit(&mut child, exit_within);
    Interrupted {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
        pgid,
    }
}

/// Spawn `command`, wait a fixed `delay` (for formats that buffer stdout), send SIGINT, then wait
/// up to `exit_within` for a clean exit.
fn interrupt_after_delay(command: Command, delay: Duration, exit_within: Duration) -> Interrupted {
    let mut child = spawn_group(command);
    let pid = child.id();
    let pgid = pid;
    let (_rx, out_thread, err_thread) = drain_streams(&mut child);

    std::thread::sleep(delay);
    // The turn may already be complete; a failed signal to a dead PID is acceptable here.
    let _ = send_sigint(pid);

    let status = wait_for_exit(&mut child, exit_within);
    Interrupted {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
        pgid,
    }
}

/// Assert a child's stderr carries no Rust panic/backtrace text.
fn assert_no_panic(stderr: &[u8]) {
    let text = String::from_utf8_lossy(stderr);
    for marker in ["panicked at", "RUST_BACKTRACE", "stack backtrace"] {
        assert!(
            !text.contains(marker),
            "stderr must not contain a panic/backtrace ({marker}): {text:?}"
        );
    }
}

/// Assert a child's process group left no orphan/child behind (best-effort when `pgrep` exists).
fn assert_no_orphans(pgid: u32) {
    if let Some(true) = group_has_survivors(pgid) {
        panic!("process group {pgid} still has live members after exit (orphan leaked)");
    }
}

// VAL-RUN-003: the reported step/iteration count in machine output never exceeds MAX_STEPS=25, in
// both `--format json` and `--jsonl`.
#[test]
fn step_count_is_bounded_in_machine_output() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    // `--format json`: the document carries a bounded integer step count.
    let out = run(
        cortex(&key, &["run", "--format", "json", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0), "a real turn must exit 0");
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be a single strict JSON document");
    let steps = doc["steps"]
        .as_u64()
        .expect("the document reports an integer step count");
    assert!(steps >= 1, "a completed turn took at least one step");
    assert!(
        steps <= 25,
        "the step count must never exceed MAX_STEPS=25, got {steps}"
    );

    // `--jsonl`: the terminal done event carries a bounded step count.
    let out = run(
        cortex(&key, &["run", "--jsonl", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).expect("utf8 jsonl");
    let mut done_steps = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).expect("every jsonl line is a strict JSON object");
        if value["type"] == "done" {
            done_steps = value["steps"].as_u64();
        }
    }
    let steps = done_steps.expect("the jsonl stream ends with a done event carrying a step count");
    assert!(
        steps <= 25,
        "the jsonl step count must never exceed MAX_STEPS=25, got {steps}"
    );
}

// VAL-RUN-004: a deliberately tiny idle budget aborts a stalled/slow upstream within a bounded
// window with a transport/timeout diagnostic on stderr and a non-zero exit (fail-closed), well
// under the 120s default that would otherwise apply — proving the override took effect, not a hang.
#[test]
fn tiny_idle_budget_fails_closed_with_a_transport_diagnostic() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let mut command = cortex(
        &key,
        &["run", "Count slowly from 1 to 50, one number per line."],
    );
    command.env("CORTEX_STREAM_IDLE_TIMEOUT_MS", "1");

    // A 30s hang guard is far below the 120s default idle timeout: reaching the assertions proves
    // the tiny override bounded the turn rather than the run hanging until an external kill.
    let out = run(command, Duration::from_secs(30));
    assert_ne!(
        out.status.code(),
        Some(0),
        "a stalled-out turn must exit non-zero, not succeed"
    );
    assert_no_panic(&out.stderr);
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        stderr.contains("timeout") || stderr.contains("transport"),
        "stderr must carry a timeout/transport diagnostic, got: {stderr:?}"
    );
}

// VAL-RUN-005: SIGINT during streaming cancels the turn cleanly — the process exits promptly with
// no panic/backtrace and leaves no orphan/child behind.
#[test]
fn sigint_during_streaming_exits_cleanly() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let result = interrupt_after_first_output(
        cortex(&key, &["run", "Count slowly to 200, one per line."]),
        Duration::from_secs(60),
        Duration::from_secs(8),
    );

    let status = result
        .status
        .expect("the process must exit within a few seconds of SIGINT (no hang)");
    assert_no_panic(&result.stderr);
    let code = status.code();
    assert!(
        code == Some(130) || code == Some(0),
        "a SIGINT-cancelled turn exits 130 (interrupted) or 0 (already complete), got {code:?}"
    );
    assert_no_orphans(result.pgid);
}

// VAL-RUN-006: a session survives an interrupted turn — after establishing a fact and interrupting
// a following long turn, a later run on the same session still recalls the fact with exit 0.
#[test]
fn session_survives_an_interrupt_and_stays_resumable() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let home = scratch_dir("survives");
    let db_path = home.join("cortex.db");
    let session = format!("val-run-006-{}", std::process::id());
    let nonce = format!("zephyr{}", std::process::id());

    let with_state = |key: &str, args: &[&str]| {
        let mut c = cortex(key, args);
        c.env("HOME", &home);
        c.env("CORTEX_DB_PATH", &db_path);
        c
    };

    // 1) Establish a fact in the session (completes normally and is persisted).
    let establish = run(
        with_state(
            &key,
            &[
                "run",
                "--session",
                &session,
                &format!("Remember this exact secret word: {nonce}. Reply with only: ok"),
            ],
        ),
        Duration::from_secs(60),
    );
    assert_eq!(
        establish.status.code(),
        Some(0),
        "establishing the fact must succeed"
    );

    // 2) Interrupt a following long turn on the same session.
    let interrupted = interrupt_after_first_output(
        with_state(
            &key,
            &[
                "run",
                "--session",
                &session,
                "Count slowly to 200, one per line.",
            ],
        ),
        Duration::from_secs(60),
        Duration::from_secs(8),
    );
    assert!(
        interrupted.status.is_some(),
        "the interrupted turn must exit promptly"
    );
    assert_no_panic(&interrupted.stderr);
    assert_no_orphans(interrupted.pgid);

    // 3) The session is uncorrupted and still resumable: the fact is recalled, exit 0.
    let recall = run(
        with_state(
            &key,
            &[
                "run",
                "--session",
                &session,
                "What exact secret word did I give you earlier? Reply with only that word.",
            ],
        ),
        Duration::from_secs(60),
    );
    assert_eq!(
        recall.status.code(),
        Some(0),
        "resuming the session after an interrupt must succeed (no DB/lock error), stderr: {:?}",
        String::from_utf8_lossy(&recall.stderr)
    );
    let recalled = String::from_utf8_lossy(&recall.stdout);
    assert!(
        recalled.contains(&nonce),
        "the resumed session must recall the fact {nonce:?}, got: {recalled:?}"
    );
}

// VAL-RUN-013: interrupting a `--format json` run never leaves malformed JSON on stdout — stdout is
// either a complete parseable JSON document or byte-empty, and no orphan remains.
#[test]
fn sigint_during_json_run_never_leaves_partial_json() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let result = interrupt_after_delay(
        cortex(&key, &["run", "--format", "json", "Count slowly to 200."]),
        Duration::from_millis(1500),
        Duration::from_secs(8),
    );

    assert!(
        result.status.is_some(),
        "the process must exit promptly after SIGINT (no hang)"
    );
    assert_no_panic(&result.stderr);

    // stdout is either byte-empty (interrupted before the document was written) or a complete,
    // strictly-parseable JSON document (the turn finished first) — never a parse error.
    if result.stdout.is_empty() {
        // byte-empty: acceptable.
    } else {
        let doc: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout must be empty or strict JSON, got a parse error {e}: {:?}",
                String::from_utf8_lossy(&result.stdout)
            )
        });
        assert_eq!(
            doc["role"], "assistant",
            "a complete document is the assistant answer"
        );
        assert_eq!(doc["model"], DEFAULT_MODEL);
    }
    assert_no_orphans(result.pgid);
}
