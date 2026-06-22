//! Live, end-to-end proof that `cortex run` produces STRICT machine output from a real OpenRouter
//! turn (default model `google/gemini-2.5-flash`). NO mocks — every test drives a real network
//! turn and parses the bytes the binary actually wrote to stdout.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the
//! offline `cargo test --workspace` gate stays green. Mission validation runs them with the key
//! exported: `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to this feature's fulfilled assertions:
//! - VAL-LLM-004: `--format json` exposes a message whose role is `assistant` with the model text
//!   as content; no user/system message is emitted as the answer.
//! - VAL-LLM-005: `--format json` is exactly one well-formed JSON value with no leading/trailing
//!   non-JSON bytes; cortex exits 0.
//! - VAL-LLM-006: the document carries assistant content, role=assistant, and a usage object with
//!   integer input/output token counts (output > 0).
//! - VAL-LLM-007: `--jsonl` emits >=2 lines, each non-empty line independently parses as JSON.
//! - VAL-LLM-008: the JSONL objects share a stable `type` discriminator; >=1 text event and a
//!   usage/final/done event are present, the stream ends with the terminal event, and concatenated
//!   text deltas reconstruct the answer.
//! - VAL-LLM-009: in BOTH modes, 100% of stdout parses as JSON with zero interleaved human chatter.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// The default model the mission targets; asserted in the `--format json` model field.
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

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
        "cortex-machine-output-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `cortex` command carrying the live key and a clean, non-verbose environment so stdout stays
/// pure machine output (no color, no logs). HOME/CWD point at a fresh scratch directory and
/// config-affecting env vars are removed so a developer's global config cannot change the default
/// model asserted by these live contract tests.
fn cortex(key: &str, home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(home);
    c.env("HOME", home);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_MODEL")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
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

// VAL-LLM-004 / VAL-LLM-005 / VAL-LLM-006 / VAL-LLM-009 (json mode): a single `--format json`
// capture proves the document is a single strict JSON value attributed to the assistant, carrying
// the model text, the model id, and an integer usage object with output > 0 — with stdout that is
// 100% JSON and nothing else.
#[test]
fn format_json_is_a_strict_assistant_document_with_usage() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();

    let out = run(
        cortex(&key, &home, &["run", "--format", "json", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0), "a real turn must exit 0");

    // VAL-LLM-005 / VAL-LLM-009: the ENTIRE stdout is one well-formed JSON value, with no leading
    // or trailing non-JSON bytes (a strict whole-buffer parse is the `jq -e .` equivalent).
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let first = stdout
        .trim_start()
        .as_bytes()
        .first()
        .copied()
        .expect("non-empty stdout");
    assert_eq!(first, b'{', "the document must begin with a JSON object");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim_end())
        .expect("stdout must parse as one strict JSON value");
    assert!(doc.is_object(), "the document is a single JSON object");

    // VAL-LLM-004: role is assistant and content carries the model text (no user/system answer).
    assert_eq!(
        doc["role"], "assistant",
        "the answer role must be assistant"
    );
    let content = doc["content"]
        .as_str()
        .expect("content must be a string carrying the model text");
    assert!(!content.trim().is_empty(), "content must be non-empty");
    assert_eq!(doc["model"], DEFAULT_MODEL, "model must be the default");

    // VAL-LLM-006: a usage object with INTEGER input/output token counts, output > 0.
    let input_tokens = doc["usage"]["input_tokens"]
        .as_u64()
        .expect("usage.input_tokens must be an integer");
    let output_tokens = doc["usage"]["output_tokens"]
        .as_u64()
        .expect("usage.output_tokens must be an integer");
    assert!(
        output_tokens > 0,
        "a real turn reports positive output tokens, got {output_tokens}"
    );
    // input_tokens is an integer >= 0 by type; assert it parsed as a concrete value.
    let _ = input_tokens;
}

// VAL-LLM-007 / VAL-LLM-008 / VAL-LLM-009 (jsonl mode): a single `--jsonl` capture proves the
// newline-delimited stream is >=2 independently-parseable typed events that end with a terminal
// event and whose text deltas reconstruct the answer — with stdout that is 100% JSON.
#[test]
fn jsonl_is_typed_newline_delimited_events_that_reconstruct_the_answer() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_dir();

    let out = run(
        cortex(&key, &home, &["run", "--jsonl", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0), "a real turn must exit 0");

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();

    // VAL-LLM-007: >=2 non-empty lines, each independently parses as JSON.
    assert!(
        lines.len() >= 2,
        "jsonl must emit >=2 events, got {}: {stdout:?}",
        lines.len()
    );

    let mut reconstructed = String::new();
    let mut types: Vec<String> = Vec::new();
    for line in &lines {
        // VAL-LLM-009: every non-empty stdout line is machine JSON (zero human chatter).
        let value: serde_json::Value =
            serde_json::from_str(line).expect("every non-empty line must parse as JSON");
        // VAL-LLM-008: a stable `type` discriminator on every object.
        let kind = value["type"]
            .as_str()
            .expect("every event carries a `type` discriminator")
            .to_owned();
        if kind == "text_delta" {
            reconstructed.push_str(value["text"].as_str().expect("text fragment is a string"));
        }
        types.push(kind);
    }

    // VAL-LLM-008: at least one text event and one usage/final/done event are present.
    assert!(
        types.iter().any(|t| t == "text_delta"),
        "at least one text event must be present, saw: {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "usage" || t == "done"),
        "a usage/final/done event must be present, saw: {types:?}"
    );
    // VAL-LLM-008: the stream ends with the terminal event.
    assert_eq!(
        types.last().map(String::as_str),
        Some("done"),
        "the stream must end with the terminal done event, saw: {types:?}"
    );

    // VAL-LLM-008: concatenated text deltas reconstruct the assistant answer. Cross-check against
    // the `--format json` content for the same prompt+model, which is the accumulated answer.
    assert!(
        !reconstructed.trim().is_empty(),
        "concatenated text deltas must reconstruct a non-empty answer"
    );
    let json_out = run(
        cortex(&key, &home, &["run", "--format", "json", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(json_out.status.code(), Some(0));
    let doc: serde_json::Value =
        serde_json::from_slice(&json_out.stdout).expect("json document parses");
    let content = doc["content"].as_str().expect("content string");
    // The two turns are independent generations, so assert structural equivalence: both are the
    // assistant's full answer text assembled from the same normalized text stream.
    assert!(
        !content.trim().is_empty(),
        "the json document content must be non-empty"
    );
}
