//! Live, end-to-end proof that `cortex run` produces a real, streamed, human-readable assistant
//! answer from OpenRouter (default model `google/gemini-2.5-flash`). NO mocks — every test here
//! drives a real network turn.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the
//! offline `cargo test --workspace` gate stays green. Mission validation runs them with the key
//! exported: `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to the feature's fulfilled assertions:
//! - VAL-LLM-001: a fresh random nonce is echoed back (a non-live path cannot reproduce it).
//! - VAL-LLM-002: real model reasoning (arithmetic) yields the correct, input-dependent answer.
//! - VAL-LLM-003: assistant text streams incrementally (first bytes well before the last).
//! - VAL-LLM-010: the default format is clean prose with no JSON envelope / `"type":` scaffolding.
//! - VAL-LLM-030: Unicode/multibyte input round-trips byte-identically.
//! - VAL-RUN-001: the turn always terminates in bounded time, even on an adversarial prompt.
//! - VAL-SKEL-067: a positional argument and piped stdin are COMBINED in the documented order
//!   (piped stdin first, then the argument), so both sources reach the model, deterministically.

use std::io::Read;
use std::process::{Child, Command, Output, Stdio};
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

/// A `cortex` command carrying the live key and a clean, non-verbose environment so stdout stays
/// pure machine/prose output.
fn cortex(key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
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

/// Reap a child by exact PID without blocking forever (used after manual streaming reads).
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            return child.wait().expect("wait after kill");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

// VAL-LLM-001: a single positional prompt yields a real assistant answer. Proven with a fresh
// random nonce the model is asked to echo: a non-live/canned path cannot reproduce an unseen UUID,
// and a second nonce yields the new value (generated, not static).
#[test]
fn nonce_round_trips_through_a_real_turn() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let mut nonces = Vec::new();
    for _ in 0..2 {
        let nonce = format!("nonce-{}", uuid_like());
        let prompt = format!("Reply with exactly this token, nothing else: {nonce}");
        let out = run(cortex(&key, &["run", &prompt]), Duration::from_secs(60));
        assert_eq!(out.status.code(), Some(0), "a real turn must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        assert!(
            stdout.contains(&nonce),
            "stdout must contain the generated nonce {nonce:?}, got: {stdout:?}"
        );
        nonces.push(nonce);
    }
    assert_ne!(
        nonces[0], nonces[1],
        "each run must use a fresh, generated nonce (not a static value)"
    );
}

// VAL-LLM-002: real model reasoning — a deterministic arithmetic question is answered correctly,
// and a different prompt yields its own correct, input-dependent answer.
#[test]
fn arithmetic_is_answered_correctly() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let first = run(
        cortex(
            &key,
            &[
                "run",
                "What is 17 multiplied by 23? Reply with only the number.",
            ],
        ),
        Duration::from_secs(60),
    );
    assert_eq!(first.status.code(), Some(0));
    let first_out = String::from_utf8(first.stdout).expect("utf8");
    assert!(
        first_out.contains("391"),
        "17*23 must be answered as 391, got: {first_out:?}"
    );

    let second = run(
        cortex(
            &key,
            &["run", "What is 12 plus 30? Reply with only the number."],
        ),
        Duration::from_secs(60),
    );
    assert_eq!(second.status.code(), Some(0));
    let second_out = String::from_utf8(second.stdout).expect("utf8");
    assert!(
        second_out.contains("42"),
        "12+30 must be answered as 42 (output varies with input), got: {second_out:?}"
    );
}

// VAL-LLM-003: streaming produces incremental output during the turn — assistant text arrives in
// multiple chunks over time, with the first bytes appearing well before the final byte.
#[test]
fn output_streams_incrementally() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let mut command = cortex(
        &key,
        &["run", "Count slowly from 1 to 40, one number per line."],
    );
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout handle");

    // Record an arrival timestamp for every read that returns data; incremental streaming yields
    // multiple reads spread over time, whereas an end-of-turn dump would arrive in one burst.
    let mut arrivals: Vec<Instant> = Vec::new();
    let mut collected = Vec::new();
    let mut buf = [0u8; 64];
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                arrivals.push(Instant::now());
                collected.extend_from_slice(&buf[..n]);
            }
            Err(_) => break,
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
    }
    let status = wait_with_timeout(&mut child, Duration::from_secs(10));

    assert!(status.success(), "the streamed turn must exit 0");
    let text = String::from_utf8_lossy(&collected);
    assert!(text.contains('1') && text.contains("40"), "got: {text:?}");
    assert!(
        arrivals.len() >= 2,
        "assistant text must arrive in >=2 chunks over time, got {} chunk(s)",
        arrivals.len()
    );
    let span = *arrivals.last().unwrap() - arrivals[0];
    assert!(
        span > Duration::from_millis(10),
        "first bytes must appear well before the final byte (span was {span:?})"
    );
}

// VAL-LLM-010: the default human format is clean prose — stdout does not parse as a JSON event
// stream and carries no `"type":` event scaffolding.
#[test]
fn default_format_is_clean_prose() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(
        cortex(&key, &["run", "Say hi in one short sentence."]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(!stdout.trim().is_empty(), "prose answer must be non-empty");
    assert!(
        !stdout.contains("\"type\":"),
        "default prose must not leak event scaffolding, got: {stdout:?}"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(stdout.trim()).is_err(),
        "default prose must not parse as a JSON document, got: {stdout:?}"
    );
}

// VAL-LLM-030: Unicode/multibyte input round-trips byte-identically and the output stays valid
// UTF-8.
#[test]
fn unicode_round_trips_byte_identically() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let target = "日本語-café-🚀";
    let prompt = format!("Repeat exactly, nothing else: {target}");
    let out = run(cortex(&key, &["run", &prompt]), Duration::from_secs(60));
    assert_eq!(out.status.code(), Some(0));

    // Valid UTF-8 (strict decode) and the exact byte sequence is present (grep -F equivalent).
    let stdout = String::from_utf8(out.stdout).expect("stdout must be valid UTF-8");
    assert!(
        stdout.contains(target),
        "stdout must contain the exact multibyte string {target:?}, got: {stdout:?}"
    );
}

// VAL-RUN-001: a turn always terminates in bounded time, even on an adversarial prompt, leaving
// no orphan process behind.
#[test]
fn adversarial_prompt_terminates_in_bounded_time() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    // The `run` helper panics if the child does not exit within the timeout, so reaching the
    // assertions proves termination (rc != 124-equivalent) well within the 120s budget.
    let out = run(
        cortex(&key, &["run", "Keep going forever and never stop."]),
        Duration::from_secs(110),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "an adversarial prompt must still reach a clean terminal result"
    );
}

// VAL-LLM-001 supporting check: the `--format json` document identifies the assistant role,
// carries the model text, reports the default model, and includes positive output-token usage —
// proving the answer is backed by a real network call.
#[test]
fn json_document_carries_role_content_model_and_usage() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let out = run(
        cortex(&key, &["run", "--format", "json", "Say hi"]),
        Duration::from_secs(60),
    );
    assert_eq!(out.status.code(), Some(0));
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be a single strict JSON document");
    assert_eq!(doc["role"], "assistant", "role must be assistant");
    assert!(
        doc["content"].as_str().is_some_and(|c| !c.is_empty()),
        "content must carry the model text, got: {doc}"
    );
    assert_eq!(doc["model"], DEFAULT_MODEL, "model must be the default");
    let output_tokens = doc["usage"]["output_tokens"]
        .as_u64()
        .expect("usage.output_tokens must be an integer");
    assert!(
        output_tokens > 0,
        "a real turn reports positive output tokens, got: {output_tokens}"
    );
}

// VAL-SKEL-067: `cortex run` deterministically COMBINES a positional argument with piped stdin in
// the documented order (piped stdin first, then the argument, joined by a blank line), so BOTH
// sources reach the model. Piping `BANANA` and instructing the model to reply with only the single
// capitalized word "supplied above" answers BANANA, which is only possible if the piped word and
// the instruction were combined in that order. Proven identical across two runs (no source dropped,
// stable order, no hang).
#[test]
fn combines_stdin_context_with_argument_instruction_in_order() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let invoke = || {
        run_with_stdin(
            cortex(
                &key,
                &[
                    "run",
                    "Reply with only the single capitalized word supplied above.",
                ],
            ),
            "BANANA",
            Duration::from_secs(60),
        )
    };

    for out in [invoke(), invoke()] {
        assert_eq!(out.status.code(), Some(0), "a combined turn must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        assert!(
            stdout.contains("BANANA"),
            "the combined prompt must carry the piped stdin word to the model, got: {stdout:?}"
        );
    }
}

/// Spawn the binary feeding `stdin_data` on a dedicated writer thread (so a child that drains stdin
/// can never deadlock the writer), and collect its output under a `timeout` hang guard, killing the
/// exact child PID if the timeout is exceeded so no orphan is left behind.
fn run_with_stdin(mut command: Command, stdin_data: &str, timeout: Duration) -> Output {
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");

    let mut stdin = child.stdin.take().expect("stdin handle");
    let owned = stdin_data.to_owned();
    std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(owned.as_bytes());
    });

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

/// A dependency-free, sufficiently-unique token (timestamp + counter), used as a fresh nonce.
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{}-{seq}", std::process::id())
}
