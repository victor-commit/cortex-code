//! Live, end-to-end proof of automatic compaction and overflow recovery for `cortex run`, driven
//! by REAL OpenRouter turns (default model `google/gemini-2.5-flash`). NO mocks: the runner sends
//! real (compacted) requests and the assertions inspect the bytes the binary actually wrote.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the offline
//! `cargo test --workspace` gate stays green. Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! To keep the live cost and latency bounded and deterministic, both tests pin the effective
//! context window low via `CORTEX_CONTEXT_LIMIT_TOKENS` so compaction provably engages on inputs
//! that comfortably exceed it. The same code path runs with the model's real (huge) window when the
//! override is absent; the override only changes the budget, not the behavior.
//!
//! Coverage maps to this feature's fulfilled assertions:
//! - VAL-LLM-025: a single very large piped input does not crash or hang; the runner compacts the
//!   over-large turn and still returns a coherent final answer containing `done`, with no panic.
//! - VAL-LLM-026: with a fixed `--session`, ~10+ successive prompts each adding sizable text keep
//!   answering; the final follow-up returns a valid assistant answer (exit 0) without a
//!   context-length crash, demonstrating automatic compaction of accumulated history.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Hang guard for a single live turn; a real (compacted) turn completes well within this.
const TURN_TIMEOUT: Duration = Duration::from_secs(90);

/// A low effective context window so compaction provably engages on the test inputs while keeping
/// each real turn cheap and fast.
const CONTEXT_LIMIT: &str = "6000";

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
        "cortex-compaction-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// A `cortex` command bound to a hermetic `home` (so repeated invocations in one test share one
/// database), carrying the live key, the low context-window override, and a clean, non-verbose
/// environment so stdout is pure result output.
fn cortex(key: &str, home: &PathBuf, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.current_dir(home);
    c.env("HOME", home);
    c.env("OPENROUTER_API_KEY", key);
    c.env("CORTEX_CONTEXT_LIMIT_TOKENS", CONTEXT_LIMIT);
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

/// Spawn the binary, optionally feed `stdin_data`, and collect its output under a hard timeout. The
/// exact child PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, stdin_data: Option<&[u8]>) -> Output {
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
        let data = data.to_vec();
        // Write stdin on a thread so a large pipe cannot deadlock against the child's output.
        std::thread::spawn(move || {
            let _ = handle.write_all(&data);
            // Drop closes stdin (EOF) so the child stops reading.
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

/// Assert the run produced no Rust panic/backtrace on stderr.
fn assert_no_panic(stderr: &str, what: &str) {
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "{what} must not panic; stderr was:\n{stderr}"
    );
}

fn parse_json(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).expect("valid JSON output")
}

fn parse_jsonl_lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("valid JSONL line"))
        .collect()
}

// VAL-LLM-025: a single very large piped input combined with the instruction argument does not
// crash or hang. The runner compacts the over-large turn down to the context budget and still
// returns a coherent final answer containing `done` (the preferred "compaction kept the turn alive"
// outcome); a clear context-overflow diagnostic with a non-zero exit would also be acceptable, but
// never a panic or an indefinite hang.
#[test]
fn large_piped_input_compacts_and_still_answers() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    // A genuinely large piped input (far exceeding the 6000-token budget): ~60k lines of padding.
    let padding = "lorem ipsum dolor sit amet padding line\n".repeat(60_000);

    let out = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "After reading the padding above, reply with the single word: done.",
            ],
        ),
        Some(padding.as_bytes()),
    );

    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_no_panic(&stderr, "the large-input run");

    let code = out.status.code();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();

    if code == Some(0) {
        assert!(
            stdout.to_lowercase().contains("done"),
            "a successful compacted turn must answer with `done`, got stdout: {stdout:?}"
        );
    } else {
        // The only acceptable non-zero outcome is a clear context-overflow diagnostic.
        assert!(
            stderr.to_lowercase().contains("context overflow"),
            "a non-zero exit must be a clear context-overflow diagnostic, got (rc={code:?}) \
             stderr: {stderr}"
        );
    }
}

// VAL-LLM-026: with a fixed `--session`, ~10+ successive prompts each adding sizable text keep
// answering. Once the accumulated history exceeds the (low) context budget, `compactIfNeeded`
// prunes prior context automatically, so the final follow-up still returns a valid assistant answer
// (exit 0) without a context-length crash.
#[test]
fn multi_turn_growth_stays_answerable_via_compaction() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();
    let session = format!("compaction-{}", std::process::id());

    // Each prompt adds sizable filler so the projected history outgrows the 6000-token budget within
    // a few turns, forcing automatic compaction before the later turns are sent.
    let filler = "context filler sentence number ".repeat(120);

    for i in 0..11 {
        let prompt = format!("{filler}\nThis is note {i}. Acknowledge in one short word.");
        let out = run(
            cortex(&key, &home, &["run", "--session", &session, &prompt]),
            None,
        );
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_no_panic(&stderr, &format!("growth turn {i}"));
        assert_eq!(
            out.status.code(),
            Some(0),
            "growth turn {i} must exit 0 (no context-length crash); stderr: {stderr}"
        );
    }

    // The final follow-up, after a large accumulated history, still returns a valid answer.
    let out = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "Reply with exactly one word: ready.",
            ],
        ),
        None,
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_no_panic(&stderr, "the final follow-up");
    assert_eq!(
        out.status.code(),
        Some(0),
        "the final follow-up must exit 0 after history growth; stderr: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !stdout.trim().is_empty(),
        "the final follow-up must return a non-empty assistant answer, got: {stdout:?}"
    );
}

// VAL-GAP-ENGINE-005: after compaction, older seeded facts remain answerable and export includes
// persisted compaction summary metadata (summary + tail_start_id).
#[test]
fn compaction_summary_preserves_seeded_facts_and_persists_metadata() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();
    let session = format!("gap-compaction-summary-{}", std::process::id());
    let token = format!("RIVERSTONE-{}", std::process::id());

    let seed = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                &format!("Remember this archival token exactly: {token}. Reply with stored."),
            ],
        ),
        None,
    );
    assert_eq!(
        seed.status.code(),
        Some(0),
        "seed turn must succeed: {}",
        String::from_utf8_lossy(&seed.stderr)
    );

    let filler = "historical context line ".repeat(220);
    for i in 0..10 {
        let out = run(
            cortex(
                &key,
                &home,
                &[
                    "run",
                    "--session",
                    &session,
                    &format!("{filler}\nTurn {i}: acknowledge with one short word."),
                ],
            ),
            None,
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "history growth turn {i} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let recall = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "What is the archival token? Reply with exactly the token.",
            ],
        ),
        None,
    );
    assert_eq!(
        recall.status.code(),
        Some(0),
        "recall turn must succeed: {}",
        String::from_utf8_lossy(&recall.stderr)
    );
    let recall_stdout = String::from_utf8_lossy(&recall.stdout);
    assert!(
        recall_stdout.contains(&token),
        "final answer should still recall old fact after compaction; got: {recall_stdout}"
    );

    let exported = run(cortex(&key, &home, &["export", &session]), None);
    assert_eq!(
        exported.status.code(),
        Some(0),
        "session export must succeed: {}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let artifact = parse_json(&exported.stdout);
    let compaction_entries: Vec<Value> = artifact["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter_map(|message| {
            let content = message["content"].as_str()?;
            serde_json::from_str::<Value>(content).ok()
        })
        .filter(|content| content["type"] == "compaction_summary")
        .collect();
    assert!(
        !compaction_entries.is_empty(),
        "export must include persisted compaction summary metadata"
    );
    let first = &compaction_entries[0];
    assert!(
        first["summary"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty()),
        "compaction metadata summary should be non-empty: {first}"
    );
    assert!(
        first["tail_start_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("tail_")),
        "compaction metadata should include a tail_start_id: {first}"
    );
}

// VAL-GAP-ENGINE-006: when compaction is needed during a user request, JSONL shows compaction
// first and the same request is auto-continued to a final answer without user resubmission.
#[test]
fn jsonl_shows_compaction_then_auto_continued_answer() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();
    let session = format!("gap-compaction-continue-{}", std::process::id());
    let nonce = format!("AUTO-CONTINUE-{}", std::process::id());

    let filler = "long history payload ".repeat(260);
    for i in 0..8 {
        let out = run(
            cortex(
                &key,
                &home,
                &[
                    "run",
                    "--session",
                    &session,
                    &format!("{filler}\nPrior turn {i}: acknowledge."),
                ],
            ),
            None,
        );
        assert_eq!(
            out.status.code(),
            Some(0),
            "history turn {i} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let out = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--jsonl",
                "--session",
                &session,
                &format!("Reply with exactly this token and nothing else: {nonce}"),
            ],
        ),
        None,
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "jsonl continuation run must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = parse_jsonl_lines(&out.stdout);
    let compaction_index = events.iter().position(|event| {
        event["type"] == "reasoning"
            && event["text"]
                .as_str()
                .is_some_and(|text| text.contains("[context compaction]"))
    });
    let first_text_index = events
        .iter()
        .position(|event| event["type"] == "text_delta");
    assert!(
        compaction_index.is_some(),
        "jsonl stream should mark compaction before continuation"
    );
    assert!(
        first_text_index.is_some(),
        "jsonl stream should include text deltas for the auto-continued request"
    );
    assert!(
        compaction_index.expect("compaction marker") < first_text_index.expect("first text"),
        "compaction marker should arrive before the continued answer stream"
    );
    let answer_text: String = events
        .iter()
        .filter_map(|event| {
            (event["type"] == "text_delta")
                .then(|| event["text"].as_str().unwrap_or_default().to_owned())
        })
        .collect();
    assert!(
        answer_text.contains(&nonce),
        "continued request should be answered without resubmission; got: {answer_text}"
    );
}
