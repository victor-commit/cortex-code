//! Black-box checks for `cortex run` stdout/stderr hygiene AFTER the M0→M1 transition.
//!
//! `cortex run` now performs a real OpenRouter turn, so the M0 echo-path machine-output
//! assertions (VAL-SKEL-057..064) no longer apply to a successful run; the strict structure of a
//! successful `--format json`/`--jsonl` document is proven against the live path (see
//! `run_live.rs`) and governed by VAL-LLM-005..010.
//!
//! What survives the transition — and is deterministically testable WITHOUT a network turn — is
//! the hygiene invariant: on a failure, every output format keeps stdout byte-empty (no partial
//! document, no banners, no ANSI, no log prefixes), routing the human diagnostic to stderr only.
//! These tests exercise that via the `CORTEX_NO_NET` fail-closed refusal, which is fully local.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(10);

/// The dedicated exit code for a kill-switch refusal (`Exit::NetDisabled`).
const NET_DISABLED_CODE: i32 = 4;

/// The three output-format selections `cortex run` accepts.
const FORMAT_ARGS: [&[&str]; 3] = [
    &["run", "payload here"],
    &["run", "--format", "json", "payload here"],
    &["run", "--jsonl", "payload here"],
];

/// A hermetic `cortex run` invocation under the engaged kill-switch: a dummy key is present (so
/// the refusal is the net-disabled path, not no-credential) and HOME/CWD point at a fresh empty
/// temp dir so no ambient config interferes.
fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(scratch_home());
    c.env("HOME", scratch_home());
    c.env("CORTEX_NO_NET", "1");
    c.env("OPENROUTER_API_KEY", "sk-or-v1-not-a-real-key");
    c.env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-fmt-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// Run the binary with a hard timeout, draining stdout/stderr on dedicated threads so an
/// undrained pipe can never deadlock the child.
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

// VAL-SKEL-057/058 (preserved as a failure-path invariant): no output format ever writes a
// partial document or any chatter to stdout on failure — stdout stays byte-empty while the
// diagnostic goes to stderr only.
#[test]
fn every_format_keeps_stdout_empty_on_failure() {
    for args in FORMAT_ARGS {
        let out = run(cmd(args));
        assert_eq!(
            out.status.code(),
            Some(NET_DISABLED_CODE),
            "{args:?} must fail closed under the kill-switch"
        );
        assert!(
            out.stdout.is_empty(),
            "{args:?} must keep stdout byte-empty on failure, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(
            !out.stderr.is_empty(),
            "{args:?} must route its diagnostic to stderr"
        );
    }
}

// VAL-SKEL-061/063 (preserved): no output format emits ANSI escapes onto stdout when piped, and
// the human diagnostic on stderr is not colorized either.
#[test]
fn no_ansi_escapes_in_piped_output() {
    for args in FORMAT_ARGS {
        let out = run(cmd(args));
        assert!(
            !out.stdout.contains(&0x1b),
            "{args:?} stdout must contain no ESC/ANSI byte"
        );
        assert!(
            !out.stderr.contains(&0x1b),
            "{args:?} stderr diagnostic must not be colorized"
        );
    }
}

// VAL-SKEL-064 (preserved): no debug/log prefix leaks onto stdout on a default (non-verbose) run;
// stdout carries only intended payload (here: nothing, because the run was refused).
#[test]
fn no_log_prefixes_leak_onto_stdout() {
    for args in FORMAT_ARGS {
        let out = run(cmd(args));
        let stdout = String::from_utf8_lossy(&out.stdout);
        for prefix in ["INFO", "DEBUG", "WARN", "ERROR", "TRACE"] {
            assert!(
                !stdout.contains(prefix),
                "{args:?}: no `{prefix}` log prefix may leak onto stdout, got: {stdout:?}"
            );
        }
    }
}
