//! Black-box tests for `cortex import <url>` (VAL-GAP-CLI-001) and `cortex attach`
//! (VAL-GAP-CLI-004).
//!
//! The import test drives the REAL shipped path: a loopback `cortex serve` creates a share, and
//! `cortex import <url>` fetches and stores it into a separate database. Attach tests prove the
//! observer streams current and future messages and that detaching leaves the producing turn
//! running. Process hygiene: every server is stopped by its lsof-resolved listener PID and every
//! child is waited on; ports stay in 3200-3299.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SERVE_LOCK: Mutex<()> = Mutex::new(());
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);
const WAIT_TIMEOUT: Duration = Duration::from_secs(8);

fn scratch_dir(label: &str) -> PathBuf {
    let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn cortex(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_DATA_DIR");
    command
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn new_session(db_path: &Path) -> String {
    let mut command = cortex(&["session", "new", "--format", "json"]);
    command.env("CORTEX_DB_PATH", db_path);
    let output = command.output().expect("session new");
    assert!(
        output.status.success(),
        "session new failed: {}",
        stderr(&output)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("session json");
    value["id"].as_str().expect("session id").to_owned()
}

fn add_message(db_path: &Path, session: &str, role: &str, content: &str) {
    let mut command = cortex(&["session", "message", session, "--role", role, content]);
    command.env("CORTEX_DB_PATH", db_path);
    let output = command.output().expect("session message");
    assert!(
        output.status.success(),
        "session message failed: {}",
        stderr(&output)
    );
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-001: URL share import fetches and imports a remote share
// --------------------------------------------------------------------------

#[test]
fn import_url_fetches_and_imports_a_real_serve_share() {
    let _guard = SERVE_LOCK.lock().expect("serve lock");
    let port = free_port();
    let src = scratch_dir("import-src");
    let dst = scratch_dir("import-dst");
    let src_db = src.join("cortex.db");
    let dst_db = dst.join("cortex.db");

    // Seed the source database with a transcript (no model turn needed).
    let session = new_session(&src_db);
    add_message(&src_db, &session, "user", "remember the word kumquat");

    let mut server = ServerProcess::start(port, &src_db);
    let share_url = create_share(port, &session);
    assert!(
        share_url.starts_with(&format!("http://127.0.0.1:{port}/share/")),
        "share URL should be a loopback link: {share_url}"
    );

    // Import the share into a DIFFERENT database and prove it is stored and resumable.
    let mut import = cortex(&["import", &share_url]);
    import.env("CORTEX_DB_PATH", &dst_db);
    let output = import.output().expect("import");
    assert!(
        output.status.success(),
        "import failed: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains(&session),
        "import should print the imported session id: {}",
        stdout(&output)
    );

    server.stop();

    let mut list = cortex(&["session", "list"]);
    list.env("CORTEX_DB_PATH", &dst_db);
    let listed = list.output().expect("session list");
    assert!(
        stdout(&listed).contains(&session),
        "imported session must appear in the destination list: {}",
        stdout(&listed)
    );

    let mut show = cortex(&["session", "show", &session]);
    show.env("CORTEX_DB_PATH", &dst_db);
    let shown = show.output().expect("session show");
    assert!(
        stdout(&shown).contains("kumquat"),
        "imported transcript must match the source fixture: {}",
        stdout(&shown)
    );

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&dst).ok();
}

#[test]
fn import_url_fails_closed_under_no_net() {
    let dst = scratch_dir("import-nonet");
    let db = dst.join("cortex.db");
    let mut command = cortex(&[
        "import",
        "http://127.0.0.1:3299/share/shr_x?token=secret-abc",
    ]);
    command.env("CORTEX_DB_PATH", &db).env("CORTEX_NO_NET", "1");
    let output = command.output().expect("import");
    assert!(!output.status.success(), "no-net import must fail");
    assert_eq!(output.status.code(), Some(4), "net-disabled exit code");
    let err = stderr(&output);
    assert!(
        err.to_lowercase().contains("network disabled"),
        "net-disabled diagnostic: {err}"
    );
    assert!(
        stdout(&output).is_empty(),
        "stdout must stay empty on failure"
    );
    std::fs::remove_dir_all(&dst).ok();
}

#[test]
fn import_url_does_not_log_the_share_token_on_failure() {
    let _guard = SERVE_LOCK.lock().expect("serve lock");
    let port = free_port();
    let src = scratch_dir("import-token-src");
    let dst = scratch_dir("import-token-dst");
    let src_db = src.join("cortex.db");
    let _session = new_session(&src_db);

    let mut server = ServerProcess::start(port, &src_db);
    // A bogus share id with a secret token query: the fetch 404s, and the secret must not be
    // echoed in the diagnostic (the URL is sanitized before logging).
    let url = format!("http://127.0.0.1:{port}/share/shr_missing?token=super-secret-xyz");
    let mut import = cortex(&["import", &url]);
    import.env("CORTEX_DB_PATH", dst.join("cortex.db"));
    let output = import.output().expect("import");
    server.stop();

    assert!(
        !output.status.success(),
        "importing a missing share should fail"
    );
    let err = stderr(&output);
    assert!(
        !err.contains("super-secret-xyz"),
        "the share token must not be logged: {err}"
    );

    std::fs::remove_dir_all(&src).ok();
    std::fs::remove_dir_all(&dst).ok();
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-004: attach connects to an active session
// --------------------------------------------------------------------------

#[test]
fn attach_once_prints_current_messages_and_exits() {
    let dir = scratch_dir("attach-once");
    let db = dir.join("cortex.db");
    let session = new_session(&db);
    add_message(&db, &session, "user", "current-message-here");

    let mut command = cortex(&["attach", &session, "--once", "--format", "jsonl"]);
    command.env("CORTEX_DB_PATH", &db);
    let output = command.output().expect("attach --once");
    assert!(
        output.status.success(),
        "attach --once failed: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("current-message-here"),
        "attach should print the current transcript: {}",
        stdout(&output)
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn attach_streams_current_and_future_messages() {
    let dir = scratch_dir("attach-follow");
    let db = dir.join("cortex.db");
    let session = new_session(&db);
    add_message(&db, &session, "user", "first-msg-aaa");

    let mut command = cortex(&[
        "attach",
        &session,
        "--timeout",
        "5",
        "--poll-ms",
        "100",
        "--format",
        "jsonl",
    ]);
    command
        .env("CORTEX_DB_PATH", &db)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command.spawn().expect("spawn attach");

    // Persist a future message while the observer is following.
    thread::sleep(Duration::from_millis(800));
    add_message(&db, &session, "assistant", "second-msg-bbb");

    let output = child.wait_with_output().expect("attach output");
    assert!(
        output.status.success(),
        "attach follow failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        out.contains("first-msg-aaa"),
        "current message should stream: {out}"
    );
    assert!(
        out.contains("second-msg-bbb"),
        "future message should stream: {out}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn attach_through_server_fetches_transcript() {
    let _guard = SERVE_LOCK.lock().expect("serve lock");
    let port = free_port();
    let dir = scratch_dir("attach-server");
    let db = dir.join("cortex.db");
    let session = new_session(&db);
    add_message(&db, &session, "user", "served-transcript-zzz");

    let mut server = ServerProcess::start(port, &db);
    let mut command = cortex(&[
        "attach",
        &session,
        "--server",
        &format!("http://127.0.0.1:{port}"),
        "--once",
        "--format",
        "jsonl",
    ]);
    command.env("CORTEX_DB_PATH", &db);
    let output = command.output().expect("attach --server");
    server.stop();

    assert!(
        output.status.success(),
        "attach --server failed: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("served-transcript-zzz"),
        "attach should fetch the transcript through the server: {}",
        stdout(&output)
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn detaching_the_observer_leaves_the_producing_turn_running() {
    let dir = scratch_dir("attach-detach");
    let db = dir.join("cortex.db");
    let session = new_session(&db);

    // A producer that persists a message after a delay, standing in for an in-flight turn.
    let bin = env!("CARGO_BIN_EXE_cortex");
    let producer_cmd =
        format!("sleep 2; '{bin}' session message '{session}' --role assistant 'late-answer-ccc'");
    let mut producer = Command::new("sh");
    producer
        .arg("-c")
        .arg(&producer_cmd)
        .env("CORTEX_DB_PATH", &db)
        .env_remove("CORTEX_NO_NET")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut producer = producer.spawn().expect("spawn producer");

    // Attach in follow mode, then detach (kill the observer) before the producer finishes.
    let mut attach = cortex(&["attach", &session, "--timeout", "10", "--poll-ms", "100"]);
    attach
        .env("CORTEX_DB_PATH", &db)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut attach = attach.spawn().expect("spawn attach");
    thread::sleep(Duration::from_millis(700));
    attach.kill().expect("detach observer");
    attach.wait().expect("reap observer");

    // The producing turn is unaffected by the detach: it completes and persists.
    let status = producer.wait().expect("producer wait");
    assert!(
        status.success(),
        "producer turn should complete after detach"
    );

    let mut show = cortex(&["session", "show", &session]);
    show.env("CORTEX_DB_PATH", &db);
    let shown = show.output().expect("session show");
    assert!(
        stdout(&shown).contains("late-answer-ccc"),
        "the turn must keep running and persist after the observer detached: {}",
        stdout(&shown)
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Authentic live proof of VAL-GAP-CLI-004's detach semantics: a real OpenRouter `cortex run`
/// turn is in flight while the observer attaches; detaching (killing the observer) must not abort
/// the turn, which finishes and persists. Gated on `OPENROUTER_API_KEY`; skips cleanly when unset.
#[test]
fn attach_then_detach_does_not_abort_a_live_run_turn() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("attach-live-detach");
    let db = dir.join("cortex.db");
    let session = new_session(&db);

    // A real streamed turn that takes several seconds, so the observer attaches mid-flight.
    let prompt = "Count slowly from 1 to 40. Put each number on its own line and add one short \
                  factual sentence about that number.";
    let mut run = cortex(&["run", "--session", &session, prompt]);
    run.env("OPENROUTER_API_KEY", &key)
        .env("CORTEX_DB_PATH", &db)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut run = run.spawn().expect("spawn run");
    let run_pgid = run.id();

    // Attach as a read-only observer while the turn streams.
    let mut attach = cortex(&["attach", &session, "--timeout", "120", "--poll-ms", "200"]);
    attach
        .env("CORTEX_DB_PATH", &db)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut attach = attach.spawn().expect("spawn attach");

    thread::sleep(Duration::from_millis(2500));
    let alive_before_detach = run.try_wait().expect("run try_wait").is_none();

    // Detach: kill ONLY the observer by its exact PID.
    attach.kill().expect("detach observer");
    attach.wait().expect("reap observer");
    thread::sleep(Duration::from_millis(400));

    // Detaching the observer must not have aborted the producing turn.
    if let Some(status) = run.try_wait().expect("run try_wait") {
        assert!(
            status.success(),
            "the live turn must not be aborted by the detach (exit {status})"
        );
    }

    // The turn runs to completion and persists its assistant message.
    let status = wait_for_exit(&mut run, Duration::from_secs(120))
        .expect("the live run turn must finish on its own after the detach");
    assert!(status.success(), "live run turn exited non-zero: {status}");
    assert!(
        alive_before_detach,
        "the turn should have still been streaming when the observer detached"
    );

    if let Some(survivors) = group_has_survivors(run_pgid) {
        assert!(!survivors, "the run process group must leave no orphans");
    }

    let mut show = cortex(&["session", "show", &session]);
    show.env("CORTEX_DB_PATH", &db);
    let shown = show.output().expect("session show");
    assert!(
        stdout(&shown).to_lowercase().contains("assistant"),
        "the completed turn must persist an assistant message: {}",
        stdout(&shown)
    );

    std::fs::remove_dir_all(&dir).ok();
}

// --------------------------------------------------------------------------
// HTTP + process helpers
// --------------------------------------------------------------------------

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// Wait up to `timeout` for a child to exit on its own; force-kill and return `None` otherwise.
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
        thread::sleep(Duration::from_millis(50));
    }
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

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(port: u16, db_path: &Path) -> Self {
        let port_arg = port.to_string();
        let mut command = cortex(&["serve", "--hostname", "127.0.0.1", "--port", &port_arg]);
        command
            .env("CORTEX_DB_PATH", db_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("spawn cortex serve");
        wait_for_health(port, &mut child);
        Self {
            child,
            port,
            stopped: false,
        }
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(pid) = listener_pid(self.port) {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        let deadline = Instant::now() + WAIT_TIMEOUT;
        while Instant::now() < deadline {
            if self.child.try_wait().expect("try wait").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn create_share(port: u16, session: &str) -> String {
    let response = http_request(
        port,
        "POST",
        &format!("/session/{session}/share"),
        &[("Host", &format!("127.0.0.1:{port}"))],
        Some("{}"),
    )
    .expect("share request");
    assert!(
        response.0 == 200,
        "share POST status {}: {}",
        response.0,
        response.1
    );
    let body: serde_json::Value = serde_json::from_str(&response.1).expect("share json");
    body["url"].as_str().expect("share url").to_owned()
}

fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try wait") {
            panic!("server exited before health on port {port}: {status}");
        }
        if let Ok((200, _)) = http_request(port, "GET", "/health", &[], None) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server on port {port} did not become healthy");
}

fn http_request(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> std::io::Result<(u16, String)> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let body = body.unwrap_or("");
    let mut request = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if !has_host {
        request.push_str("Host: 127.0.0.1\r\n");
    }
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, payload) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Ok((status, payload.to_owned()))
}

fn free_port() -> u16 {
    for port in (server::MIN_PORT..=server::MAX_PORT).rev() {
        if listener_pid(port).is_none() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in 3200-3299");
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse().ok())
}
