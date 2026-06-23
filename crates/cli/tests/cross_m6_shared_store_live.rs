//! Live cross-surface shared-store proof for the M6 server integration slice.
//!
//! This test is gated on `OPENROUTER_API_KEY` so offline workspace tests remain deterministic. When
//! the key is exported it starts a real `cortex serve` process, drives a concurrent server turn and
//! headless turn against the same SQLite file, then verifies the sessions remain listable and stats
//! can read the store cleanly. The server listener is stopped by the lsof-resolved listener PID.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-m6-shared-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn free_server_port() -> u16 {
    for port in 3220..=3299 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3220-3299");
}

fn nonce(label: &str, salt: u32) -> String {
    format!("{label}x{:x}", (std::process::id() ^ salt) & 0xff_ffff)
}

fn cortex_command(key: &str, home: &Path, db_path: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("CORTEX_DB_PATH", db_path);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn spawn_group(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.process_group(0);
    command.spawn().expect("spawn process")
}

struct ProcessResult {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pgid: u32,
}

impl ProcessResult {
    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

fn collect_child(mut child: Child, timeout: Duration) -> ProcessResult {
    let pgid = child.id();
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = wait_for_exit(&mut child, timeout);
    ProcessResult {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
        pgid,
    }
}

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
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn curl(args: &[&str]) -> ProcessResult {
    let mut command = Command::new("curl");
    command.args(args);
    collect_child(spawn_group(command), RUN_TIMEOUT)
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.trim().parse::<u32>().ok())
}

fn signal_pid(sig: &str, pid: u32) -> bool {
    Command::new("kill")
        .args([sig, &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

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

fn assert_no_orphans(pgid: u32) {
    if let Some(true) = group_has_survivors(pgid) {
        panic!("process group {pgid} still has live members after exit");
    }
}

fn wait_for_health(port: u16) {
    let started = Instant::now();
    loop {
        let response = curl(&["-sf", &format!("http://127.0.0.1:{port}/health")]);
        if response.status.and_then(|status| status.code()) == Some(0) {
            return;
        }
        assert!(
            started.elapsed() < SERVER_START_TIMEOUT,
            "server on port {port} did not become healthy, last stderr: {}",
            response.stderr_str()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn stop_server(mut child: Child, port: u16) {
    let listener = listener_pid(port).unwrap_or_else(|| child.id());
    assert!(
        signal_pid("-TERM", listener),
        "failed to signal listener PID {listener}"
    );
    let status = wait_for_exit(&mut child, Duration::from_secs(10));
    if status.is_none() {
        let _ = signal_pid("-KILL", listener);
    }
    let mut stdout = child.stdout.take().expect("server stdout");
    let mut stderr = child.stderr.take().expect("server stderr");
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();
    let _ = stdout.read_to_end(&mut out_buf);
    let _ = stderr.read_to_end(&mut err_buf);
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "server port {port} should be released after stopping listener PID {listener}"
    );
}

struct ServerGuard {
    child: Option<Child>,
    port: u16,
    pgid: u32,
}

impl ServerGuard {
    fn new(child: Child, port: u16) -> Self {
        let pgid = child.id();
        Self {
            child: Some(child),
            port,
            pgid,
        }
    }

    fn pgid(&self) -> u32 {
        self.pgid
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            stop_server(child, self.port);
        }
    }
}

#[test]
fn concurrent_headless_and_server_turns_share_store_without_locking() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");
    let db_path = root.join("cortex.db");
    let port = free_server_port();

    let server = ServerGuard::new(
        spawn_group(cortex_command(
            &key,
            &home,
            &db_path,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port.to_string(),
            ],
        )),
        port,
    );
    let server_pgid = server.pgid();
    wait_for_health(port);

    let create = curl(&[
        "-sfS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        "{}",
        &format!("http://127.0.0.1:{port}/session"),
    ]);
    assert_eq!(
        create.status.and_then(|status| status.code()),
        Some(0),
        "create session stderr: {}",
        create.stderr_str()
    );
    let session_b: serde_json::Value =
        serde_json::from_slice(&create.stdout).expect("created session JSON");
    let session_b = session_b["id"].as_str().expect("session id").to_owned();
    let session_a = format!("ses_live_headless_{}", std::process::id());
    let nonce_a = nonce("headless", 17);
    let nonce_b = nonce("server", 29);
    let prompt_a =
        format!("Output exactly this text and nothing else, no quotes, no extra words: {nonce_a}");
    let prompt_b =
        format!("Output exactly this text and nothing else, no quotes, no extra words: {nonce_b}");

    let headless = spawn_group(cortex_command(
        &key,
        &home,
        &db_path,
        &["run", "--session", &session_a, &prompt_a],
    ));
    let server_turn = spawn_group({
        let mut command = Command::new("curl");
        command.args([
            "-sfS",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &serde_json::json!({ "text": prompt_b }).to_string(),
            &format!("http://127.0.0.1:{port}/session/{session_b}/message"),
        ]);
        command
    });

    let headless = collect_child(headless, RUN_TIMEOUT);
    let server_turn = collect_child(server_turn, RUN_TIMEOUT);

    assert_eq!(
        headless.status.and_then(|status| status.code()),
        Some(0),
        "headless stderr: {}",
        headless.stderr_str()
    );
    assert!(
        headless.stdout_str().contains(&nonce_a),
        "headless output should contain its own nonce: {}",
        headless.stdout_str()
    );
    assert_eq!(
        server_turn.status.and_then(|status| status.code()),
        Some(0),
        "server curl stderr: {}",
        server_turn.stderr_str()
    );
    let server_body: serde_json::Value =
        serde_json::from_slice(&server_turn.stdout).expect("server turn JSON");
    assert!(
        server_body["content"]
            .as_str()
            .is_some_and(|content| content.contains(&nonce_b)),
        "server response should contain its own nonce: {server_body:?}"
    );

    let listed = collect_child(
        spawn_group(cortex_command(
            &key,
            &home,
            &db_path,
            &["session", "list", "--format", "json"],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(listed.status.and_then(|status| status.code()), Some(0));
    let list_body: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("session list JSON");
    let ids = list_body["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .filter_map(|session| session["id"].as_str())
        .collect::<Vec<_>>();
    assert!(
        ids.contains(&session_a.as_str()),
        "session A listed: {ids:?}"
    );
    assert!(
        ids.contains(&session_b.as_str()),
        "session B listed: {ids:?}"
    );

    let stats = collect_child(
        spawn_group(cortex_command(
            &key,
            &home,
            &db_path,
            &["stats", "--format", "json"],
        )),
        RUN_TIMEOUT,
    );
    assert_eq!(
        stats.status.and_then(|status| status.code()),
        Some(0),
        "stats stderr: {}",
        stats.stderr_str()
    );
    let stats_body: serde_json::Value = serde_json::from_slice(&stats.stdout).expect("stats JSON");
    assert!(
        stats_body["totals"]["total_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0),
        "stats should read non-zero usage from both turns: {stats_body:?}"
    );

    assert_no_orphans(headless.pgid);
    assert_no_orphans(server_turn.pgid);
    drop(server);
    assert_no_orphans(server_pgid);
    let _ = std::fs::remove_dir_all(&root);
}
