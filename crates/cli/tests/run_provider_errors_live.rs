//! Live, end-to-end proof for the upstream-auth and secret-hygiene surfaces of `cortex run`.
//!
//! These tests require a live OpenRouter environment and are gated on `OPENROUTER_API_KEY`; they
//! skip cleanly when it is absent, so the offline `cargo test --workspace` gate stays green.
//! Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage:
//! - **VAL-LLM-023**: an invalid/garbage key produces a normalized upstream auth/provider failure
//!   (e.g. unauthorized/401) on stderr, with a non-zero exit, no panic, and no partial answer.
//!   The real key is NOT used here — an intentionally invalid key drives the 401.
//! - **VAL-LLM-024**: running with the REAL key (verbose `-vv`) never leaks the key's literal
//!   value into stdout or stderr; redaction is active on diagnostics/log output.

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Resolve the live OpenRouter key, or `None` to skip cleanly when it is unset/empty. Its presence
/// also signals a live environment (network + a working OpenRouter account) for the invalid-key
/// test, which itself uses a different, intentionally-invalid key.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// An intentionally-invalid, key-shaped value assembled from fragments (so no contiguous
/// key-shaped literal lives in source) — used to drive a real upstream 401.
fn invalid_key() -> String {
    format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
}

/// A `cortex` command carrying the given key and a clean environment (no kill-switch, no endpoint
/// override). Verbosity is left to the caller so the no-leak test can opt into `-vv`.
fn cortex(key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// Spawn the binary and collect its output, enforcing `timeout` as a hang guard. The exact child
/// PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    use std::io::Read;
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

// VAL-LLM-023: an invalid/garbage key is surfaced as a normalized upstream auth/provider failure
// (unauthorized/401) on stderr, with a non-zero exit, no panic, and no partial answer on stdout.
#[test]
fn invalid_key_surfaces_a_normalized_upstream_auth_error() {
    if live_key().is_none() {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    }

    let bad = invalid_key();
    let out = run(cortex(&bad, &["run", "hi"]), Duration::from_secs(60));

    assert_ne!(
        out.status.code(),
        Some(0),
        "an invalid key must fail the run"
    );
    assert_ne!(
        out.status.code(),
        Some(4),
        "an upstream auth error is not a net-disabled refusal"
    );
    assert!(
        out.stdout.is_empty(),
        "no partial answer may be written on an upstream auth failure, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("upstream") || lower.contains("401") || lower.contains("unauthorized"),
        "stderr must convey a normalized upstream auth/provider failure, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "an upstream error must not panic, got: {stderr}"
    );
    assert!(
        !stderr.contains(&bad),
        "the (invalid) key must never appear verbatim in the diagnostic, got: {stderr}"
    );
}

// VAL-LLM-024: running with the REAL key (verbose `-vv`) never leaks the key's literal value into
// stdout or stderr. A successful turn is exercised so any logging/diagnostic path that could
// surface the key is in play; redaction keeps it out.
#[test]
fn real_key_never_leaks_into_stdout_or_stderr_even_verbose() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(
        cortex(&key, &["-vv", "run", "Say hi in one short sentence."]),
        Duration::from_secs(60),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a real verbose turn must succeed, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains(key.as_str()),
        "the API key must never appear in stdout"
    );
    assert!(
        !stderr.contains(key.as_str()),
        "the API key must never appear in stderr (verbose/log output included)"
    );
    // The verbose run must have produced an actual answer (proving the turn ran, not just failed
    // before any key-touching code path).
    assert!(
        !stdout.trim().is_empty(),
        "the verbose turn must produce an assistant answer on stdout"
    );
}
