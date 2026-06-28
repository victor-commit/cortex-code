//! Live no-mock hardening proofs for M7 finishing.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent so offline
//! `cargo test --workspace` remains green. With the key present, they lock in the "live-by-default"
//! contract for both `cortex run` and `cortex serve`:
//! - VAL-FINISH-058: plain `cortex run` is live by default (fresh nonce round-trip, no flags).
//! - VAL-FINISH-059: default `cortex run` opens an outbound socket (`connect()` seen in strace).
//! - VAL-FINISH-060: default path does not read cassette/fixture files as a response source.
//! - VAL-FINISH-061: live works in a fresh clean CWD with only the key present.
//! - VAL-FINISH-062: `cortex serve` is also live by default, and the listener is stopped by the
//!   `lsof -tiTCP:<port> -sTCP:LISTEN` PID with the port released.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(90);
const SERVE_TIMEOUT: Duration = Duration::from_secs(20);
// A live LLM turn over HTTP can take far longer than a control-plane request, so the socket read
// must outlast a real model turn rather than the ~2s that suffices for /health and /session.
const SERVE_HTTP_READ_TIMEOUT: Duration = Duration::from_secs(90);
const NONCE_MAX_ATTEMPTS: u32 = 3;
const NONCE_RETRY_BASE_BACKOFF_MS: u64 = 200;
const SERVE_PORT_START: u16 = 3200;
const SERVE_PORT_END: u16 = 3239;
static COUNTER: AtomicU64 = AtomicU64::new(0);
static NEXT_SERVE_PORT: AtomicU16 = AtomicU16::new(SERVE_PORT_START);
// Acquired poison-tolerantly at every call site so a single panicking live test cannot cascade a
// PoisonError into the other live tests sharing this lock.
static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(label: &str) -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-no-mock-{label}-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::create_dir_all(dir.join(".config")).expect("create scratch config dir");
    std::fs::create_dir_all(dir.join(".local/share")).expect("create scratch data dir");
    dir
}

fn uuid_like() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{}-{seq}", std::process::id())
}

fn cortex_command(home: &Path, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.current_dir(home);
    command
        .env("OPENROUTER_API_KEY", key)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("CORTEX_DATA_DIR", home.join(".local/share/cortex"))
        .env_remove("CORTEX_TRANSPORT")
        .env_remove("CORTEX_CASSETTE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("RUST_LOG")
        .env_remove("NO_COLOR");
    command
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = thread::spawn(move || {
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
            panic!("process did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

/// Formatting-tolerant nonce match: compares only the ascii-alphanumeric characters (lowercased) so
/// a correctly echoed nonce still matches when the model wraps it in code fences/quotes, swaps the
/// hyphens for spaces, or changes case. Outright truncation/omission is handled by the retry below.
fn echoes_nonce(answer: &str, nonce: &str) -> bool {
    fn squeeze(text: &str) -> String {
        text.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }
    squeeze(answer).contains(&squeeze(nonce))
}

/// Round-trip a fresh nonce through `cortex run`, retrying on a live-model wording miss (a slipped
/// character or a generic "agent-less mode" greeting). Each attempt uses its own fresh nonce and is
/// matched with the formatting-tolerant `echoes_nonce`, so a single live slip no longer fails the
/// test while a genuinely missing answer still does after the bounded retries.
fn assert_round_trips_fresh_nonce_with_retries(
    home: &Path,
    key: &str,
    cwd: &Path,
    label: &str,
) -> String {
    let mut failures = Vec::new();
    for attempt in 1..=NONCE_MAX_ATTEMPTS {
        let nonce = format!("{label}-{}", uuid_like());
        let prompt = format!("Reply with exactly this token and nothing else: {nonce}");
        let mut command = cortex_command(home, key, &["run", &prompt]);
        command.current_dir(cwd);
        let out = run_with_timeout(command, RUN_TIMEOUT);
        let status = out.status.code();
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
        if status == Some(0) && echoes_nonce(&stdout, &nonce) {
            return nonce;
        }

        failures.push(format!(
            "attempt {attempt}/{NONCE_MAX_ATTEMPTS}, nonce={nonce:?}, status={status:?}, stdout={stdout:?}, stderr={stderr:?}"
        ));
        if attempt < NONCE_MAX_ATTEMPTS {
            thread::sleep(Duration::from_millis(
                NONCE_RETRY_BASE_BACKOFF_MS * u64::from(attempt),
            ));
        }
    }

    panic!(
        "default run failed to round-trip a fresh nonce after {NONCE_MAX_ATTEMPTS} attempts: {}",
        failures.join(" | ")
    );
}

#[test]
fn default_run_is_live_by_default_with_fresh_nonces() {
    let Some(key) = live_key() else {
        eprintln!("skipping no-mock live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let home = scratch_dir("run-default-live");
    let nonce1 = assert_round_trips_fresh_nonce_with_retries(&home, &key, &home, "first");
    let nonce2 = assert_round_trips_fresh_nonce_with_retries(&home, &key, &home, "second");
    assert_ne!(nonce1, nonce2, "fresh nonces must differ");
}

#[test]
fn default_run_opens_outbound_connection() {
    let Some(key) = live_key() else {
        eprintln!("skipping no-mock live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !strace_usable() {
        eprintln!("skipping strace no-mock test: strace unavailable or ptrace restricted");
        return;
    }
    let _guard = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let home = scratch_dir("run-connect-trace");
    let trace_path = home.join("connect.trace");
    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=connect", "-o"])
        .arg(&trace_path)
        .arg(env!("CARGO_BIN_EXE_cortex"))
        .arg("run")
        .arg("Say hi");
    command
        .current_dir(&home)
        .env("OPENROUTER_API_KEY", &key)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("CORTEX_DATA_DIR", home.join(".local/share/cortex"))
        .env_remove("CORTEX_TRANSPORT")
        .env_remove("CORTEX_CASSETTE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH");
    let out = run_with_timeout(command, RUN_TIMEOUT);
    assert_eq!(
        out.status.code(),
        Some(0),
        "traced default run should still succeed"
    );

    let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
    let connects = trace.matches("connect(").count();
    assert!(
        connects >= 1,
        "default live path must open >=1 outbound socket, saw {connects}:\n{trace}"
    );
}

#[test]
fn default_path_does_not_read_cassette_or_fixture_files() {
    let Some(key) = live_key() else {
        eprintln!("skipping no-mock live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !strace_usable() {
        eprintln!("skipping strace no-mock test: strace unavailable or ptrace restricted");
        return;
    }
    let _guard = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let home = scratch_dir("run-no-mock-open-trace");
    let clean_cwd = home.join("clean");
    std::fs::create_dir_all(&clean_cwd).expect("create clean cwd");
    let trace_path = home.join("open.trace");

    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=open,openat", "-o"])
        .arg(&trace_path)
        .arg(env!("CARGO_BIN_EXE_cortex"))
        .arg("run")
        .arg("Reply with the single word live");
    command
        .current_dir(&clean_cwd)
        .env("OPENROUTER_API_KEY", &key)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("CORTEX_DATA_DIR", home.join(".local/share/cortex"))
        .env_remove("CORTEX_TRANSPORT")
        .env_remove("CORTEX_CASSETTE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH");

    let out = run_with_timeout(command, RUN_TIMEOUT);
    assert_eq!(
        out.status.code(),
        Some(0),
        "default run in clean cwd should succeed live"
    );

    let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
    let lower = trace.to_lowercase();
    assert!(
        !lower.contains("cassette") && !lower.contains("fixture"),
        "default run should not open cassette/fixture files in clean cwd:\n{trace}"
    );
}

#[test]
fn live_run_works_in_fresh_clean_cwd_with_only_key() {
    let Some(key) = live_key() else {
        eprintln!("skipping no-mock live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let home = scratch_dir("run-clean-cwd");
    let clean_cwd = home.join("fresh");
    std::fs::create_dir_all(&clean_cwd).expect("create clean cwd");

    assert_round_trips_fresh_nonce_with_retries(&home, &key, &clean_cwd, "clean-cwd");
}

#[test]
fn serve_surface_is_live_by_default_and_stops_by_listener_pid() {
    let Some(key) = live_key() else {
        eprintln!("skipping no-mock live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = scratch_dir("serve-live-default");
    let port = free_port();
    let mut server = ServerProcess::start(&home, &key, port);
    let _health = wait_for_health(port, &mut server.child);

    serve_round_trips_fresh_nonce_with_retries(port, "serve");

    let listener_pid = single_listener_pid(port).expect("listener pid");
    server.stop_by_listener_pid(listener_pid);
    wait_for_port_released(port);
}

/// Round-trip a fresh nonce through a live `cortex serve` message turn, retrying on a live-model
/// wording miss. Each attempt creates its own fresh session so a retried turn never piggybacks on a
/// prior session's transcript, and the answer is matched with the formatting-tolerant `echoes_nonce`
/// so a single live slip no longer fails the test while a genuinely missing answer still does.
fn serve_round_trips_fresh_nonce_with_retries(port: u16, label: &str) -> (String, String) {
    let mut failures = Vec::new();
    for attempt in 1..=NONCE_MAX_ATTEMPTS {
        let created = request_json(port, "POST", "/session", Some("{}"));
        assert_eq!(created.status, 201, "session create should succeed");
        let session: Value = serde_json::from_str(&created.body).expect("session json");
        let session_id = session["id"]
            .as_str()
            .expect("session id string")
            .to_owned();

        let nonce = format!("{label}-{}", uuid_like());
        let body =
            format!(r#"{{"text":"Reply with exactly this token and nothing else: {nonce}"}}"#);
        let turn = request_json(
            port,
            "POST",
            &format!("/session/{session_id}/message"),
            Some(&body),
        );
        if turn.status == 200 {
            let content = serde_json::from_str::<Value>(&turn.body)
                .ok()
                .and_then(|doc| doc["content"].as_str().map(str::to_owned))
                .unwrap_or_default();
            if echoes_nonce(&content, &nonce) {
                return (session_id, nonce);
            }
            failures.push(format!(
                "attempt {attempt}/{NONCE_MAX_ATTEMPTS}, nonce={nonce:?}, status=200, content={content:?}"
            ));
        } else {
            failures.push(format!(
                "attempt {attempt}/{NONCE_MAX_ATTEMPTS}, nonce={nonce:?}, status={}, body={:?}",
                turn.status, turn.body
            ));
        }
        if attempt < NONCE_MAX_ATTEMPTS {
            thread::sleep(Duration::from_millis(
                NONCE_RETRY_BASE_BACKOFF_MS * u64::from(attempt),
            ));
        }
    }

    panic!(
        "serve turn failed to round-trip a fresh nonce after {NONCE_MAX_ATTEMPTS} attempts: {}",
        failures.join(" | ")
    );
}

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(home: &Path, key: &str, port: u16) -> Self {
        let db_path = home.join("serve.db");
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args([
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ])
            .current_dir(home)
            .env("OPENROUTER_API_KEY", key)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("CORTEX_DATA_DIR", home.join(".local/share/cortex"))
            .env("CORTEX_DB_PATH", db_path)
            .env_remove("CORTEX_TRANSPORT")
            .env_remove("CORTEX_CASSETTE")
            .env_remove("CORTEX_NO_NET")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = command.spawn().expect("spawn cortex serve");
        Self {
            child,
            port,
            stopped: false,
        }
    }

    fn stop_by_listener_pid(&mut self, pid: u32) {
        if self.stopped {
            return;
        }
        kill_pid(pid);
        wait_for_child_exit(&mut self.child, SERVE_TIMEOUT);
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        if let Some(pid) = single_listener_pid(self.port) {
            kill_pid(pid);
        }
        wait_for_child_exit(&mut self.child, SERVE_TIMEOUT);
        self.stopped = true;
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn wait_for_health(port: u16, child: &mut Child) -> HttpResponse {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < SERVE_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!("server exited before health check on port {port}: {status}");
        }
        match request_http(port, "GET", "/health", None) {
            Ok(response) if response.status == 200 => return response,
            Ok(response) => last_error = format!("http {}", response.status),
            Err(err) => last_error = err.to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not become healthy on port {port}: {last_error}");
}

fn request_json(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    request_http(port, method, path, body)
        .unwrap_or_else(|err| panic!("{method} {path} on {port} failed: {err}"))
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(SERVE_HTTP_READ_TIMEOUT))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let body = body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes())?;

    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Ok(HttpResponse {
        status,
        body: body.to_owned(),
    })
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve child did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn free_port() -> u16 {
    for _ in 0..80 {
        let port = NEXT_SERVE_PORT.fetch_add(1, Ordering::SeqCst);
        if port > SERVE_PORT_END {
            NEXT_SERVE_PORT.store(SERVE_PORT_START, Ordering::SeqCst);
            continue;
        }
        if listener_pids(port).is_empty() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free serve port in reserved range 3200-3239");
}

fn listener_pids(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .arg(format!("-tiTCP:{port}"))
        .arg("-sTCP:LISTEN")
        .output()
        .expect("run lsof");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn single_listener_pid(port: u16) -> Option<u32> {
    let pids = listener_pids(port);
    match pids.as_slice() {
        [pid] => Some(*pid),
        [] => None,
        many => panic!("expected one listener pid for {port}, got {many:?}"),
    }
}

fn kill_pid(pid: u32) {
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .expect("run kill");
    assert!(status.success(), "kill {pid} failed with {status}");
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < SERVE_TIMEOUT {
        if listener_pids(port).is_empty() {
            assert!(
                TcpListener::bind(("127.0.0.1", port)).is_ok(),
                "port {port} appears free by lsof but cannot be rebound"
            );
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("port {port} still busy after listener kill");
}

/// Offline regression guard for the tolerant nonce matcher: a correctly echoed nonce must still
/// match through common live-model reformatting, while a truncated/different token must NOT match
/// (those misses are the bounded retry's job, not the matcher's). Runs without a key.
#[test]
fn echoes_nonce_tolerates_reformatting_but_not_truncation() {
    let nonce = "serve-18bd5b0644f5a86d-e69a2";
    assert!(echoes_nonce(nonce, nonce), "exact echo matches");
    assert!(
        echoes_nonce("Sure, here it is: `serve-18bd5b0644f5a86d-e69a2`", nonce),
        "code-fenced/quoted echo with surrounding prose matches"
    );
    assert!(
        echoes_nonce("serve 18bd5b0644f5a86d e69a2.", nonce),
        "hyphens reflowed to spaces with trailing punctuation matches"
    );
    assert!(
        echoes_nonce("SERVE-18BD5B0644F5A86D-E69A2", nonce),
        "case-changed echo matches"
    );
    assert!(
        !echoes_nonce("serve-18bd5b0644f5a86d", nonce),
        "a truncated nonce must not match (handled by retry)"
    );
    assert!(
        !echoes_nonce("a generic agent-less mode greeting", nonce),
        "an unrelated greeting must not match"
    );
}
