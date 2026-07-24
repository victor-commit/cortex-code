//! Integration proof for first-run cleanliness and cross-surface model/OpenAPI discovery
//! (VAL-CROSS-034, VAL-CROSS-036, VAL-CROSS-037).
//!
//! Offline halves always run under a pristine HOME/project and verify:
//! - headless keyless guidance + no panic
//! - `cortex serve` health + OpenAPI document listing session/message/event routes
//! - invalid model rejection with a clear, consistent error on headless and server
//! - clean lsof-PID shutdown with no orphan listener
//!
//! Live halves (gated on `OPENROUTER_API_KEY`) additionally prove a shared capable model
//! (`anthropic/claude-sonnet-4`) is honored by `cortex run --model` and the server message
//! `model` parameter. The TUI `/models` path is coverable via tuistory in mission validation;
//! row ranking + selection is unit-tested in the `tui` crate.

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(90);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
/// Shared capable model for the cross-surface selection proof (VAL-CROSS-036).
const SELECTED_MODEL: &str = "anthropic/claude-sonnet-4";
const INVALID_MODEL: &str = "not-a-real-model";
const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cross-m6-firstrun-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("create home");
    std::fs::create_dir_all(dir.join("project")).expect("create project");
    dir
}

fn free_server_port() -> u16 {
    for port in 18765..=18864 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 18765-18864");
}

fn cortex_base(home: &Path, project: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(project);
    command.env("HOME", home);
    command.env("PATH", std::env::var("PATH").unwrap_or_default());
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_MODEL")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
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
}

impl ProcessResult {
    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    fn code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
    }
}

fn collect_child(mut child: Child, timeout: Duration) -> ProcessResult {
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

fn run_cortex(home: &Path, project: &Path, args: &[&str], key: Option<&str>) -> ProcessResult {
    let mut command = cortex_base(home, project);
    command.args(args);
    if let Some(key) = key {
        command.env("OPENROUTER_API_KEY", key);
    }
    collect_child(spawn_group(command), RUN_TIMEOUT)
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

fn wait_for_health(port: u16) {
    let started = Instant::now();
    loop {
        let response = curl(&["-sf", &format!("http://127.0.0.1:{port}/health")]);
        if response.code() == Some(0) {
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
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        listener_pid(port).is_none(),
        "server port {port} still has a listener after kill of PID {listener}"
    );
    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_ok(),
        "server port {port} should be free after shutdown"
    );
}

fn assert_no_panic(label: &str, out: &ProcessResult) {
    let combined = format!("{}\n{}", out.stdout_str(), out.stderr_str());
    assert!(
        !combined.contains("panicked at")
            && !combined.contains("RUST_BACKTRACE")
            && !combined.contains("stack backtrace"),
        "{label} must not emit a panic/backtrace:\n{combined}"
    );
}

fn assert_actionable_no_credential(stderr: &str) {
    let lower = stderr.to_lowercase();
    assert!(
        lower.contains("credential") || lower.contains("api key"),
        "keyless first-run must name the missing credential, got: {stderr}"
    );
    assert!(
        stderr.contains("OPENROUTER_API_KEY") || lower.contains("openrouter"),
        "keyless first-run should name OPENROUTER_API_KEY / OpenRouter, got: {stderr}"
    );
}

fn assert_invalid_model_message(surface: &str, text: &str) {
    assert!(
        text.contains(INVALID_MODEL),
        "{surface} must name the invalid model id, got: {text}"
    );
    assert!(
        text.to_lowercase().contains("unknown model")
            || text.to_lowercase().contains("provider/model")
            || text.to_lowercase().contains("fully-qualified"),
        "{surface} must give an actionable invalid-model diagnostic, got: {text}"
    );
}

// VAL-CROSS-034 + VAL-CROSS-036 (invalid path offline) + VAL-CROSS-037
#[test]
fn pristine_first_run_and_openapi_discovery_stay_sane() {
    let root = scratch_dir("offline");
    let home = root.join("home");
    let project = root.join("project");

    // --- headless keyless first-run guidance ---
    let keyless = run_cortex(&home, &project, &["run", "hi"], None);
    assert_ne!(
        keyless.code(),
        Some(0),
        "keyless first-run must not silently succeed"
    );
    assert!(
        keyless.stdout.is_empty(),
        "keyless first-run must keep stdout empty for machines, got: {}",
        keyless.stdout_str()
    );
    assert_actionable_no_credential(&keyless.stderr_str());
    assert_no_panic("keyless headless run", &keyless);

    // --- headless invalid model (no credential needed — local validation) ---
    let invalid_headless = run_cortex(
        &home,
        &project,
        &["run", "--model", INVALID_MODEL, "--format", "json", "hi"],
        None,
    );
    assert_ne!(
        invalid_headless.code(),
        Some(0),
        "invalid model on headless must fail"
    );
    assert!(
        invalid_headless.stdout.is_empty(),
        "invalid model must not emit an answer on stdout"
    );
    assert_invalid_model_message("headless", &invalid_headless.stderr_str());
    assert_no_panic("invalid-model headless run", &invalid_headless);

    // --- serve on pristine HOME + OpenAPI + invalid model ---
    let port = free_server_port();
    let mut serve_cmd = cortex_base(&home, &project);
    serve_cmd.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    let child = spawn_group(serve_cmd);
    wait_for_health(port);

    let health = curl(&["-sS", &format!("http://127.0.0.1:{port}/health")]);
    assert_eq!(
        health.code(),
        Some(0),
        "health stderr: {}",
        health.stderr_str()
    );
    let health_json: serde_json::Value =
        serde_json::from_slice(&health.stdout).expect("health JSON");
    assert_eq!(health_json, serde_json::json!({"status": "ok"}));

    let openapi = curl(&["-sS", &format!("http://127.0.0.1:{port}/openapi.json")]);
    assert_eq!(
        openapi.code(),
        Some(0),
        "openapi stderr: {}",
        openapi.stderr_str()
    );
    let document: serde_json::Value =
        serde_json::from_slice(&openapi.stdout).expect("openapi JSON");
    assert!(
        document
            .get("openapi")
            .or_else(|| document.get("swagger"))
            .is_some(),
        "OpenAPI document must declare a version: {document}"
    );
    assert!(document["info"].is_object(), "info required: {document}");
    let paths = document["paths"]
        .as_object()
        .expect("paths object required");
    for expected in [
        "/health",
        "/session",
        "/session/{id}",
        "/session/{id}/message",
        "/event",
    ] {
        assert!(
            paths.contains_key(expected),
            "OpenAPI must list {expected}, got keys: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }

    // At least one documented endpoint behaves as documented (session list + create).
    let listed = curl(&["-sS", &format!("http://127.0.0.1:{port}/session")]);
    assert_eq!(
        listed.code(),
        Some(0),
        "session list stderr: {}",
        listed.stderr_str()
    );
    let listed_json: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("session list JSON");
    assert!(
        listed_json.is_array(),
        "GET /session must return an array, got: {listed_json}"
    );

    let created = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        "{}",
        &format!("http://127.0.0.1:{port}/session"),
    ]);
    assert_eq!(
        created.code(),
        Some(0),
        "create session stderr: {}",
        created.stderr_str()
    );
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("created session JSON");
    let session_id = created_json["id"]
        .as_str()
        .expect("created session id")
        .to_owned();

    let invalid_server = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        &format!(r#"{{"text":"hi","model":"{INVALID_MODEL}"}}"#),
        "-w",
        "\nHTTP_STATUS=%{http_code}\n",
        &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
    ]);
    let invalid_body = invalid_server.stdout_str();
    assert!(
        invalid_body.contains("HTTP_STATUS=400"),
        "invalid model on server must be 400, got: {invalid_body}"
    );
    assert_invalid_model_message("server", &invalid_body);
    assert_no_panic(
        "invalid-model server message",
        &ProcessResult {
            status: invalid_server.status,
            stdout: invalid_server.stdout,
            stderr: invalid_server.stderr,
        },
    );

    // Default model is documented in the capability surface (config/default) when listed via
    // providers catalogue — sanity check against health not panicking under pristine env.
    let _ = DEFAULT_MODEL;

    stop_server(child, port);
}

// VAL-CROSS-036 live halves: same model on headless --model and server model param.
#[test]
fn selected_model_is_consistent_across_headless_and_server() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let root = scratch_dir("live");
    let home = root.join("home");
    let project = root.join("project");

    // Headless --model.
    let headless = run_cortex(
        &home,
        &project,
        &[
            "run",
            "--model",
            SELECTED_MODEL,
            "--format",
            "json",
            "Reply with exactly: pong",
        ],
        Some(&key),
    );
    assert_eq!(
        headless.code(),
        Some(0),
        "headless model turn failed: stdout={} stderr={}",
        headless.stdout_str(),
        headless.stderr_str()
    );
    assert_no_panic("headless selected model", &headless);
    let headless_doc: serde_json::Value =
        serde_json::from_slice(&headless.stdout).expect("headless JSON");
    assert_eq!(
        headless_doc["model"], SELECTED_MODEL,
        "headless turn must report the requested model: {headless_doc}"
    );

    // Server model param.
    let port = free_server_port();
    let mut serve_cmd = cortex_base(&home, &project);
    serve_cmd.env("OPENROUTER_API_KEY", &key);
    serve_cmd.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    let child = spawn_group(serve_cmd);
    wait_for_health(port);

    let created = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        "{}",
        &format!("http://127.0.0.1:{port}/session"),
    ]);
    assert_eq!(
        created.code(),
        Some(0),
        "create session stderr: {}",
        created.stderr_str()
    );
    let created_json: serde_json::Value =
        serde_json::from_slice(&created.stdout).expect("created session JSON");
    let session_id = created_json["id"]
        .as_str()
        .expect("created session id")
        .to_owned();

    let turn = curl(&[
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-d",
        &format!(r#"{{"text":"Reply with exactly: pong","model":"{SELECTED_MODEL}"}}"#),
        &format!("http://127.0.0.1:{port}/session/{session_id}/message"),
    ]);
    assert_eq!(
        turn.code(),
        Some(0),
        "server model turn failed: stdout={} stderr={}",
        turn.stdout_str(),
        turn.stderr_str()
    );
    assert_no_panic("server selected model", &turn);
    let turn_doc: serde_json::Value =
        serde_json::from_slice(&turn.stdout).expect("server turn JSON");
    assert_eq!(
        turn_doc["model"], SELECTED_MODEL,
        "server turn must report the requested model: {turn_doc}"
    );

    // OpenAPI remains fetchable while the live server is up (discovery consistent with CLI).
    let openapi = curl(&["-sS", &format!("http://127.0.0.1:{port}/openapi.json")]);
    assert_eq!(openapi.code(), Some(0));
    let document: serde_json::Value =
        serde_json::from_slice(&openapi.stdout).expect("openapi JSON");
    assert!(
        document["paths"]
            .as_object()
            .is_some_and(|paths| paths.contains_key("/session/{id}/message")),
        "live OpenAPI must keep listing message ops"
    );

    stop_server(child, port);
}
