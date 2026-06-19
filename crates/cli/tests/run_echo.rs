//! Black-box checks for the headless `cortex run` path AFTER the M0→M1 transition.
//!
//! `cortex run` now drives a real OpenRouter turn, superseding the M0 echo placeholder. These
//! tests cover the still-valid, network-free invariants of the new path (no live OpenRouter turn
//! is needed for any of them; the live behavior is proven in `run_live.rs`):
//!
//! - **No-input determinism** (the still-valid part of VAL-SKEL-015): no argument and empty stdin
//!   terminates fast with a clear `no input` diagnostic and a clean stdout — never a hang.
//! - **No-credential error** (VAL-LLM-022, which replaces the M0 VAL-SKEL-016 "echo needs no
//!   key/network" expectation): with no key available, a run fails fast with a clear missing-
//!   credential diagnostic and writes no answer to stdout.
//! - **Net-disabled, fail-closed** (the CORTEX_NO_NET behavior that also replaces VAL-SKEL-016):
//!   the kill-switch refuses before any socket and before the credential is even consulted.
//! - **Input-source combine determinism** (VAL-SKEL-067 + clean stdout): a positional argument and
//!   piped stdin are combined (piped stdin first, then the argument) without hanging and reach the
//!   network path deterministically, and every error keeps stdout byte-empty.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Generous upper bound: every path under test fails fast locally (before any network), so any
/// run that exceeds this is treated as a hang and fails the test loudly instead of blocking.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);

/// The dedicated exit code for a kill-switch refusal (`Exit::NetDisabled`).
const NET_DISABLED_CODE: i32 = 4;

/// A `cortex` invocation with a hermetic environment: the kill-switch, any inherited credential,
/// the data-dir overrides, and verbosity are all cleared, and HOME points at a fresh empty temp
/// directory so no global config or on-disk auth store can supply a key. Each test then sets
/// exactly the environment it intends to exercise.
fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(scratch_home());
    c.env("HOME", scratch_home());
    c.env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    c
}

/// A unique, freshly created empty temp directory used as a hermetic HOME/CWD.
fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-run-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// Run the binary with a hard timeout, optionally feeding `stdin_data`.
///
/// stdout/stderr are drained on dedicated threads so a child that produces output can never
/// deadlock against an undrained pipe, and stdin is written on its own thread so a child that
/// ignores stdin (the precedence case) cannot deadlock the writer.
fn run(mut command: Command, stdin_data: Option<&str>) -> Output {
    command.stdin(if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");

    if let Some(data) = stdin_data {
        let mut handle = child.stdin.take().expect("stdin handle");
        let owned = data.to_owned();
        std::thread::spawn(move || {
            let _ = handle.write_all(owned.as_bytes());
        });
    }

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
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

// Still-valid part of VAL-SKEL-015: no argument and empty stdin terminates deterministically
// (no hang), with the diagnostic on stderr and stdout left clean. This invariant survives the
// transition to the real runner unchanged.
#[test]
fn empty_input_is_deterministic_and_terminates() {
    let out = run(cmd(&["run"]), None);

    assert_eq!(
        out.status.code(),
        Some(3),
        "no input maps to the stable MissingInput exit code"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must stay clean when there is no input"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("no input"),
        "stderr must carry a clear `no input` diagnostic, got: {stderr}"
    );

    let again = run(cmd(&["run"]), None);
    assert_eq!(
        out.status.code(),
        again.status.code(),
        "the empty-input exit code must be identical across runs"
    );
}

// VAL-LLM-022 (replaces the M0 VAL-SKEL-016 echo "needs no credential"): with no key available
// anywhere, a run that needs the model fails fast with an explicit missing-credential diagnostic
// and writes no answer to stdout — not a crash and not a generic network error.
#[test]
fn missing_credential_errors_clearly_with_clean_stdout() {
    let out = run(cmd(&["run", "hi"]), None);

    assert_ne!(out.status.code(), Some(0), "a keyless run must fail");
    assert!(
        out.stdout.is_empty(),
        "no assistant answer may be written without a credential, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("credential") || lower.contains("api key"),
        "stderr must name the missing credential, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "a missing credential must not panic, got: {stderr}"
    );
}

// CORTEX_NO_NET behavior (also replaces the M0 VAL-SKEL-016 "needs no network"): with the
// kill-switch engaged the run refuses with the dedicated net-disabled code and a clear,
// distinguishable diagnostic, writing nothing to stdout.
#[test]
fn net_disabled_refuses_the_run_with_clean_stdout() {
    let mut command = cmd(&["run", "hi"]);
    command.env("OPENROUTER_API_KEY", "sk-or-v1-not-a-real-key");
    command.env("CORTEX_NO_NET", "1");

    let out = run(command, None);

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "the kill-switch must exit with the dedicated net-disabled code"
    );
    assert!(
        out.stdout.is_empty(),
        "a refusal keeps stdout byte-empty for machine consumers"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network disabled") && stderr.contains("CORTEX_NO_NET"),
        "stderr must read as a net-disabled error naming the kill-switch, got: {stderr}"
    );
    assert!(
        !stderr.contains("missing credential"),
        "net-disabled must not be confused with a credential error, got: {stderr}"
    );
}

// Fail-closed ordering: the kill-switch is checked before the credential, so a keyless run under
// CORTEX_NO_NET=1 still refuses with net-disabled (never reaching the credential check).
#[test]
fn net_disabled_is_checked_before_the_credential() {
    let mut command = cmd(&["run", "hi"]);
    command.env("CORTEX_NO_NET", "1"); // no key set by the hermetic `cmd`

    let out = run(command, None);

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "with the switch engaged the run must fail closed before the credential check"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network disabled"),
        "the refusal must be net-disabled, not no-credential, got: {stderr}"
    );
}

// VAL-RUN-010 (no-panic portion, offline): `--continue` on a fresh HOME/data dir terminates
// deterministically without a panic/backtrace. Without a credential the run fails fast at provider
// setup (before any session/DB work), proving the `--continue` flag is wired and never panics or
// hangs; the live test proves the fresh-session-and-answer path with a real key.
#[test]
fn continue_on_fresh_dir_terminates_cleanly_without_a_credential() {
    let out = run(cmd(&["run", "--continue", "Say hi"]), None);

    assert_ne!(
        out.status.code(),
        Some(0),
        "without a credential the run must fail, not answer"
    );
    assert_ne!(
        out.status.code(),
        None,
        "the process must exit (no signal/hang) on a fresh-dir --continue"
    );
    assert!(
        out.stdout.is_empty(),
        "no answer may be written, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "a fresh-dir --continue must never panic, got: {stderr}"
    );
}

// `--session <id>` likewise reaches the (refused) network path without panicking, proving the flag
// is accepted and wired. The net kill-switch makes the outcome deterministic and key-free.
#[test]
fn explicit_session_flag_is_wired_without_panic() {
    let mut command = cmd(&["run", "--session", "val-deterministic-id", "hi"]);
    command.env("CORTEX_NO_NET", "1");

    let out = run(command, None);

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "the kill-switch refuses before any socket, even with --session"
    );
    assert!(out.stdout.is_empty(), "a refusal keeps stdout byte-empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "--session must never panic, got: {stderr}"
    );
}

// VAL-SKEL-067 (offline invariants): with BOTH an argument and piped stdin, `cortex run` combines
// the two sources (piped stdin first, then the argument, joined by a blank line) without hanging
// and reaches the network path deterministically. Observed here via the net-disabled refusal,
// which fires only after the prompt is resolved (stdin fully drained, no blocking) and keeps
// stdout byte-empty, identically across runs. The combined order actually reaching the model over
// a real OpenRouter turn is proven live in `run_live.rs`.
#[test]
fn combining_argument_and_stdin_does_not_hang_and_is_deterministic() {
    let invoke = || {
        let mut command = cmd(&["run", "FROM_ARG"]);
        command.env("CORTEX_NO_NET", "1");
        run(command, Some("FROM_STDIN\n"))
    };

    let first = invoke();
    let second = invoke();

    assert_eq!(
        first.status.code(),
        Some(NET_DISABLED_CODE),
        "both sources are combined and the run reaches the (refused) network path"
    );
    assert_eq!(
        first.status.code(),
        second.status.code(),
        "the combined prompt and outcome must be identical across repeated runs"
    );
    assert!(
        first.stdout.is_empty() && second.stdout.is_empty(),
        "no answer is produced under the kill-switch"
    );
}
