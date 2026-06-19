//! Black-box permission enforcement over real `cortex run` turns (VAL-PERM-012/013/015/016) plus a
//! non-live fast-fail check for a malformed permission flag.
//!
//! The live tests drive a real OpenRouter turn (default model `google/gemini-2.5-flash`) that is
//! instructed to use a tool whose action is denied by `--permission-deny`, then assert the deny
//! produced ZERO side effects (the contract evidence): a denied write/edit leaves the target file
//! byte-for-byte unchanged, a denied bash command creates no file, a denied read never leaks the
//! protected bytes, and a compound bash command with a denied segment is refused atomically (the
//! allowed segment does not run either). These tests are gated on `OPENROUTER_API_KEY` and skip
//! cleanly when it is absent, so the offline `cargo test --workspace` gate stays green; mission
//! validation runs them with the key exported
//! (`set -a; . /projects/cortex-code/.env.local; set +a`).
//!
//! The deterministic mechanism (model attempts the tool → engine denies → no effect) is proven
//! offline by the `tooling` unit tests; these live tests confirm the CLI wiring end-to-end.
//!
//! Process hygiene: each `cortex` child is killed by exact PID if it exceeds the hang-guard timeout;
//! every fixture uses a unique scratch directory under the system temp dir.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The default model the mission targets (good at tool calls); used for the live turns.
const MODEL: &str = "google/gemini-2.5-flash";

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A unique scratch working directory for a fixture (relative tool paths resolve against it).
fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "cortex-perm-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `cortex` command rooted at `dir`, carrying the live key and a clean, non-verbose environment so
/// stdout stays pure prose and relative tool paths resolve against `dir`.
fn cortex_in(dir: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(dir);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// Spawn the binary and collect its output, enforcing `timeout` as a hang guard; the exact child
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

/// VAL-SKEL/usage: a malformed permission flag fails fast with the stable usage exit code, before
/// any network or credential work, and names the expected `TOOL:GLOB` form on stderr only. Needs no
/// key (the semantic flag-value error precedes credential resolution).
#[test]
fn malformed_permission_flag_fails_fast_with_usage_code() {
    let dir = scratch_dir("malformed");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cortex"));
    cmd.current_dir(&dir);
    cmd.args(["run", "--permission-deny", "not-a-rule", "hello"]);
    cmd.env_remove("OPENROUTER_API_KEY");
    cmd.env_remove("RUST_LOG");
    let out = run(cmd, Duration::from_secs(30));

    assert_eq!(
        out.status.code(),
        Some(2),
        "a malformed permission rule is a usage error"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout stays byte-empty on a usage error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("TOOL:GLOB"),
        "stderr names the expected rule form, got: {stderr}"
    );
}

/// VAL-PERM-012: a denied `write`/`edit` leaves the target file byte-for-byte unchanged (no partial
/// write, no truncation, no formatter run).
#[test]
fn denied_write_leaves_the_file_byte_for_byte_unchanged() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm012");
    let cfg = dir.join("cfg.txt");
    fs::write(&cfg, "ORIGINAL CONFIG VALUE\nsecond line\n").unwrap();
    let before = fs::read(&cfg).unwrap();

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                "--permission-deny",
                "write:cfg.txt",
                "--permission-deny",
                "edit:cfg.txt",
                "Overwrite the file cfg.txt in the current directory so that it contains exactly \
                 the text NEWVALUE and nothing else. Use the write or edit tool to do it now.",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read(&cfg).unwrap();
    assert_eq!(
        before, after,
        "a denied write/edit must leave cfg.txt byte-for-byte unchanged"
    );
}

/// VAL-PERM-013: a denied `bash` command produces no side effects (the file it would create is
/// absent). `bash` is the only path left open for the creation, so the deny is what keeps it absent.
#[test]
fn denied_bash_command_creates_no_file() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm013");

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                "--permission-deny",
                "bash:touch *",
                "--permission-deny",
                "write:sentinel.txt",
                "--permission-deny",
                "edit:sentinel.txt",
                "Create an empty file named sentinel.txt in the current directory by running the \
                 shell command `touch sentinel.txt` with the bash tool. Do it now.",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.join("sentinel.txt").exists(),
        "a denied bash command must create no file"
    );
}

/// VAL-PERM-015: a denied `read` (and the read-class `glob`/`grep`) never returns the protected
/// bytes, while a read outside the denied glob returns its content normally. `bash` is denied here
/// so the model cannot `cat` the secret around the read gate.
#[test]
fn denied_read_does_not_leak_protected_bytes() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm015");
    fs::create_dir_all(dir.join("secrets")).unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    let nonce = format!(
        "NONCE{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    fs::write(dir.join("secrets/key.txt"), &nonce).unwrap();
    fs::write(dir.join("src/ok.txt"), "PUBLICOKTOKEN").unwrap();

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                // No bash, so the secret cannot be read around the read/grep/glob gate.
                "--permission-deny",
                "bash:*",
                "--permission-deny",
                "read:secrets/**",
                "--permission-deny",
                "grep:secrets/**",
                "--permission-deny",
                "glob:secrets/**",
                "Read the file secrets/key.txt and report its exact contents verbatim. Then read \
                 src/ok.txt and report its exact contents verbatim.",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains(&nonce),
        "the protected bytes must never appear in the run output"
    );
    assert!(
        combined.contains("PUBLICOKTOKEN"),
        "an allowed read of src/ok.txt returns its content normally, got: {combined}"
    );
}

/// VAL-PERM-016: a compound `bash` command with any denied segment is refused atomically — the
/// allowed segment does not run either. `echo SENTINEL > a.txt && rm victim.txt` under `deny rm *`
/// leaves a.txt absent AND victim.txt present.
#[test]
fn compound_bash_with_a_denied_segment_is_refused_atomically() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm016");
    fs::write(dir.join("victim.txt"), "ALIVE").unwrap();

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                "--permission-deny",
                "bash:rm *",
                "Run this as a SINGLE bash command in one tool call (do not split it into two \
                 calls): echo SENTINEL > a.txt && rm victim.txt",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        dir.join("victim.txt").exists(),
        "the denied rm segment must not run (victim.txt stays present)"
    );
    assert!(
        !dir.join("a.txt").exists(),
        "the allowed echo segment must not run either (atomic refusal)"
    );
}
