//! Live, end-to-end proof of durable session continuity and stdin input for `cortex run`, driven
//! by REAL OpenRouter turns (default model `google/gemini-2.5-flash`). NO mocks: recall is proven
//! by asking the model to recall a fact it could only know from a prior, separately-invoked turn
//! that was persisted to SQLite and replayed as history.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so the
//! offline `cargo test --workspace` gate stays green. Mission validation runs them with the key
//! exported: `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Each test gets its own fresh `HOME` (which is also the data-dir root), so sessions never collide
//! across tests and every multi-invocation scenario shares exactly one database.
//!
//! Coverage maps to this feature's fulfilled assertions:
//! - VAL-LLM-015: `--session <id>` resumes prior context across separate process invocations.
//! - VAL-LLM-016: a distinct `--session` is isolated (no cross-contamination).
//! - VAL-LLM-017: `--continue` resumes the most recent session.
//! - VAL-LLM-018: the JSON `session` id is non-empty and re-usable via `--session`.
//! - VAL-LLM-019: the prompt is read from stdin (human and machine formats).
//! - VAL-RUN-010: `--continue` on a fresh data dir terminates cleanly (fresh session, exit 0).

use std::io::{Read, Write};
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
        "cortex-session-live-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

/// A `cortex` command bound to a specific hermetic `home` (so repeated invocations in one test share
/// one database), carrying the live key and a clean, non-verbose environment.
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

/// Spawn the binary, optionally feed `stdin_data`, and collect its output under a hard timeout. The
/// exact child PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, stdin_data: Option<&str>) -> Output {
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
        let owned = data.to_owned();
        std::thread::spawn(move || {
            let _ = handle.write_all(owned.as_bytes());
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

/// stdout of a run as a UTF-8 string, asserting a clean exit 0 first.
fn ok_stdout(out: Output, what: &str) -> String {
    assert_eq!(out.status.code(), Some(0), "{what} must exit 0");
    String::from_utf8(out.stdout).expect("stdout must be valid UTF-8")
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

// VAL-LLM-015: `--session <id>` resumes prior context across two SEPARATE process invocations. The
// second turn can only name the word by recalling the first turn from durable history.
#[test]
fn explicit_session_resumes_prior_context() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();
    let session = unique_session_id();

    let first = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "Remember the word: marmalade. Acknowledge.",
            ],
        ),
        None,
    );
    let _ = ok_stdout(first, "the seeding turn");

    let second = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &session,
                "What single word did I ask you to remember?",
            ],
        ),
        None,
    );
    let recalled = ok_stdout(second, "the recall turn");
    assert!(
        recalled.to_lowercase().contains("marmalade"),
        "the resumed session must recall `marmalade`, got: {recalled:?}"
    );
}

// VAL-LLM-016: a distinct `--session` id in the SAME data dir does NOT inherit another session's
// context, proving isolation (no cross-contamination).
#[test]
fn distinct_session_is_isolated() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    // Seed `marmalade` into one session.
    let seeded = unique_session_id();
    let first = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &seeded,
                "Remember the word: marmalade. Acknowledge.",
            ],
        ),
        None,
    );
    let _ = ok_stdout(first, "the seeding turn");

    // A different session, asked the same recall question, has no such context.
    let other = unique_session_id();
    let isolated = run(
        cortex(
            &key,
            &home,
            &[
                "run",
                "--session",
                &other,
                "What single word did I ask you to remember?",
            ],
        ),
        None,
    );
    let answer = ok_stdout(isolated, "the isolated turn");
    assert!(
        !answer.to_lowercase().contains("marmalade"),
        "a distinct session must NOT confidently answer `marmalade`, got: {answer:?}"
    );
}

// VAL-LLM-017: `--continue` (no id) attaches to the most recent session and references its context.
#[test]
fn continue_resumes_the_most_recent_session() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    let first = run(
        cortex(
            &key,
            &home,
            &["run", "My project codename is Falcon. Acknowledge."],
        ),
        None,
    );
    let _ = ok_stdout(first, "the seeding turn");

    let second = run(
        cortex(
            &key,
            &home,
            &["run", "--continue", "What is my project codename?"],
        ),
        None,
    );
    let recalled = ok_stdout(second, "the --continue turn");
    assert!(
        recalled.to_lowercase().contains("falcon"),
        "--continue must recall the codename `Falcon`, got: {recalled:?}"
    );
}

// VAL-LLM-018: the machine `--format json` output surfaces a non-empty, stable session id that,
// fed back via `--session`, continues the same conversation.
#[test]
fn json_session_id_is_nonempty_and_reusable() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    let seed = run(
        cortex(
            &key,
            &home,
            &["run", "--format", "json", "Remember: 4242. Acknowledge."],
        ),
        None,
    );
    let seed_out = ok_stdout(seed, "the json seeding turn");
    let doc: serde_json::Value =
        serde_json::from_str(seed_out.trim()).expect("stdout must be one strict JSON document");
    let session = doc["session"]
        .as_str()
        .expect("the json document must carry a `session` id field");
    assert!(
        !session.trim().is_empty(),
        "the surfaced session id must be non-empty, got: {session:?}"
    );

    let resumed = run(
        cortex(
            &key,
            &home,
            &["run", "--session", session, "What number did I give you?"],
        ),
        None,
    );
    let recalled = ok_stdout(resumed, "the resumed turn");
    assert!(
        recalled.contains("4242"),
        "feeding the surfaced session id back must recall `4242`, got: {recalled:?}"
    );
}

// VAL-LLM-019: the prompt is read from stdin when no positional argument is given, for both the
// human default format and the machine `--format json` format.
#[test]
fn prompt_is_read_from_stdin_in_both_formats() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    let human = run(
        cortex(&key, &home, &["run"]),
        Some("What is 2+2? Reply with only the number.\n"),
    );
    let answer = ok_stdout(human, "the stdin (human) turn");
    assert!(
        answer.contains('4'),
        "a piped human prompt must be answered (`4`), got: {answer:?}"
    );

    let machine = run(
        cortex(&key, &home, &["run", "--format", "json"]),
        Some("Say the word kiwi."),
    );
    let machine_out = ok_stdout(machine, "the stdin (json) turn");
    let doc: serde_json::Value = serde_json::from_str(machine_out.trim())
        .expect("a piped machine prompt must still yield one strict JSON document");
    assert_eq!(
        doc["role"], "assistant",
        "the json answer is the assistant's"
    );
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|c| !c.trim().is_empty()),
        "the json document must carry the model's answer, got: {doc}"
    );
}

// VAL-RUN-010: `--continue` on a FRESH HOME/data dir (no prior session) terminates quickly without
// a panic/hang. This implementation starts a fresh session and answers with exit 0.
#[test]
fn continue_on_fresh_data_dir_starts_fresh_and_answers() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let home = scratch_home();

    let out = run(cortex(&key, &home, &["run", "--continue", "Say hi"]), None);

    // The `run` helper panics on a hang, so reaching here proves bounded termination.
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "a fresh-dir --continue must never panic, got: {stderr}"
    );
    let answer = ok_stdout(out, "the fresh-dir --continue turn");
    assert!(
        !answer.trim().is_empty(),
        "a fresh session must still produce an assistant answer, got: {answer:?}"
    );
}
