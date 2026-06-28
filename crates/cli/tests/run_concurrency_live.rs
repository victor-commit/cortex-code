//! Live, end-to-end proof that the durable `cortex run` lifecycle keeps its SQLite state coherent
//! under concurrency and hard failure. NO mocks — every test drives real OpenRouter turns (default
//! `google/gemini-2.5-flash`).
//!
//! Gated on `OPENROUTER_API_KEY`; each test skips cleanly when it is absent so the offline
//! `cargo test --workspace` gate stays green. Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage maps to the feature's fulfilled assertions:
//! - VAL-RUN-009: 3-5 parallel runs on DISTINCT sessions each return their own nonce with no
//!   cross-talk and no DB-lock failure; the shared database stays uncorrupted.
//! - VAL-RUN-011: a `SIGKILL` mid-turn leaves the session resumable and the DB uncorrupted
//!   (`PRAGMA integrity_check` = `ok`), with a later run answering and no orphan/stale-lock error.
//! - VAL-RUN-012: two parallel runs on the SAME session serialize without a "database is locked"
//!   crash, leaving a coherent, non-duplicated message history; integrity_check = `ok`.
//!
//! Process hygiene: every child is launched in its own process group and signalled by its exact
//! PID; the group is checked for survivors after exit so no orphan is left behind.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A generous per-run hang guard: a single real turn completes well within this window.
const RUN_TIMEOUT: Duration = Duration::from_secs(90);

/// Bound for retrying a live-model recall miss (refusal/rephrasing) on the resumed session.
const RECALL_MAX_ATTEMPTS: u32 = 3;
const RECALL_RETRY_BASE_BACKOFF_MS: u64 = 200;

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A unique scratch directory for hermetic per-test state (an isolated `HOME` + database).
fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-run-conc-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A distinctive word+alphanumeric nonce that `gemini-2.5-flash` echoes back verbatim (a plain
/// long digit suffix is sometimes "corrected" by the model, so the suffix stays mixed).
fn nonce(word: &str, salt: u32) -> String {
    format!("{word}x{:x}", (std::process::id() ^ salt) & 0xff_ffff)
}

/// A `cortex` command carrying the live key plus an isolated `HOME` + database path, and a clean,
/// non-verbose environment so stdout stays pure machine/prose output.
fn cortex(key: &str, home: &Path, db_path: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("HOME", home);
    c.env("CORTEX_DB_PATH", db_path);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR")
        .env_remove("CORTEX_STREAM_IDLE_TIMEOUT_MS");
    c
}

/// The collected outcome of a single child run launched in its own process group.
struct RunResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pgid: u32,
}

impl RunResult {
    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Spawn `command` in its own process group with piped streams and a null stdin.
fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn cortex binary")
}

/// Drain a child's stdout/stderr on background threads, returning a first-byte signal for stdout
/// and the join handles for the collected bytes.
#[allow(clippy::type_complexity)]
fn drain_streams(
    child: &mut Child,
) -> (
    mpsc::Receiver<()>,
    std::thread::JoinHandle<Vec<u8>>,
    std::thread::JoinHandle<Vec<u8>>,
) {
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let (tx, rx) = mpsc::channel();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            match out.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let first = buf.is_empty();
                    buf.extend_from_slice(&chunk[..n]);
                    if first {
                        let _ = tx.send(());
                    }
                }
                Err(_) => break,
            }
        }
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    (rx, out_thread, err_thread)
}

/// Wait for a child to exit within `timeout`, returning its status, or `None` (after a force-kill)
/// if it did not exit on its own in time.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Send a signal to an exact PID via `kill -<sig>`. Returns whether the signal was delivered.
fn signal_pid(sig: &str, pid: u32) -> bool {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Whether a process group still has live members. `Some(true)`/`Some(false)` from `pgrep -g`;
/// `None` when `pgrep` is unavailable (the orphan check is then skipped).
fn group_has_survivors(pgid: u32) -> Option<bool> {
    let out = Command::new("pgrep")
        .args(["-g", &pgid.to_string()])
        .output()
        .ok()?;
    match out.status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// Assert a child's stderr carries no Rust panic/backtrace text.
fn assert_no_panic(stderr: &str) {
    for marker in ["panicked at", "RUST_BACKTRACE", "stack backtrace"] {
        assert!(
            !stderr.contains(marker),
            "stderr must not contain a panic/backtrace ({marker}): {stderr:?}"
        );
    }
}

/// Assert a child's process group left no orphan behind (best-effort when `pgrep` exists).
fn assert_no_orphans(pgid: u32) {
    if let Some(true) = group_has_survivors(pgid) {
        panic!("process group {pgid} still has live members after exit (orphan leaked)");
    }
}

/// Run a single child to completion (no signalling), enforcing the hang guard.
fn run_to_completion(command: Command, timeout: Duration) -> RunResult {
    let mut child = spawn_group(command);
    let pgid = child.id();
    let (_rx, out_thread, err_thread) = drain_streams(&mut child);
    let status = wait_for_exit(&mut child, timeout);
    RunResult {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
        pgid,
    }
}

/// Launch every command at once (each in its own process group) so the runs genuinely overlap,
/// then collect each result with the hang guard.
fn run_all_concurrently(commands: Vec<Command>, timeout: Duration) -> Vec<RunResult> {
    let mut children: Vec<(Child, _, _, _, u32)> = Vec::new();
    for command in commands {
        let mut child = spawn_group(command);
        let pgid = child.id();
        let (rx, out_thread, err_thread) = drain_streams(&mut child);
        children.push((child, rx, out_thread, err_thread, pgid));
    }
    let mut results = Vec::new();
    for (mut child, _rx, out_thread, err_thread, pgid) in children {
        let status = wait_for_exit(&mut child, timeout);
        results.push(RunResult {
            status,
            stdout: out_thread.join().expect("join stdout"),
            stderr: err_thread.join().expect("join stderr"),
            pgid,
        });
    }
    results
}

/// A read-only probe: `PRAGMA integrity_check` against the on-disk file is `"ok"`.
fn integrity_ok(db_path: &Path) -> bool {
    let conn = rusqlite::Connection::open(db_path).expect("open db file read-only");
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity check");
    result == "ok"
}

/// The (id, role, content) of every message under a session, in chronological order.
fn message_rows(db_path: &Path, session: &str) -> Vec<(String, String, String)> {
    let conn = rusqlite::Connection::open(db_path).expect("open db file read-only");
    let mut statement = conn
        .prepare(
            "SELECT id, role, content FROM messages WHERE session_id = ?1 \
             ORDER BY created_at ASC, rowid ASC",
        )
        .expect("prepare");
    statement
        .query_map([session], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

// VAL-RUN-009: 3-5 independent runs on DISTINCT sessions, each with a unique nonce prompt, all run
// concurrently against the same shared database; every process exits 0 with its own nonce in its
// stdout (no cross-talk, no DB-lock failure), and the shared DB stays uncorrupted.
#[test]
fn concurrent_distinct_session_runs_each_return_their_own_answer() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let home = scratch_dir("distinct");
    let db_path = home.join("cortex.db");

    let words = ["quokka", "wombat", "narwhal", "axolotl", "pangolin"];
    let nonces: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| nonce(w, i as u32 * 7 + 1))
        .collect();

    // Build every command up front, then launch them all at once for genuine overlap.
    let mut commands = Vec::new();
    let sessions: Vec<String> = (0..words.len())
        .map(|i| format!("val-run-009-{}-{i}", std::process::id()))
        .collect();
    let prompts: Vec<String> = nonces
        .iter()
        .map(|n| {
            format!(
                "Output exactly the following text and nothing else, no quotes, no extra words: {n}"
            )
        })
        .collect();
    for i in 0..words.len() {
        commands.push(cortex(
            &key,
            &home,
            &db_path,
            &["run", "--session", &sessions[i], &prompts[i]],
        ));
    }

    let results = run_all_concurrently(commands, RUN_TIMEOUT);

    for (i, result) in results.iter().enumerate() {
        let status = result
            .status
            .unwrap_or_else(|| panic!("run {i} did not terminate within the hang guard"));
        assert_no_panic(&result.stderr_str());
        assert_eq!(
            status.code(),
            Some(0),
            "run {i} must exit 0 (no DB-lock failure), stderr: {:?}",
            result.stderr_str()
        );
        let stdout = result.stdout_str();
        assert!(
            stdout.contains(&nonces[i]),
            "run {i} stdout must contain its own nonce {:?}, got: {stdout:?}",
            nonces[i]
        );
        // No cross-talk: this run's stdout must not contain any OTHER run's nonce.
        for (j, other) in nonces.iter().enumerate() {
            if j != i {
                assert!(
                    !stdout.contains(other),
                    "run {i} stdout leaked another run's nonce {other:?}: {stdout:?}"
                );
            }
        }
        assert_no_orphans(result.pgid);
    }

    assert!(
        integrity_ok(&db_path),
        "the shared database must remain uncorrupted after concurrent distinct-session runs"
    );

    let _ = std::fs::remove_dir_all(&home);
}

// VAL-RUN-011: a hard `SIGKILL` mid-turn leaves the session resumable and the DB uncorrupted. After
// establishing a fact and hard-killing a following long streaming turn, a later run on the same
// session recalls the fact with exit 0, integrity_check is `ok`, and no orphan/stale lock remains.
#[test]
fn sigkilled_turn_leaves_session_resumable_and_db_uncorrupted() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let home = scratch_dir("sigkill");
    let db_path = home.join("cortex.db");
    let session = format!("val-run-011-{}", std::process::id());
    let secret = nonce("anchovy", 3);

    // 1) Establish a fact that completes normally and is durably persisted.
    let establish = run_to_completion(
        cortex(
            &key,
            &home,
            &db_path,
            &[
                "run",
                "--session",
                &session,
                &format!("Remember this exact word: {secret}. Reply with only: ok"),
            ],
        ),
        RUN_TIMEOUT,
    );
    assert_eq!(
        establish.status.and_then(|s| s.code()),
        Some(0),
        "establishing the fact must succeed, stderr: {:?}",
        establish.stderr_str()
    );

    // 2) Start a long streaming turn on the same session and hard-kill it mid-stream by exact PID.
    let mut child = spawn_group(cortex(
        &key,
        &home,
        &db_path,
        &[
            "run",
            "--session",
            &session,
            "Count slowly from 1 to 300, one number per line.",
        ],
    ));
    let pid = child.id();
    let pgid = pid;
    let (rx, out_thread, err_thread) = drain_streams(&mut child);
    let saw_output = rx.recv_timeout(RUN_TIMEOUT).is_ok();
    assert!(
        saw_output,
        "expected streamed output before SIGKILL (pid {pid})"
    );
    assert!(
        signal_pid("-KILL", pid),
        "kill -KILL must reach the live child {pid}"
    );
    let killed_status = wait_for_exit(&mut child, Duration::from_secs(8))
        .expect("a SIGKILLed process must be reaped promptly");
    // A SIGKILL is not a clean exit: it is terminated by the signal, not an exit code.
    assert!(
        killed_status.code().is_none(),
        "a SIGKILLed turn is killed by signal, not a normal exit: {killed_status:?}"
    );
    let _ = out_thread.join();
    let _ = err_thread.join();
    std::thread::sleep(Duration::from_millis(200));
    assert_no_orphans(pgid);

    // 3) The session is uncorrupted and still resumable: the fact is recalled, exit 0, no stale
    //    lock error. A live model occasionally refuses or rephrases the recall, so the recall is
    //    retried on the SAME resumed session. The real resumability coverage (exit 0, no panic, no
    //    stale-lock error, no orphan) is asserted on EVERY attempt; only the model's recall wording
    //    is tolerated and retried, so the durability/db-integrity coverage is unchanged.
    let recall = recall_fact_until_remembered(&key, &home, &db_path, &session, &secret);
    assert_no_orphans(recall.pgid);

    assert!(
        integrity_ok(&db_path),
        "PRAGMA integrity_check must be ok after a SIGKILL mid-turn"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// Tolerant recall match: compares only the ascii-alphanumeric characters (lowercased) so the
/// seeded word still matches when the model wraps it in prose/quotes or changes case. A truncated
/// or omitted word does NOT match (that miss is the retry's job, not the matcher's).
fn recalls_fact(answer: &str, secret: &str) -> bool {
    fn squeeze(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    squeeze(answer).contains(&squeeze(secret))
}

/// Re-ask the resumed session to recall the pre-kill fact, retrying on a live-model recall miss
/// (refusal/rephrasing). EVERY attempt still proves real resumability: the run must exit 0 with no
/// panic and no stale-lock error, and a hard failure (non-zero exit/panic) panics immediately
/// rather than being retried. Only the model's recall wording is tolerated and retried. Returns the
/// first attempt whose stdout recalls the fact, or panics with every attempt's output.
fn recall_fact_until_remembered(
    key: &str,
    home: &Path,
    db_path: &Path,
    session: &str,
    secret: &str,
) -> RunResult {
    let mut failures = Vec::new();
    for attempt in 1..=RECALL_MAX_ATTEMPTS {
        let recall = run_to_completion(
            cortex(
                key,
                home,
                db_path,
                &[
                    "run",
                    "--session",
                    session,
                    "What exact word did I ask you to remember earlier? Reply with only that word.",
                ],
            ),
            RUN_TIMEOUT,
        );
        assert_no_panic(&recall.stderr_str());
        assert_eq!(
            recall.status.and_then(|s| s.code()),
            Some(0),
            "resuming after a SIGKILL must succeed (no stale-lock error), stderr: {:?}",
            recall.stderr_str()
        );
        if recalls_fact(&recall.stdout_str(), secret) {
            return recall;
        }
        assert_no_orphans(recall.pgid);
        failures.push(format!(
            "attempt {attempt}/{RECALL_MAX_ATTEMPTS}: stdout={:?}",
            recall.stdout_str()
        ));
        if attempt < RECALL_MAX_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(
                RECALL_RETRY_BASE_BACKOFF_MS * u64::from(attempt),
            ));
        }
    }
    panic!(
        "the resumed session must recall the pre-kill fact {secret:?} within {RECALL_MAX_ATTEMPTS} attempts: {}",
        failures.join(" | ")
    );
}

/// Offline regression guard for the tolerant recall matcher: a correctly recalled word must still
/// match through common live-model reformatting, while a truncated word or a refusal must NOT match
/// (those misses are the bounded retry's job, not the matcher's).
#[test]
fn recalls_fact_tolerates_reformatting_but_not_omission() {
    let secret = "anchovyx1a2b3c";
    assert!(recalls_fact(secret, secret), "exact recall matches");
    assert!(
        recalls_fact("The word was: anchovyx1a2b3c.", secret),
        "prose-wrapped recall matches"
    );
    assert!(
        recalls_fact("`ANCHOVYX1A2B3C`", secret),
        "quoted/case-changed recall matches"
    );
    assert!(
        !recalls_fact("anchovy", secret),
        "a truncated word must not match"
    );
    assert!(
        !recalls_fact("I don't recall any specific word.", secret),
        "a refusal must not match"
    );
}

// VAL-RUN-012: two runs on the SAME session with distinct nonce prompts run in parallel. Both
// terminate cleanly (no panic, no "database is locked" crash) — serialized by the busy timeout — and
// a follow-up read of the session shows both turns recorded in a consistent order with no duplicated
// or truncated rows; integrity_check is `ok`.
#[test]
fn concurrent_same_session_runs_serialize_without_corruption() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let home = scratch_dir("same-session");
    let db_path = home.join("cortex.db");
    let session = format!("val-run-012-{}", std::process::id());

    let nonce_a = nonce("marmalade", 11);
    let nonce_b = nonce("zucchini", 23);
    let prompt = |n: &str| {
        format!(
            "Output exactly the following text and nothing else, no quotes, no extra words: {n}"
        )
    };
    let pa = prompt(&nonce_a);
    let pb = prompt(&nonce_b);

    let commands = vec![
        cortex(&key, &home, &db_path, &["run", "--session", &session, &pa]),
        cortex(&key, &home, &db_path, &["run", "--session", &session, &pb]),
    ];

    let results = run_all_concurrently(commands, RUN_TIMEOUT);

    let mut successes = 0usize;
    for (i, result) in results.iter().enumerate() {
        assert!(
            result.status.is_some(),
            "same-session run {i} must terminate within the hang guard"
        );
        let stderr = result.stderr_str();
        assert_no_panic(&stderr);
        let lower = stderr.to_lowercase();
        // The raw SQLite "database is locked" text must never crash through: contention is either
        // absorbed (exit 0) or surfaced as the clean, classified "busy" diagnostic.
        assert!(
            !lower.contains("database is locked"),
            "run {i} must not surface a raw 'database is locked' crash, got: {stderr:?}"
        );
        let code = result.status.and_then(|s| s.code());
        assert!(
            code == Some(0) || lower.contains("busy"),
            "run {i} must exit 0 or yield a clear busy diagnostic, got {code:?}: {stderr:?}"
        );
        if code == Some(0) {
            successes += 1;
        }
        assert_no_orphans(result.pgid);
    }
    assert!(
        successes >= 1,
        "at least one same-session run must complete; both are expected to"
    );

    // A follow-up read shows a coherent, non-duplicated history: each completed run contributed
    // exactly one user prompt and one assistant reply.
    let rows = message_rows(&db_path, &session);
    let users = rows.iter().filter(|(_, role, _)| role == "user").count();
    let assistants = rows
        .iter()
        .filter(|(_, role, _)| role == "assistant")
        .count();
    assert_eq!(
        users, successes,
        "each completed run persists exactly one user prompt, rows: {rows:?}"
    );
    assert_eq!(
        assistants, successes,
        "each completed run persists exactly one assistant reply, rows: {rows:?}"
    );
    assert_eq!(
        rows.len(),
        successes * 2,
        "no duplicated or truncated rows, got: {rows:?}"
    );

    // Message ids are unique (no duplicated rows from interleaved writes).
    let mut ids: Vec<&str> = rows.iter().map(|(id, _, _)| id.as_str()).collect();
    ids.sort_unstable();
    let unique_before = ids.len();
    ids.dedup();
    assert_eq!(
        ids.len(),
        unique_before,
        "every message id is unique: {rows:?}"
    );

    // In the expected (both-succeed) case, both distinct nonce prompts are present, proving no
    // lost write and no cross-contamination.
    if successes == 2 {
        let user_text: String = rows
            .iter()
            .filter(|(_, role, _)| role == "user")
            .map(|(_, _, content)| content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            user_text.contains(&nonce_a) && user_text.contains(&nonce_b),
            "both distinct prompts must be recorded, got: {user_text:?}"
        );
    }

    assert!(
        integrity_ok(&db_path),
        "PRAGMA integrity_check must be ok after concurrent same-session runs"
    );

    let _ = std::fs::remove_dir_all(&home);
}
