//! Black-box checks for the credential / transport / network-guard failure surfaces of
//! `cortex run`. These are all network-free in the sense that none reach OpenRouter: the
//! missing-credential and net-disabled paths fail before any socket, and the transport path
//! connects only to a guaranteed-closed loopback port (an immediate, deterministic refusal).
//!
//! Coverage:
//! - **VAL-LLM-022**: a keyless run fails fast with a clear missing-credential diagnostic that
//!   names how to provide the key (`OPENROUTER_API_KEY`), with a clean stdout and no backtrace.
//! - **VAL-LLM-032**: pointing the provider base URL at an unreachable loopback port (3200-3299,
//!   nothing listening) yields a transport/connection diagnostic distinct from net-disabled and
//!   missing-credential, within a bounded time, with no answer on stdout and no backtrace.
//! - **VAL-RUN-008**: `CORTEX_NO_NET=1 cortex run` fails closed before any socket — a syscall
//!   trace shows zero outbound `connect()` and the diagnostic reads as net-disabled.

use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Generous upper bound: every path under test fails fast (locally, or against an immediately
/// refusing loopback port), so exceeding this is treated as a hang and fails the test loudly.
const RUN_TIMEOUT: Duration = Duration::from_secs(15);

/// The dedicated exit code for a kill-switch refusal (`Exit::NetDisabled`).
const NET_DISABLED_CODE: i32 = 4;

/// A `cortex` invocation with a hermetic environment: the kill-switch, any inherited credential,
/// the data-dir overrides, the endpoint override, and verbosity are all cleared, and HOME points
/// at a fresh empty temp directory so no global config or on-disk auth store can supply a key.
fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(scratch_home());
    c.env("HOME", scratch_home());
    c.env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
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
        "cortex-run-errs-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// A fake-but-key-shaped OpenRouter secret assembled from fragments so no contiguous key-shaped
/// literal lives in source (the commit secret scanner flags those).
fn fake_key() -> String {
    format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
}

/// Find a port in the mission's 3200-3299 range that currently refuses connections, so a probe
/// against it yields a deterministic transport failure (never a hang).
fn closed_local_port() -> u16 {
    for port in (3200u16..=3299).rev() {
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("valid loopback addr");
        if TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_err() {
            return port;
        }
    }
    panic!("no closed port available in 3200-3299");
}

/// Run the binary with a hard timeout. stdout/stderr are drained on dedicated threads so a child
/// that produces output can never deadlock against an undrained pipe.
fn run(mut command: Command) -> Output {
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

fn assert_no_backtrace(stderr: &str) {
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "the failure must be a clean diagnostic, not a panic/backtrace: {stderr}"
    );
}

// VAL-LLM-022: with no key available anywhere, a run that needs the model fails fast with an
// explicit missing-credential diagnostic that NAMES how to provide the key, with a clean stdout,
// non-zero exit, and no backtrace.
#[test]
fn missing_credential_names_how_to_provide_the_key() {
    let out = run(cmd(&["run", "hi"]));

    assert_ne!(out.status.code(), Some(0), "a keyless run must fail");
    assert_ne!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "a missing credential is not a net-disabled refusal"
    );
    assert!(
        out.stdout.is_empty(),
        "no assistant answer may be written without a credential, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("credential") || lower.contains("api key"),
        "stderr must name a missing/required credential, got: {stderr}"
    );
    assert!(
        stderr.contains("OPENROUTER_API_KEY"),
        "stderr must name how to provide the key, got: {stderr}"
    );
    assert!(
        !lower.contains("network disabled") && !lower.contains("transport unavailable"),
        "a missing credential must be distinct from net-disabled and transport errors, got: {stderr}"
    );
    assert_no_backtrace(&stderr);
}

// VAL-LLM-032: pointing the provider base URL at an unreachable loopback port (nothing listening)
// yields a transport/connection diagnostic that is plainly distinct from a net-disabled refusal
// and from a missing-credential error, within a bounded time, with no answer on stdout and no
// backtrace. A (fake) key is present so the run reaches the transport rather than the credential
// check.
#[test]
fn unreachable_endpoint_is_a_transport_error_distinct_from_net_and_credential() {
    let port = closed_local_port();
    let base_url = format!("http://127.0.0.1:{port}");

    let mut command = cmd(&["run", "hi"]);
    command.env("OPENROUTER_API_KEY", fake_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", &base_url);

    let out = run(command);

    assert_eq!(
        out.status.code(),
        Some(1),
        "a refused connection is a generic transport failure (exit 1), got: {:?}",
        out.status.code()
    );
    assert_ne!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "an unreachable endpoint must not produce the net-disabled code"
    );
    assert!(
        out.stdout.is_empty(),
        "no partial answer may be written on a transport failure, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("transport unavailable"),
        "stderr must read as a transport/connection error, got: {stderr}"
    );
    assert!(
        !lower.contains("network disabled"),
        "a transport error must not be confused with a net-disabled refusal, got: {stderr}"
    );
    assert!(
        !lower.contains("missing credential"),
        "a transport error must not be confused with a missing-credential error, got: {stderr}"
    );
    assert!(
        !stderr.contains(&fake_key()),
        "the key must never appear in a transport diagnostic, got: {stderr}"
    );
    assert_no_backtrace(&stderr);
}

// VAL-RUN-008 (exit/diagnostic half): with the kill-switch engaged the run refuses with the
// dedicated net-disabled code and a clear, distinguishable diagnostic, writing nothing to stdout
// even when a key is present (fail-closed ordering).
#[test]
fn net_disabled_refuses_run_before_credential_with_clean_stdout() {
    let mut command = cmd(&["run", "hi"]);
    command.env("OPENROUTER_API_KEY", fake_key());
    command.env("CORTEX_NO_NET", "1");

    let out = run(command);

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "the kill-switch must exit with the dedicated net-disabled code"
    );
    assert!(out.stdout.is_empty(), "a refusal keeps stdout byte-empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("network disabled") && stderr.contains("CORTEX_NO_NET"),
        "stderr must read as a net-disabled error naming the kill-switch, got: {stderr}"
    );
    assert!(
        !stderr.contains("missing credential") && !stderr.contains("transport unavailable"),
        "net-disabled must be distinct from credential and transport errors, got: {stderr}"
    );
    assert_no_backtrace(&stderr);
}

/// Whether `strace` can trace a child here (it may be absent or `ptrace` may be restricted).
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

// VAL-RUN-008 (syscall half): under CORTEX_NO_NET=1, `cortex run` issues ZERO outbound connect()
// syscalls — the guard fails closed before any socket is created. Skips cleanly if strace is
// unavailable or ptrace is restricted.
#[test]
fn net_disabled_run_makes_zero_connect_syscalls() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }

    let trace_path = scratch_home().join("connect.trace");
    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=connect", "-o"])
        .arg(&trace_path)
        .arg(env!("CARGO_BIN_EXE_cortex"))
        .args(["run", "hi"])
        .env("HOME", scratch_home())
        .env("OPENROUTER_API_KEY", fake_key())
        .env("CORTEX_NO_NET", "1")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = command.output().expect("spawn strace");

    assert_eq!(
        out.status.code(),
        Some(NET_DISABLED_CODE),
        "the traced run must still fail closed with the net-disabled code"
    );
    assert!(
        out.stdout.is_empty(),
        "no answer on stdout under the kill-switch"
    );

    let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
    let connects = trace.matches("connect(").count();
    assert_eq!(
        connects, 0,
        "under CORTEX_NO_NET=1 `cortex run` must issue zero connect() syscalls:\n{trace}"
    );
}
