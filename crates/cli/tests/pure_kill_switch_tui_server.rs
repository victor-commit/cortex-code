use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_PORT: AtomicU16 = AtomicU16::new(18765);
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn cortex_pure_serve_suppresses_plugin_event_hooks() {
    if skip_without_runtime() {
        return;
    }

    let normal = Scratch::new("serve-normal");
    write_marker_plugin(&normal.project, "server-events.log");
    let normal_provider = FakeOpenRouter::spawn("SERVER-NORMAL-OK");
    let mut normal_server = ServerProcess::start(&normal, &normal_provider, false);
    post_server_turn(&normal_server, "normal server turn");
    normal_server.stop();
    assert_marker_contains(
        &normal.project.join("server-events.log"),
        "serve:turn.started",
    );

    let pure = Scratch::new("serve-pure");
    write_marker_plugin(&pure.project, "server-events.log");
    let pure_provider = FakeOpenRouter::spawn("SERVER-PURE-OK");
    let mut pure_server = ServerProcess::start(&pure, &pure_provider, true);
    post_server_turn(&pure_server, "pure server turn");
    pure_server.stop();
    assert!(
        !pure.project.join("server-events.log").exists(),
        "`cortex --pure serve` must not load plugins or fire event hooks"
    );
}

#[test]
fn cortex_pure_tui_entrypoints_suppress_plugin_event_hooks() {
    if skip_without_runtime() || !tuistory_available() {
        return;
    }

    let normal = Scratch::new("tui-normal");
    write_marker_plugin(&normal.project, "tui-events.log");
    let normal_provider = FakeOpenRouter::spawn("TUI-NORMAL-OK");
    run_tui_turn(&normal, &normal_provider, &["tui"], "TUI-NORMAL-OK");
    assert_marker_contains(&normal.project.join("tui-events.log"), "tui:turn.started");

    let pure_bare = Scratch::new("tui-pure-bare");
    write_marker_plugin(&pure_bare.project, "tui-events.log");
    let pure_bare_provider = FakeOpenRouter::spawn("TUI-PURE-BARE-OK");
    run_tui_turn(
        &pure_bare,
        &pure_bare_provider,
        &["--pure"],
        "TUI-PURE-BARE-OK",
    );
    assert!(
        !pure_bare.project.join("tui-events.log").exists(),
        "bare `cortex --pure` must not load plugins or fire TUI hooks"
    );

    let pure_tui = Scratch::new("tui-pure-subcommand");
    write_marker_plugin(&pure_tui.project, "tui-events.log");
    let pure_tui_provider = FakeOpenRouter::spawn("TUI-PURE-SUBCOMMAND-OK");
    run_tui_turn(
        &pure_tui,
        &pure_tui_provider,
        &["--pure", "tui"],
        "TUI-PURE-SUBCOMMAND-OK",
    );
    assert!(
        !pure_tui.project.join("tui-events.log").exists(),
        "`cortex --pure tui` must not load plugins or fire TUI hooks"
    );
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping pure kill-switch proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

fn tuistory_available() -> bool {
    match Command::new("tuistory").arg("--version").output() {
        Ok(output) if output.status.success() => true,
        _ => {
            eprintln!("skipping TUI pure kill-switch proof: tuistory is unavailable");
            false
        }
    }
}

fn write_marker_plugin(project: &Path, marker_name: &str) {
    let marker = project.join(marker_name);
    std::fs::write(
        project.join("plugin.mjs"),
        r#"
import { appendFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async event(event) {
      if (event.kind === "turn.started") {
        await appendFile(options.marker, `${event.surface}:${event.kind}\n`);
      }
    }
  };
}
"#,
    )
    .expect("write plugin");
    std::fs::write(
        project.join("cortex.toml"),
        format!(
            "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]\n",
            marker.display()
        ),
    )
    .expect("write config");
}

fn assert_marker_contains(marker: &Path, expected: &str) {
    let marker_text = std::fs::read_to_string(marker).expect("marker file exists");
    assert!(
        marker_text.lines().any(|line| line == expected),
        "expected marker {expected:?}, got {marker_text:?}"
    );
}

fn run_tui_turn(scratch: &Scratch, provider: &FakeOpenRouter, args: &[&str], expected: &str) {
    let session_name = format!(
        "cortex-pure-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let session = TuistorySession::new(session_name.clone());
    let command_line = std::iter::once(binary().to_string_lossy().into_owned())
        .chain(args.iter().map(|arg| (*arg).to_owned()))
        .collect::<Vec<_>>()
        .join(" ");

    let launch = Command::new("tuistory")
        .arg("launch")
        .arg(command_line)
        .arg("-s")
        .arg(&session_name)
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
        .arg(format!("OPENROUTER_API_KEY={}", fake_openrouter_key()))
        .arg("--env")
        .arg(format!("CORTEX_OPENROUTER_BASE_URL={}", provider.base_url))
        .arg("--env")
        .arg("CORTEX_AUTH_FILE=")
        .arg("--env")
        .arg("CORTEX_PURE=")
        .arg("--env")
        .arg("CORTEX_NO_NET=")
        .arg("--env")
        .arg("RUST_LOG=")
        .arg("--env")
        .arg("CORTEX_VERBOSE=")
        .arg("--env")
        .arg("NO_COLOR=")
        .arg("--background")
        .arg("--timeout")
        .arg("10000")
        .output()
        .expect("launch tuistory session");
    assert!(
        launch.status.success(),
        "tuistory launch failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&launch.stdout),
        String::from_utf8_lossy(&launch.stderr)
    );

    tuistory(
        &session_name,
        &["type", "Reply with the validation marker."],
    );
    tuistory(&session_name, &["press", "enter"]);
    let wait = Command::new("tuistory")
        .arg("-s")
        .arg(&session_name)
        .arg("wait")
        .arg(expected)
        .arg("--timeout")
        .arg("20000")
        .output()
        .expect("wait for TUI answer");
    assert!(
        wait.status.success(),
        "TUI did not render {expected:?}:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&wait.stdout),
        String::from_utf8_lossy(&wait.stderr)
    );
    drop(session);
}

fn tuistory(session: &str, args: &[&str]) {
    let output = Command::new("tuistory")
        .arg("-s")
        .arg(session)
        .args(args)
        .output()
        .expect("run tuistory command");
    assert!(
        output.status.success(),
        "tuistory {args:?} failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct TuistorySession {
    name: String,
}

impl TuistorySession {
    fn new(name: String) -> Self {
        Self { name }
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

struct ServerProcess {
    child: Child,
    port: u16,
    stopped: bool,
}

impl ServerProcess {
    fn start(scratch: &Scratch, provider: &FakeOpenRouter, pure: bool) -> Self {
        let port = free_port();
        let port_arg = port.to_string();
        let mut args = Vec::new();
        if pure {
            args.push("--pure");
        }
        args.extend([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            port_arg.as_str(),
        ]);
        let mut command = cortex_command(scratch, &args);
        command.env("OPENROUTER_API_KEY", fake_openrouter_key());
        command.env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url);
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
            assert!(status.success(), "kill {pid} failed with {status}");
        }
        wait_for_child_exit(&mut self.child, RUN_TIMEOUT);
        self.stopped = true;
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn post_server_turn(server: &ServerProcess, prompt: &str) {
    let created = request_json(server.port, "POST", "/session", Some("{}"));
    assert_eq!(created.status, 201, "create session: {}", created.body);
    let session: serde_json::Value =
        serde_json::from_str(&created.body).expect("created session JSON");
    let session_id = session["id"].as_str().expect("session id");
    let body = serde_json::json!({ "text": prompt }).to_string();
    let response = request_json(
        server.port,
        "POST",
        &format!("/session/{session_id}/message"),
        Some(&body),
    );
    assert_eq!(
        response.status, 200,
        "message response status/body: {} {}",
        response.status, response.body
    );
}

fn cortex_command(scratch: &Scratch, args: &[&str]) -> Command {
    let mut command = Command::new(binary());
    command.args(args);
    command.current_dir(&scratch.project);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cortex"))
}

struct FakeOpenRouter {
    base_url: String,
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeOpenRouter {
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
                        thread::sleep(Duration::from_millis(20));
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

impl Drop for FakeOpenRouter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct HttpResponse {
    status: u16,
    body: String,
}

fn request_json(port: u16, method: &str, path: &str, body: Option<&str>) -> HttpResponse {
    request_http(port, method, path, body).unwrap_or_else(|err| {
        panic!("HTTP request {method} {path} to port {port} failed: {err}");
    })
}

fn request_http(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<HttpResponse> {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
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
    let mut last_error = String::new();
    while started.elapsed() < RUN_TIMEOUT {
        if let Some(status) = child.try_wait().expect("try wait") {
            let mut stderr = String::new();
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_string(&mut stderr);
            }
            panic!("server exited before health on port {port}: {status}\n{stderr}");
        }
        match request_http(port, "GET", "/health", None) {
            Ok(response) if response.status == 200 => return,
            Ok(response) => last_error = format!("HTTP {}", response.status),
            Err(err) => last_error = err.to_string(),
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("server on port {port} did not become healthy: {last_error}");
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
            panic!("server child did not exit within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(50));
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
        many => panic!("expected a single listener pid for {port}, got {many:?}"),
    }
}

fn free_port() -> u16 {
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
    panic!("no free port in 18765-18864");
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
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return buffer;
        }
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
        let read = stream.read(&mut chunk).expect("read body");
        if read == 0 {
            break;
        }
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

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "01234567".repeat(8))
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
            "cortex-pure-kill-switch-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
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
