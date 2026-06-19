//! Live, end-to-end proof of per-turn and per-session usage/cost reporting for `cortex run`,
//! driven by REAL OpenRouter turns (default model `google/gemini-2.5-flash`). NO mocks: token
//! counts and cost come from the provider's own usage accounting, parsed from the bytes the binary
//! actually wrote to stdout.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the offline
//! `cargo test --workspace` gate stays green. Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to this feature's fulfilled assertions:
//! - VAL-LLM-020: a completed turn reports integer input/output token counts (>= 1) that vary with
//!   prompt/response size (not a constant placeholder).
//! - VAL-LLM-021: a completed turn reports a non-negative monetary cost that is positive for a paid
//!   model, and a substantially longer turn reports a larger cost than a one-word turn.
//! - VAL-LLM-031: with a fixed `--session`, the session-level usage/cost total after turn 2 is
//!   strictly greater than after turn 1 and is consistent with the accumulated per-turn token
//!   counts; both turns exit 0.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Hang guard for a single live turn; a real turn completes well within this.
const TURN_TIMEOUT: Duration = Duration::from_secs(90);

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A unique, freshly created empty temp directory used as a hermetic HOME + data-dir root, so each
/// test's sessions are fully isolated from other tests and from any real user data.
fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-usage-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// A `cortex` command bound to a specific hermetic `home` (so repeated invocations in one test share
/// one database), carrying the live key and a clean, non-verbose environment so stdout is pure JSON.
fn cortex(key: &str, home: &PathBuf, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(home);
    c.env("HOME", home);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_MODEL")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// Spawn the binary and collect its output under a hard timeout. The exact child PID is killed if
/// the timeout is exceeded so no orphan is left behind.
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
        if start.elapsed() > TURN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {TURN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

/// Parse a `--format json` turn's stdout into a JSON document, asserting a clean exit 0 first.
fn json_doc(out: Output, what: &str) -> serde_json::Value {
    assert_eq!(out.status.code(), Some(0), "{what} must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("stdout must be valid UTF-8");
    serde_json::from_str(stdout.trim()).expect("stdout must be one strict JSON document")
}

/// A sufficiently-unique session id, mirroring the contract's `val-$(uuidgen)` pattern without a
/// uuid dependency.
fn unique_session_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!(
        "val-{nanos:x}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A long prompt that reliably elicits a much larger response than a one-word turn, so output
/// tokens and cost are substantially higher than a `Say hi` turn.
const LONG_PROMPT: &str = "Write a detailed, well-structured essay of at least 600 words about the \
    history of timekeeping, from sundials to atomic clocks. Use several paragraphs.";

// VAL-LLM-020 / VAL-LLM-021: a short, one-word turn and a substantially longer turn each report
// integer token counts (>= 1) and a numeric cost; the longer turn's tokens and cost are strictly
// larger, proving the figures track prompt/response size rather than being a constant placeholder.
#[test]
fn per_turn_usage_and_cost_track_turn_size() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    let short = json_doc(
        run(cortex(&key, &home, &["run", "--format", "json", "Say hi"])),
        "the short turn",
    );
    let long = json_doc(
        run(cortex(
            &key,
            &home,
            &["run", "--format", "json", LONG_PROMPT],
        )),
        "the long turn",
    );

    // VAL-LLM-020: integer input/output token counts, each >= 1, on both turns.
    let short_in = short["usage"]["input_tokens"]
        .as_u64()
        .expect("short input_tokens is an integer");
    let short_out = short["usage"]["output_tokens"]
        .as_u64()
        .expect("short output_tokens is an integer");
    let long_in = long["usage"]["input_tokens"]
        .as_u64()
        .expect("long input_tokens is an integer");
    let long_out = long["usage"]["output_tokens"]
        .as_u64()
        .expect("long output_tokens is an integer");
    assert!(short_in >= 1, "short input tokens >= 1, got {short_in}");
    assert!(short_out >= 1, "short output tokens >= 1, got {short_out}");
    assert!(long_in >= 1, "long input tokens >= 1, got {long_in}");
    assert!(long_out >= 1, "long output tokens >= 1, got {long_out}");

    // VAL-LLM-020: the values vary with size — the long turn produces many more output tokens than
    // a one-word turn (not a constant placeholder).
    assert!(
        long_out > short_out,
        "a substantially longer turn must report more output tokens \
         (long={long_out}, short={short_out})"
    );

    // VAL-LLM-021: cost is a number >= 0 and positive for this paid model.
    let short_cost = short["usage"]["cost"]
        .as_f64()
        .expect("short cost is a number");
    let long_cost = long["usage"]["cost"]
        .as_f64()
        .expect("long cost is a number");
    assert!(
        short_cost >= 0.0,
        "cost must be non-negative, got {short_cost}"
    );
    assert!(
        short_cost > 0.0,
        "a paid model reports a positive cost, got {short_cost}"
    );

    // VAL-LLM-021: a substantially longer turn costs more than a one-word turn.
    assert!(
        long_cost > short_cost,
        "a substantially longer turn must cost more (long={long_cost}, short={short_cost})"
    );
}

// VAL-LLM-031: with a fixed `--session`, the session-level usage/cost total after turn 2 is
// strictly greater than after turn 1 and is consistent (within reporting tolerance) with the
// accumulated per-turn token counts; both turns exit 0 and the total is never reset to a single
// turn's value.
#[test]
fn session_usage_accumulates_across_turns() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();
    let session = unique_session_id();

    let first = json_doc(
        run(cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "--format",
                "json",
                "Remember the word: marmalade. Acknowledge briefly.",
            ],
        )),
        "session turn 1",
    );
    let second = json_doc(
        run(cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "--format",
                "json",
                "Now write two sentences using that word in different contexts.",
            ],
        )),
        "session turn 2",
    );

    // Per-turn usage for each turn (the building blocks of the session total).
    let turn1_total = first["usage"]["total_tokens"]
        .as_u64()
        .expect("turn 1 total_tokens");
    let turn2_total = second["usage"]["total_tokens"]
        .as_u64()
        .expect("turn 2 total_tokens");

    // The session-level cumulative total is reported on each turn.
    let after1 = first["session_usage"]["total_tokens"]
        .as_u64()
        .expect("session_usage.total_tokens after turn 1");
    let after2 = second["session_usage"]["total_tokens"]
        .as_u64()
        .expect("session_usage.total_tokens after turn 2");

    // After turn 1 the session total equals turn 1's usage (it is not empty, and not double).
    assert_eq!(
        after1, turn1_total,
        "after turn 1 the session total equals turn 1's per-turn total"
    );

    // VAL-LLM-031: the session total after turn 2 is strictly greater than after turn 1, and is
    // never reset to a single turn's value.
    assert!(
        after2 > after1,
        "the session total must grow across turns (after1={after1}, after2={after2})"
    );
    assert!(
        after2 > turn2_total,
        "the session total must exceed a single turn's value \
         (after2={after2}, turn2={turn2_total})"
    );

    // VAL-LLM-031: the session total is consistent with the accumulated per-turn token counts.
    assert_eq!(
        after2,
        turn1_total + turn2_total,
        "the session total equals the sum of the per-turn totals \
         (turn1={turn1_total}, turn2={turn2_total}, after2={after2})"
    );

    // Cumulative cost also grows monotonically across the two turns and is positive (paid model).
    let cost1 = first["session_usage"]["cost"]
        .as_f64()
        .expect("session cost after turn 1");
    let cost2 = second["session_usage"]["cost"]
        .as_f64()
        .expect("session cost after turn 2");
    assert!(cost1 > 0.0, "a paid session reports a positive cost");
    assert!(
        cost2 > cost1,
        "cumulative session cost must grow across turns (cost1={cost1}, cost2={cost2})"
    );
}
