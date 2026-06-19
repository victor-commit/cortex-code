//! Live, end-to-end proof of `cortex run` model selection (architecture.md §7). NO mocks — every
//! valid-model test drives a real OpenRouter turn and inspects the bytes the binary actually wrote.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the
//! offline `cargo test --workspace` gate stays green. Mission validation runs them with the key
//! exported: `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to this feature's fulfilled assertions:
//! - VAL-LLM-011: no `--model` and no config override reports the default `google/gemini-2.5-flash`.
//! - VAL-LLM-012: `--model google/gemini-2.5-flash` exits 0 and the JSON model field equals it.
//! - VAL-LLM-013: a second, distinct valid model is honored, proving selection is not hard-wired
//!   to the default.
//! - VAL-LLM-014: a malformed model id (`not-a-real-model`, not a `provider/model` pair) fails
//!   fast with an actionable error naming it, a non-zero exit, no panic/backtrace, and no
//!   assistant answer on stdout.
//! - VAL-LLM-029: a config-file `model` is honored with no flag, and `--model` overrides it; both
//!   turns exit 0.
//!
//! NOTE on the second model: the validation contract's VAL-LLM-013 example,
//! `anthropic/claude-3.5-sonnet`, has since been removed from OpenRouter (a direct API call
//! returns HTTP 404 "No endpoints found"). To keep this proof green against the live provider we
//! exercise `anthropic/claude-sonnet-4` instead — a currently-available, embedded-catalogue model
//! that equally proves selection is not hard-wired to the default. The drift is flagged to the
//! orchestrator so the contract's literal model can be refreshed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The mission default model, expected when no flag and no config override select another.
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// A second, distinct valid OpenRouter model used to prove selection is not hard-wired. (The
/// contract's original `anthropic/claude-3.5-sonnet` is no longer served by OpenRouter; see the
/// module note.)
const SECOND_MODEL: &str = "anthropic/claude-sonnet-4";

/// A malformed model id (not a `provider/model` pair): rejected locally before any credential or
/// socket. Per the validation contract's VAL-LLM-014 example. (A bare `gemini-3.5-flash` is no
/// longer a safe "invalid" example: `google/gemini-3.5-flash` is now served by OpenRouter.)
const INVALID_MODEL: &str = "not-a-real-model";

/// Generous per-turn upper bound: a healthy single turn finishes well within this; exceeding it is
/// treated as a hang and fails the test loudly instead of blocking.
const TURN_TIMEOUT: Duration = Duration::from_secs(90);

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A unique, freshly created empty temp directory used as a hermetic HOME/CWD.
fn scratch_dir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-model-sel-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `cortex` command carrying the live key and a clean environment: HOME points at a fresh empty
/// scratch dir and every config-affecting env override is cleared, so no stray global/project
/// config or `CORTEX_MODEL` can perturb model selection. The caller sets exactly what it tests.
fn cortex(key: &str, home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(home);
    c.env("HOME", home);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_MODEL")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
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

/// Parse stdout as exactly one strict JSON document, failing with the raw bytes on a parse error.
fn parse_json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout must be a single strict JSON document ({err}): {:?}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

// VAL-LLM-011: with no `--model` flag and no config override, the turn reports the default model
// `google/gemini-2.5-flash` and exits 0.
#[test]
fn default_model_is_reported_when_unspecified() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();
    let out = run(
        cortex(&key, &home, &["run", "--format", "json", "Say hi"]),
        TURN_TIMEOUT,
    );
    assert_eq!(out.status.code(), Some(0), "the default turn must exit 0");
    let doc = parse_json(&out);
    assert_eq!(
        doc["model"], DEFAULT_MODEL,
        "the default model must be reported, got: {doc}"
    );
}

// VAL-LLM-012: `--model google/gemini-2.5-flash` exits 0 and the JSON model field equals the
// explicitly-requested default model.
#[test]
fn explicit_default_model_is_honored() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();
    let out = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--model",
                DEFAULT_MODEL,
                "--format",
                "json",
                "Say hi",
            ],
        ),
        TURN_TIMEOUT,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "an explicit default must exit 0"
    );
    let doc = parse_json(&out);
    assert_eq!(
        doc["model"], DEFAULT_MODEL,
        "the JSON model field must equal the requested model, got: {doc}"
    );
}

// VAL-LLM-013: a second, distinct valid model is selectable via `--model` and is honored, proving
// selection is not hard-wired to the default.
#[test]
fn second_valid_model_is_honored() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();
    let out = run(
        cortex(
            &key,
            &home,
            &["run", "--model", SECOND_MODEL, "--format", "json", "Say hi"],
        ),
        TURN_TIMEOUT,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a second valid model must exit 0"
    );
    let doc = parse_json(&out);
    assert_eq!(
        doc["model"], SECOND_MODEL,
        "the JSON model field must equal the second requested model, got: {doc}"
    );
}

// VAL-LLM-014: a non-existent model fails fast — non-zero exit, stderr names the offending model
// and reads as unknown/not found/invalid, stdout carries no assistant answer, and the output
// contains no panic/backtrace text.
#[test]
fn invalid_model_fails_fast_without_a_panic() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();
    let out = run(
        cortex(&key, &home, &["run", "--model", INVALID_MODEL, "Say hi"]),
        TURN_TIMEOUT,
    );

    assert_ne!(
        out.status.code(),
        Some(0),
        "an invalid model must fail with a non-zero exit"
    );
    assert!(
        out.stdout.is_empty(),
        "no assistant answer may reach stdout for an invalid model, got: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let lower = stderr.to_lowercase();
    assert!(
        stderr.contains(INVALID_MODEL),
        "stderr must name the offending model {INVALID_MODEL:?}, got: {stderr}"
    );
    assert!(
        lower.contains("not found")
            || lower.contains("not a valid")
            || lower.contains("invalid")
            || lower.contains("unknown")
            || lower.contains("no endpoints")
            || lower.contains("no allowed providers")
            || lower.contains("does not exist"),
        "stderr must read as an unknown/not-found/invalid model error, got: {stderr}"
    );
    assert!(
        !lower.contains("panicked")
            && !stderr.contains("RUST_BACKTRACE")
            && !lower.contains("backtrace"),
        "an invalid model must not panic or print a backtrace, got: {stderr}"
    );
}

// VAL-LLM-029: a config-file `model` is honored when no flag is given, and an explicit `--model`
// overrides the same config; both turns exit 0.
#[test]
fn config_model_is_honored_and_flag_overrides_it() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    // The project-config walk runs from the CWD up to HOME (exclusive), so the `cortex.toml` must
    // live in a sub-directory of HOME that we then run from.
    let home = scratch_dir();
    let proj = home.join("proj");
    std::fs::create_dir_all(&proj).expect("create project dir");
    std::fs::write(
        proj.join("cortex.toml"),
        format!("model = \"{SECOND_MODEL}\"\n"),
    )
    .expect("write project config");

    // No flag: the config-file model drives (and is reported by) the turn.
    let mut from_config_cmd = cortex(&key, &home, &["run", "--format", "json", "Say hi"]);
    from_config_cmd.current_dir(&proj);
    let from_config = run(from_config_cmd, TURN_TIMEOUT);
    assert_eq!(
        from_config.status.code(),
        Some(0),
        "the config-driven turn must exit 0"
    );
    let config_doc = parse_json(&from_config);
    assert_eq!(
        config_doc["model"], SECOND_MODEL,
        "a config-file model must be honored without a flag, got: {config_doc}"
    );

    // Same turn, now with `--model`: the flag overrides the config-file model.
    let mut from_flag_cmd = cortex(
        &key,
        &home,
        &[
            "run",
            "--model",
            DEFAULT_MODEL,
            "--format",
            "json",
            "Say hi",
        ],
    );
    from_flag_cmd.current_dir(&proj);
    let from_flag = run(from_flag_cmd, TURN_TIMEOUT);
    assert_eq!(
        from_flag.status.code(),
        Some(0),
        "the flag-overridden turn must exit 0"
    );
    let flag_doc = parse_json(&from_flag);
    assert_eq!(
        flag_doc["model"], DEFAULT_MODEL,
        "an explicit --model must override the config-file model, got: {flag_doc}"
    );
}
