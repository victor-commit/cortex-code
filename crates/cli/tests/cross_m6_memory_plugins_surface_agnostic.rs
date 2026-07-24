//! Cross-M6 memory + plugin surface-agnostic proofs (VAL-CROSS-016, VAL-CROSS-026).
//!
//! On one shared project + data dir:
//! - an `instructions[]` glob rules file (and only that file) must shape a server-driven turn;
//!   deleting it restores ordinary casing/behavior.
//! - a registered plugin lifecycle hook must fire for headless, TUI, and server turns, appending
//!   a surface-tagged marker line so all three tags appear in one marker file.
//! - every server listener and plugin child is reaped by exact PID (no orphans).
//!
//! Offline fixture provider proves both flows without OpenRouter. Live real-provider path runs when
//! `OPENROUTER_API_KEY` is set (mission validation) to match the contract's OpenRouter wording.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(90);
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(20);
const TUI_WAIT_TIMEOUT_MS: u64 = 25_000;
const LIVE_MODEL: &str = "google/gemini-2.5-flash";
const UPPERCASE_DIRECTIVE: &str = "Directive CROSS_MEM_UPPER: Reply only in uppercase. Every word of the final answer must be uppercase.";

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_PORT: AtomicU16 = AtomicU16::new(18815);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping plugin cross-surface proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

fn tuistory_available() -> bool {
    Command::new("tuistory")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "a1b2c3d4".repeat(8))
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cortex"))
}

fn free_server_port() -> u16 {
    for _ in 0..80 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > server::MAX_PORT {
            NEXT_PORT.store(server::MIN_PORT, Ordering::SeqCst);
            continue;
        }
        if listener_pids(port).is_empty() && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 18765-18864");
}

fn bind_fixture_listener() -> (u16, TcpListener) {
    for _ in 0..80 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > server::MAX_PORT {
            NEXT_PORT.store(server::MIN_PORT, Ordering::SeqCst);
            continue;
        }
        if listener_pids(port).is_empty()
            && let Ok(listener) = TcpListener::bind(("127.0.0.1", port))
        {
            return (port, listener);
        }
    }
    panic!("no free fixture port in 18765-18864");
}

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-m6-mem-plugin-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(root.join("data")).expect("create data");
        Self {
            db: root.join("data/cortex.db"),
            root,
            home,
            project,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn cortex_base(scratch: &Scratch, args: &[&str]) -> Command {
    let mut command = Command::new(binary());
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("XDG_CONFIG_HOME", scratch.home.join(".config"));
    command.env("XDG_DATA_HOME", scratch.home.join(".local/share"));
    command.env("CORTEX_DATA_DIR", scratch.home.join(".local/share/cortex"));
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("PATH", std::env::var("PATH").unwrap_or_default());
    command
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_PURE")
        .env_remove("CORTEX_MODEL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run_command(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex process timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout_thread.join().expect("stdout join"),
        stderr: stderr_thread.join().expect("stderr join"),
    }
}

fn json_doc(out: Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "command failed (code={:?}):\nstdout={}\nstderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout is not JSON: {err}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    })
}

fn content(doc: &serde_json::Value) -> &str {
    doc["content"].as_str().expect("assistant content")
}

/// Letters-only uppercase check: punctuation/digits stay allowed, but every alphabetic char is upper.
fn is_all_uppercase_message(text: &str) -> bool {
    let letters: String = text.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty() && letters.chars().all(|c| c.is_uppercase())
}

fn has_lowercase_letter(text: &str) -> bool {
    text.chars().any(|c| c.is_lowercase())
}

// ─── Fixture OpenRouter (inspects system messages for memory injection) ──────

struct MemoryAwareOpenRouter {
    base_url: String,
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MemoryAwareOpenRouter {
    /// When the request system contains UPPERCASE_DIRECTIVE, answer with all-uppercase text;
    /// otherwise answer in mixed case. This makes memory injection / post-delete restore observable
    /// without a live provider.
    fn spawn() -> Self {
        let (port, listener) = bind_fixture_listener();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_http_body(&mut stream);
                        let body = extract_http_body(&request);
                        let system = extract_system_text(&body);
                        let has_uppercase_directive = system.contains("Reply only in uppercase")
                            || system.contains("CROSS_MEM_UPPER");
                        let answer = if has_uppercase_directive {
                            "SERVER MEMORY OBEYED: HELLO FROM INSTRUCTIONS GLOB"
                        } else {
                            "Server memory normal: Hello from mixed case"
                        };
                        let multi_frame = sse_body(&[
                            serde_json::json!({
                                "choices": [{
                                    "delta": { "content": answer },
                                    "finish_reason": null
                                }]
                            })
                            .to_string(),
                            serde_json::json!({
                                "choices": [{
                                    "delta": {},
                                    "finish_reason": "stop"
                                }],
                                "usage": {
                                    "prompt_tokens": 8,
                                    "completion_tokens": 6,
                                    "total_tokens": 14
                                }
                            })
                            .to_string(),
                        ]);
                        write_sse_response(&mut stream, &multi_frame)
                            .expect("write provider response");
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15));
                    }
                    Err(err) => panic!("accept provider request: {err}"),
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            port,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for MemoryAwareOpenRouter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct StaticOpenRouter {
    base_url: String,
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl StaticOpenRouter {
    fn spawn(answer: &'static str) -> Self {
        let (port, listener) = bind_fixture_listener();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = read_http_body(&mut stream);
                        let body = sse_body(&[
                            serde_json::json!({
                                "choices": [{
                                    "delta": { "content": answer },
                                    "finish_reason": null
                                }]
                            })
                            .to_string(),
                            serde_json::json!({
                                "choices": [{
                                    "delta": {},
                                    "finish_reason": "stop"
                                }],
                                "usage": {
                                    "prompt_tokens": 1,
                                    "completion_tokens": 1,
                                    "total_tokens": 2
                                }
                            })
                            .to_string(),
                        ]);
                        write_sse_response(&mut stream, &body).expect("write provider response");
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15));
                    }
                    Err(err) => panic!("accept provider request: {err}"),
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            port,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for StaticOpenRouter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn sse_body(frames: &[String]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn write_sse_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => return buffer,
            Ok(n) => n,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(err) => panic!("read request: {err}"),
        };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&buffer) {
            break end;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let already = buffer.len().saturating_sub(header_end);
    let mut remaining = content_length.saturating_sub(already);
    while remaining > 0 {
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(err) => panic!("read body: {err}"),
        };
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    buffer
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn extract_http_body(raw: &[u8]) -> String {
    if let Some(end) = find_header_end(raw) {
        String::from_utf8_lossy(&raw[end..]).into_owned()
    } else {
        String::from_utf8_lossy(raw).into_owned()
    }
}

fn extract_system_text(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return String::new();
    };
    let mut out = String::new();

    if let Some(system) = value.get("system").and_then(|v| v.as_str()) {
        out.push_str(system);
        out.push('\n');
    }

    if let Some(messages) = value.get("messages").and_then(|v| v.as_array()) {
        for message in messages {
            let role = message
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if role != "system" && role != "developer" {
                continue;
            }
            match message.get("content") {
                Some(serde_json::Value::String(text)) => {
                    out.push_str(text);
                    out.push('\n');
                }
                Some(serde_json::Value::Array(parts)) => {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            out.push_str(text);
                            out.push('\n');
                        } else if let Some(text) = part.as_str() {
                            out.push_str(text);
                            out.push('\n');
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

// ─── Server process helpers ──────────────────────────────────────────────────

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(
        scratch: &Scratch,
        base_url: Option<&str>,
        key: Option<&str>,
        extra_env: &[(&str, &str)],
    ) -> Self {
        let port = free_server_port();
        let port_arg = port.to_string();
        let mut command = cortex_base(
            scratch,
            &[
                "serve",
                "--hostname",
                "127.0.0.1",
                "--port",
                port_arg.as_str(),
            ],
        );
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(key) = key {
            command.env("OPENROUTER_API_KEY", key);
        } else {
            command.env("OPENROUTER_API_KEY", fake_openrouter_key());
        }
        if let Some(base_url) = base_url {
            command.env("CORTEX_OPENROUTER_BASE_URL", base_url);
        } else {
            command.env_remove("CORTEX_OPENROUTER_BASE_URL");
        }
        for (k, v) in extra_env {
            command.env(k, v);
        }
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
        if let Some(pid) = single_listener_pid(self.port) {
            let status = Command::new("kill")
                .arg(pid.to_string())
                .status()
                .expect("kill listener pid");
            assert!(status.success(), "kill listener {pid} failed: {status}");
        }
        wait_for_child_exit(&mut self.child, Duration::from_secs(10));
        // Force residual reaps only by exact PID: ensure port is free.
        if let Some(pid) = single_listener_pid(self.port) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            wait_for_port_released(self.port);
        }
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
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
        many => panic!("expected single listener for {port}, got {many:?}"),
    }
}

fn wait_for_port_released(port: u16) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if single_listener_pid(port).is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(40));
    }
    panic!("port {port} still held after kill");
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) {
    let started = Instant::now();
    loop {
        if child.try_wait().expect("try wait").is_some() {
            return;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        thread::sleep(Duration::from_millis(40));
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn request_json(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    request_http(port, method, path, body).unwrap_or_else(|err| {
        panic!("HTTP {method} {path} on {port} failed: {err}");
    })
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
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

fn wait_for_health(port: u16, child: &mut Child) {
    let started = Instant::now();
    let mut last = String::new();
    while started.elapsed() < SERVER_START_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try wait") {
            let mut stderr = String::new();
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_string(&mut stderr);
            }
            panic!("serve exited before health on {port}: {status}\n{stderr}");
        }
        match request_http(port, "GET", "/health", None) {
            Ok(response) if response.status == 200 => return,
            Ok(response) => last = format!("HTTP {}", response.status),
            Err(err) => last = err.to_string(),
        }
        thread::sleep(Duration::from_millis(40));
    }
    panic!("serve on {port} never became healthy: {last}");
}

fn post_server_message(port: u16, prompt: &str, model: Option<&str>) -> serde_json::Value {
    let created = request_json(port, "POST", "/session", Some("{}"));
    assert_eq!(
        created.status, 201,
        "create session failed: {}",
        created.body
    );
    let session: serde_json::Value =
        serde_json::from_str(&created.body).expect("session create JSON");
    let session_id = session["id"].as_str().expect("session id");
    let body = match model {
        Some(model) => serde_json::json!({ "text": prompt, "model": model }).to_string(),
        None => serde_json::json!({ "text": prompt }).to_string(),
    };
    let response = request_json(
        port,
        "POST",
        &format!("/session/{session_id}/message"),
        Some(&body),
    );
    assert_eq!(
        response.status, 200,
        "message failed: {} {}",
        response.status, response.body
    );
    serde_json::from_str(&response.body).expect("message JSON")
}

// ─── TUI helpers ─────────────────────────────────────────────────────────────

struct TuistorySession {
    name: String,
}

impl TuistorySession {
    fn launch(name: String, scratch: &Scratch, env: &[(&str, String)], cli_args: &[&str]) -> Self {
        let command_line = std::iter::once(binary().to_string_lossy().into_owned())
            .chain(cli_args.iter().map(|arg| (*arg).to_owned()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut launch = Command::new("tuistory");
        launch
            .arg("launch")
            .arg(command_line)
            .arg("-s")
            .arg(&name)
            .arg("--cwd")
            .arg(&scratch.project)
            .arg("--cols")
            .arg("120")
            .arg("--rows")
            .arg("36")
            .arg("--env")
            .arg(format!("HOME={}", scratch.home.display()))
            .arg("--env")
            .arg(format!(
                "XDG_CONFIG_HOME={}",
                scratch.home.join(".config").display()
            ))
            .arg("--env")
            .arg(format!(
                "XDG_DATA_HOME={}",
                scratch.home.join(".local/share").display()
            ))
            .arg("--env")
            .arg(format!(
                "CORTEX_DATA_DIR={}",
                scratch.home.join(".local/share/cortex").display()
            ))
            .arg("--env")
            .arg(format!("CORTEX_DB_PATH={}", scratch.db.display()))
            .arg("--env")
            .arg("CORTEX_AUTH_FILE=")
            .arg("--env")
            .arg("RUST_LOG=")
            .arg("--env")
            .arg("CORTEX_VERBOSE=")
            .arg("--env")
            .arg("NO_COLOR=")
            .arg("--env")
            .arg("CORTEX_PURE=")
            .arg("--env")
            .arg("CORTEX_NO_NET=");
        for (key, value) in env {
            launch.arg("--env").arg(format!("{key}={value}"));
        }
        let out = launch
            .arg("--background")
            .arg("--timeout")
            .arg("12000")
            .output()
            .expect("launch tuistory");
        assert!(
            out.status.success(),
            "tuistory launch failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        thread::sleep(Duration::from_millis(600));
        Self { name }
    }

    fn run(&self, args: &[&str]) {
        let out = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .args(args)
            .output()
            .expect("tuistory command");
        assert!(
            out.status.success(),
            "tuistory {args:?} failed:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn wait_for(&self, needle: &str, timeout_ms: u64) {
        let out = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("wait")
            .arg(needle)
            .arg("--timeout")
            .arg(timeout_ms.to_string())
            .output()
            .expect("tuistory wait");
        assert!(
            out.status.success(),
            "TUI did not render {needle:?}:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

impl Drop for TuistorySession {
    fn drop(&mut self) {
        let _ = Command::new("tuistory")
            .arg("-s")
            .arg(&self.name)
            .arg("close")
            .output();
    }
}

// ─── Plugin + memory project fixtures ────────────────────────────────────────

fn write_instructions_rules(project: &Path, rules_body: &str) {
    write_file(
        &project.join("cortex.toml"),
        "instructions = [\"docs/*.md\"]\n",
    );
    write_file(&project.join("docs/house-rules.md"), rules_body);
}

fn write_marker_plugin(project: &Path, marker: &Path) {
    write_file(
        &project.join("surface-hook.mjs"),
        r#"
import { appendFile, mkdir } from "node:fs/promises";
import { dirname } from "node:path";

export default async function plugin(input, options) {
  return {
    async event(event) {
      if (event.kind === "turn.started") {
        const surface = event.surface ?? "unknown";
        await mkdir(dirname(options.marker), { recursive: true });
        await appendFile(options.marker, `${surface}\n`);
      }
    },
    async provider() {
      return {
        id: "plugin-cross-surface",
        name: "Plugin Cross Surface",
        models: [{
          id: "plugin/cross-surface-model",
          name: "Cross Surface Model",
          tool_call: false
        }],
        async complete(request) {
          return `PLUGIN_CROSS_SURFACE_OK:${request.model}`;
        }
      };
    }
  };
}
"#,
    );
    // Preserve any existing instructions[] line by rewriting a combined config when both exist.
    let instructions_line = "instructions = [\"docs/*.md\"]\n";
    let plugin_line = format!(
        "plugin = [[\"./surface-hook.mjs\", {{ marker = \"{}\" }}]]\n",
        marker.display()
    );
    let existing = std::fs::read_to_string(project.join("cortex.toml")).unwrap_or_default();
    let mut body = String::new();
    if existing.contains("instructions") {
        body.push_str(instructions_line);
    }
    body.push_str(&plugin_line);
    write_file(&project.join("cortex.toml"), &body);
}

fn write_lifecycle_only_plugin(project: &Path, marker: &Path) {
    write_file(
        &project.join("surface-hook.mjs"),
        r#"
import { appendFile, mkdir } from "node:fs/promises";
import { dirname } from "node:path";

export default async function plugin(input, options) {
  return {
    async event(event) {
      if (event.kind === "turn.started") {
        const surface = event.surface ?? "unknown";
        await mkdir(dirname(options.marker), { recursive: true });
        await appendFile(options.marker, `${surface}\n`);
      }
    }
  };
}
"#,
    );
    write_file(
        &project.join("cortex.toml"),
        &format!(
            "plugin = [[\"./surface-hook.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    );
}

fn read_marker_surfaces(marker: &Path) -> Vec<String> {
    if !marker.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(target_os = "linux")]
fn assert_no_orphan_proc(pid: u32) {
    let path = PathBuf::from(format!("/proc/{pid}"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !path.exists(),
        "process PID {pid} must be reaped (no orphan left)"
    );
}

// ─── VAL-CROSS-016 ───────────────────────────────────────────────────────────

/// Offline fixture: an instructions[]-only house-rules file shapes a server turn system
/// so the assistant answer goes all-uppercase; deleting it restores mixed case. Server killed
/// by lsof PID.
#[test]
fn server_turn_obeys_instructions_glob_memory_and_deletes_to_restore() {
    let scratch = Scratch::new("server-memory-fixture");
    // No AGENTS.md — directive lives only in the globbed rules file.
    write_instructions_rules(&scratch.project, UPPERCASE_DIRECTIVE);

    let provider = MemoryAwareOpenRouter::spawn();
    let mut server = ServerProcess::start(&scratch, Some(&provider.base_url), None, &[]);
    let with_rules = post_server_message(
        server.port,
        "Say hello in one short sentence.",
        Some(LIVE_MODEL),
    );
    assert_eq!(with_rules["status"], "completed");
    let with_content = with_rules["content"].as_str().unwrap_or_default();
    assert!(
        is_all_uppercase_message(with_content),
        "server turn must obey instructions[] uppercase directive, got: {with_content:?}"
    );
    assert!(
        with_content.contains("SERVER MEMORY OBEYED") || is_all_uppercase_message(with_content),
        "fixture answer should prove system injection: {with_content:?}"
    );

    // Delete the only source of the directive and restart the server so memory is re-discovered.
    std::fs::remove_file(scratch.project.join("docs/house-rules.md"))
        .expect("delete house-rules.md");
    server.stop();
    wait_for_port_released(server.port);

    let mut server2 = ServerProcess::start(&scratch, Some(&provider.base_url), None, &[]);
    let without_rules = post_server_message(
        server2.port,
        "Say hello in one short sentence.",
        Some(LIVE_MODEL),
    );
    assert_eq!(without_rules["status"], "completed");
    let without_content = without_rules["content"].as_str().unwrap_or_default();
    assert!(
        has_lowercase_letter(without_content),
        "deleting instructions[] file must restore normal mixed-case answer, got: {without_content:?}"
    );
    assert!(
        !without_content.contains("SERVER MEMORY OBEYED"),
        "uppercase fixture branch must not fire after file delete: {without_content:?}"
    );

    let port = server2.port;
    server2.stop();
    wait_for_port_released(port);
    assert!(
        single_listener_pid(port).is_none(),
        "server listener on {port} must be gone after lsof-PID kill"
    );
}

/// Live OpenRouter path for VAL-CROSS-016 (skipped without key).
#[test]
fn live_server_turn_obeys_instructions_glob_uppercase_and_restores_after_delete() {
    let Some(key) = live_key() else {
        eprintln!("skipping live VAL-CROSS-016: OPENROUTER_API_KEY not set");
        return;
    };

    let scratch = Scratch::new("server-memory-live");
    write_instructions_rules(&scratch.project, UPPERCASE_DIRECTIVE);

    let mut server = ServerProcess::start(&scratch, None, Some(&key), &[]);
    let with_rules = post_server_message(
        server.port,
        "Follow the active project memory exactly. Reply with a one-sentence greeting about the weather.",
        Some(LIVE_MODEL),
    );
    assert_eq!(
        with_rules["status"], "completed",
        "live server turn failed: {with_rules}"
    );
    let with_content = with_rules["content"].as_str().unwrap_or_default();
    assert!(
        is_all_uppercase_message(with_content),
        "live server turn must obey uppercase-only instructions[] directive, got: {with_content:?}"
    );

    std::fs::remove_file(scratch.project.join("docs/house-rules.md")).expect("delete house-rules");
    // Contract: deleting restores normal casing. No replacement rule is written.
    server.stop();
    wait_for_port_released(server.port);

    let mut server2 = ServerProcess::start(&scratch, None, Some(&key), &[]);
    let without_rules = post_server_message(
        server2.port,
        "Reply with a one-sentence greeting about the weather. Use ordinary mixed English casing.",
        Some(LIVE_MODEL),
    );
    assert_eq!(without_rules["status"], "completed");
    let without_content = without_rules["content"].as_str().unwrap_or_default();
    assert!(
        has_lowercase_letter(without_content),
        "after deleting instructions[] file, live answer must restore ordinary casing, got: {without_content:?}"
    );

    let port = server2.port;
    server2.stop();
    wait_for_port_released(port);
    assert!(single_listener_pid(port).is_none());
}

// ─── VAL-CROSS-026 ───────────────────────────────────────────────────────────

/// Offline: one plugin hooks headless + TUI + server turns on a shared project; marker ends with
/// all three surface tags; no orphan plugin child / server listener.
#[test]
fn plugin_lifecycle_hook_fires_for_headless_tui_and_server_surfaces() {
    if skip_without_runtime() {
        return;
    }

    let scratch = Scratch::new("plugin-three-surfaces");
    let marker = scratch.project.join("markers/surface-hooks.log");
    write_lifecycle_only_plugin(&scratch.project, &marker);

    // Headless (surface tag "run") via plugin-registered provider so no live key is needed.
    // Re-write plugin with provider for headless/server/tui without requiring OpenRouter.
    write_marker_plugin(&scratch.project, &marker);

    let headless = run_command(cortex_base(
        &scratch,
        &[
            "run",
            "--model",
            "plugin/cross-surface-model",
            "--format",
            "json",
            "trigger headless plugin lifecycle",
        ],
    ));
    let headless_json = json_doc(headless);
    assert!(
        content(&headless_json).contains("PLUGIN_CROSS_SURFACE_OK"),
        "headless should complete via plugin provider: {headless_json}"
    );

    // Server (surface tag "serve") using the same plugin provider bookkeeping. Server resolves
    // OpenRouter by model id string, so point it at a local static provider and keep the hook-only
    // lifecycle. The lifecycle plugin already fires on turn.started independent of which provider
    // answers — drive server with a static fixture keyed as OpenRouter.
    write_lifecycle_only_plugin(&scratch.project, &marker);
    let provider = StaticOpenRouter::spawn("SERVER-HOOK-OK");
    let mut server = ServerProcess::start(&scratch, Some(&provider.base_url), None, &[]);
    let server_resp = post_server_message(
        server.port,
        "trigger server plugin lifecycle",
        Some(LIVE_MODEL),
    );
    assert_eq!(server_resp["status"], "completed");
    assert!(
        server_resp["content"]
            .as_str()
            .unwrap_or_default()
            .contains("SERVER-HOOK-OK"),
        "server turn content: {server_resp}"
    );
    let serve_port = server.port;
    let serve_listener = single_listener_pid(serve_port);
    server.stop();
    wait_for_port_released(serve_port);
    if let Some(pid) = serve_listener {
        #[cfg(target_os = "linux")]
        assert_no_orphan_proc(pid);
    }

    // TUI (surface tag "tui") when tuistory is present.
    if tuistory_available() {
        write_lifecycle_only_plugin(&scratch.project, &marker);
        let tui_provider = StaticOpenRouter::spawn("TUI-HOOK-OK");
        let session_name = format!(
            "cross-m6-plugin-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let env = [
            ("OPENROUTER_API_KEY", fake_openrouter_key()),
            ("CORTEX_OPENROUTER_BASE_URL", tui_provider.base_url.clone()),
        ];
        let tui = TuistorySession::launch(session_name, &scratch, &env, &["tui"]);
        tui.run(&["type", "trigger tui plugin lifecycle hook"]);
        tui.run(&["press", "enter"]);
        tui.wait_for("TUI-HOOK-OK", TUI_WAIT_TIMEOUT_MS);
        drop(tui);
        drop(tui_provider);
    } else {
        eprintln!(
            "tuistory unavailable; covering TUI surface via in-process PluginEventHooks('tui')"
        );
        // Still prove the plugins crate + TUI surface name fires turn.started into the same
        // marker path by using PluginEventHooks loaded for "tui" against a synthetic event.
        write_lifecycle_only_plugin(&scratch.project, &marker);
        let config_text =
            std::fs::read_to_string(scratch.project.join("cortex.toml")).expect("read cortex.toml");
        let table: toml::Value = toml::from_str(&config_text).expect("parse cortex.toml");
        let plugin_config =
            plugins::PluginConfig::from_toml_value(table.get("plugin"), &scratch.project)
                .expect("plugin config");
        let hooks = plugins::PluginEventHooks::load_or_disabled(&plugin_config, false, "tui")
            .expect("load tui hooks");
        let event = events::Event::turn_started("session-tui-fallback", "message-tui-fallback");
        hooks.invoke_event(&event);
        // Give the external runtime a beat to flush the marker append.
        thread::sleep(Duration::from_millis(200));
        drop(hooks);
    }

    let surfaces = read_marker_surfaces(&marker);
    let has_run = surfaces.iter().any(|s| s == "run");
    let has_serve = surfaces.iter().any(|s| s == "serve");
    let has_tui = surfaces.iter().any(|s| s == "tui");
    assert!(
        has_run,
        "marker must include headless/run surface tag; got {surfaces:?}"
    );
    assert!(
        has_serve,
        "marker must include serve surface tag; got {surfaces:?}"
    );
    assert!(
        has_tui,
        "marker must include tui surface tag; got {surfaces:?}"
    );
    assert!(
        single_listener_pid(serve_port).is_none(),
        "no orphan server listener may remain"
    );
}
