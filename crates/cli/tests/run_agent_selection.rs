//! `cortex run --agent <name>` persona selection. Two halves, mapped to the validation contract:
//!
//! - **VAL-LLM-028** (offline, network-free): an unknown agent name fails fast — the run exits
//!   non-zero, names the unknown agent on stderr with an availability hint, writes no assistant
//!   answer to stdout, and never panics. This path is rejected before any credential or socket, so
//!   it needs no key and runs on the offline `cargo test` gate.
//! - **VAL-LLM-027** (live, real OpenRouter turn): a valid agent is accepted, the machine output
//!   reflects the selected agent (`agent` field), and the persona observably governs behavior — a
//!   `plan` turn produces a plan/outline rather than claiming to make edits, and a second distinct
//!   agent (`build`) is also accepted and reflected. Gated on `OPENROUTER_API_KEY`; skips cleanly
//!   when unset so the offline gate stays green. NO mocks.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The mission default model, asserted in the `--format json` model field on live turns.
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// Generous bound for a live turn; the offline unknown-agent path fails near-instantly.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// A unique, freshly created empty temp directory used as a hermetic HOME/CWD so no global config
/// or on-disk auth store leaks into a run.
fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-agent-sel-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// Resolve the live OpenRouter key, or `None` to skip the live tests cleanly when it is unset.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A `cortex` invocation in a hermetic environment with NO credential available anywhere (used for
/// the offline unknown-agent path, which must fail before any credential check).
fn keyless(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    let home = scratch_home();
    c.current_dir(&home);
    c.env("HOME", &home);
    c.env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// A `cortex` invocation carrying the live key plus a hermetic data dir and clean, non-verbose
/// environment so stdout stays pure machine output.
fn cortex(key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    let home = scratch_home();
    c.current_dir(&home);
    c.env("HOME", &home);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// Run the binary with a hard timeout; stdout/stderr are drained on dedicated threads so a child
/// can never deadlock against an undrained pipe. The exact child PID is killed on timeout.
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

// VAL-LLM-028: an unknown agent name fails fast — non-zero exit, stderr names the unknown agent
// AND hints what is available, stdout carries no assistant answer, and there is no panic/backtrace.
// This holds with no key present because the rejection precedes any credential or socket.
#[test]
fn unknown_agent_fails_fast_naming_the_agent_with_no_answer() {
    let out = run(keyless(&["run", "--agent", "no-such-agent", "hi"]));

    assert_ne!(
        out.status.code(),
        Some(0),
        "an unknown agent must fail with a non-zero exit"
    );
    assert!(
        out.stdout.is_empty(),
        "no assistant answer may be written for an unknown agent, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no-such-agent"),
        "stderr must name the unknown agent, got: {stderr}"
    );
    assert!(
        stderr.contains("plan") && stderr.contains("build") && stderr.contains("general"),
        "stderr should hint the available agents, got: {stderr}"
    );
    assert_no_backtrace(&stderr);
}

// The unknown-agent rejection is deterministic regardless of output format: `--format json` also
// fails fast with empty stdout (no partial JSON document leaks).
#[test]
fn unknown_agent_in_json_mode_leaves_stdout_empty() {
    let out = run(keyless(&[
        "run", "--agent", "bogus", "--format", "json", "hi",
    ]));
    assert_ne!(out.status.code(), Some(0));
    assert!(
        out.stdout.is_empty(),
        "no partial JSON may be written for an unknown agent, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("bogus"), "stderr must name the agent");
    assert_no_backtrace(&stderr);
}

// VAL-LLM-027 (plan persona): a real OpenRouter turn with `--agent plan` exits 0, the JSON reflects
// the selected agent, and the persona governs behavior — the model produces a plan/outline rather
// than claiming to have made edits.
#[test]
fn plan_agent_is_reflected_and_governs_behavior() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(cortex(
        &key,
        &[
            "run",
            "--agent",
            "plan",
            "--format",
            "json",
            "Outline, do not implement, a hello-world program in Python.",
        ],
    ));
    assert_eq!(out.status.code(), Some(0), "a real plan turn must exit 0");

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value =
        serde_json::from_str(stdout.trim_end()).expect("stdout must be one strict JSON value");
    assert_eq!(doc["role"], "assistant");
    assert_eq!(
        doc["agent"], "plan",
        "the document must reflect the selected agent"
    );
    assert_eq!(doc["model"], DEFAULT_MODEL, "model must be the default");

    let content = doc["content"]
        .as_str()
        .expect("content carries the model text")
        .to_lowercase();
    assert!(!content.trim().is_empty(), "content must be non-empty");
    // The plan persona must not claim to have performed edits/writes; it describes steps instead.
    for claim in [
        "i have created",
        "i've created",
        "i have written",
        "i've written",
        "i have implemented",
        "i've implemented",
        "i created the file",
        "i wrote the file",
    ] {
        assert!(
            !content.contains(claim),
            "the plan persona must not claim to make edits, found '{claim}' in: {content}"
        );
    }
}

// VAL-LLM-027 (second distinct agent): `--agent build` is also accepted over a real turn and the
// machine output reflects it, proving selection is not hard-wired to one persona.
#[test]
fn build_agent_is_accepted_and_reflected() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(cortex(
        &key,
        &[
            "run",
            "--agent",
            "build",
            "--format",
            "json",
            "Say hi in one short sentence.",
        ],
    ));
    assert_eq!(out.status.code(), Some(0), "a real build turn must exit 0");

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value =
        serde_json::from_str(stdout.trim_end()).expect("stdout must be one strict JSON value");
    assert_eq!(doc["role"], "assistant");
    assert_eq!(
        doc["agent"], "build",
        "the document must reflect the build agent"
    );
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|c| !c.trim().is_empty()),
        "a real turn must produce non-empty content"
    );
}

// The leading `--jsonl` session event surfaces the selected agent so a streaming consumer can read
// it before the runner's own events; proven over a real turn.
#[test]
fn jsonl_session_event_carries_the_selected_agent() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(cortex(
        &key,
        &["run", "--agent", "general", "--jsonl", "Say hi"],
    ));
    assert_eq!(out.status.code(), Some(0), "a real turn must exit 0");

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let first = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("at least one event line");
    let event: serde_json::Value =
        serde_json::from_str(first).expect("the leading line is a JSON object");
    assert_eq!(event["type"], "session");
    assert_eq!(
        event["agent"], "general",
        "the leading session event must carry the selected agent"
    );
}
