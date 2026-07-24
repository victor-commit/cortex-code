//! Black-box checks for the `cortex mcp` command surface that exercise the remote MCP transport
//! and the MCP-remote OAuth credential lifecycle through the real CLI binary.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);
const NET_DISABLED_CODE: i32 = 4;

fn cortex_bin() -> String {
    env!("CARGO_BIN_EXE_cortex").to_owned()
}

fn remote_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-remote-server");
    bin.exists().then_some(bin)
}

fn echo_server_bin() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?.parent()?.join("cortex-mcp-echo-server");
    bin.exists().then_some(bin)
}

fn toml_string(value: &Path) -> String {
    value
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn temp_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cortex-mcp-cli-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = temp_path(tag);
        fs::create_dir_all(root.join("home/proj")).expect("scratch project");
        fs::create_dir_all(root.join("data")).expect("scratch data");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn project(&self) -> PathBuf {
        self.root.join("home/proj")
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn write_project_config(&self, body: &str) {
        fs::write(self.project().join("cortex.toml"), body).expect("write cortex.toml");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Fixture {
    child: Child,
}

impl Fixture {
    fn spawn(mode: &str, port: u16, log: &Path) -> Option<Self> {
        let bin = remote_server_bin()?;
        let mut command = Command::new(bin);
        command
            .env("CORTEX_MCP_MODE", mode)
            .env("CORTEX_MCP_PORT", port.to_string())
            .env("CORTEX_MCP_LOG", log)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().expect("spawn remote MCP fixture");
        wait_for_port(port);
        Some(Self { child })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const OPAQUE_AUTH_CHALLENGE: &str =
    "Bearer realm=\"fixture\", opaque_challenge=\"challenge-must-not-leak\"";

/// A self-contained loopback fixture that proves the remote MCP probe executes an actual HTTP
/// `401` response. It serves a fixed number of initialize requests and joins its worker before
/// the test returns, so no listener can outlive the regression test.
struct NeedsAuthFixture {
    url: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl NeedsAuthFixture {
    fn start(expected_requests: usize) -> Self {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral needs-auth fixture");
        listener
            .set_nonblocking(true)
            .expect("set needs-auth fixture listener nonblocking");
        let port = listener
            .local_addr()
            .expect("needs-auth fixture local address")
            .port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_thread = shutdown.clone();
        let handle = std::thread::spawn(move || {
            for request_number in 1..=expected_requests {
                let Some(mut stream) = accept_fixture_connection(&listener, &shutdown_thread)?
                else {
                    return Ok(());
                };
                let request = read_http_request(&mut stream)
                    .map_err(|err| format!("read fixture request {request_number}: {err}"))?;
                assert_initialize_probe(&request, request_number)?;
                stream
                    .write_all(unauthorized_response().as_bytes())
                    .map_err(|err| format!("write fixture 401 response {request_number}: {err}"))?;
                stream
                    .flush()
                    .map_err(|err| format!("flush fixture 401 response {request_number}: {err}"))?;
            }
            Ok(())
        });

        Self {
            url: format!("http://127.0.0.1:{port}/mcp"),
            shutdown,
            handle: Some(handle),
        }
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn finish(mut self) {
        self.join()
            .expect("needs-auth fixture should serve every MCP probe");
    }

    fn join(&mut self) -> Result<(), String> {
        self.handle
            .take()
            .expect("needs-auth fixture handle")
            .join()
            .map_err(|_| "needs-auth fixture worker panicked".to_owned())?
    }
}

impl Drop for NeedsAuthFixture {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if self.handle.is_some() {
            let _ = self.join();
        }
    }
}

fn accept_fixture_connection(
    listener: &TcpListener,
    shutdown: &AtomicBool,
) -> Result<Option<TcpStream>, String> {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        if shutdown.load(Ordering::Acquire) {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, _)) => return Ok(Some(stream)),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out waiting for an MCP probe".to_owned());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(format!("accept MCP probe: {err}")),
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];
    let headers_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(request);
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..headers_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut remaining = content_length.saturating_sub(request.len() - headers_end);
    while remaining > 0 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    Ok(request)
}

fn assert_initialize_probe(request: &[u8], request_number: usize) -> Result<(), String> {
    let request = std::str::from_utf8(request)
        .map_err(|err| format!("fixture request {request_number} was not UTF-8: {err}"))?;
    let (headers, body) = request
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("fixture request {request_number} omitted HTTP headers"))?;
    if !headers
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST /mcp "))
    {
        return Err(format!(
            "fixture request {request_number} should be an MCP POST, got {headers:?}"
        ));
    }
    let method = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|body| body["method"].as_str().map(str::to_owned));
    if method.as_deref() != Some("initialize") {
        return Err(format!(
            "fixture request {request_number} should initialize MCP, got {method:?}"
        ));
    }
    Ok(())
}

fn unauthorized_response() -> String {
    format!(
        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {OPAQUE_AUTH_CHALLENGE}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn wait_for_port(port: u16) {
    let start = Instant::now();
    while start.elapsed() < RUN_TIMEOUT {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("fixture did not listen on 127.0.0.1:{port}");
}

fn run_command(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn command");
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command did not exit within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

fn cortex_command(scratch: &Scratch, args: &[&str]) -> Command {
    let mut command = Command::new(cortex_bin());
    command
        .args(args)
        .env_clear()
        .env("HOME", scratch.home())
        .env("CORTEX_DATA_DIR", scratch.data())
        .current_dir(scratch.project());
    command
}

fn mcp_list_json(scratch: &Scratch) -> Value {
    let output = run_command(cortex_command(
        scratch,
        &["mcp", "list", "--format", "json"],
    ));
    assert_eq!(
        output.status.code(),
        Some(0),
        "mcp list failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("mcp list JSON")
}

/// Whether `strace` can trace this process tree in the current environment.
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

fn strace_connect_trace(scratch: &Scratch, args: &[&str], no_net: bool) -> String {
    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=connect", env!("CARGO_BIN_EXE_cortex")])
        .args(args)
        .env_clear()
        .env("HOME", scratch.home())
        .env("CORTEX_DATA_DIR", scratch.data())
        .current_dir(scratch.project());
    if no_net {
        command.env("CORTEX_NO_NET", "1");
    }
    let output = command.output().expect("spawn strace");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn count_connect(trace: &str) -> usize {
    trace.matches("connect(").count()
}

fn pid_alive(pid: u64) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn assert_pid_reaped(pid: u64) {
    for _ in 0..50 {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!pid_alive(pid), "MCP child pid {pid} should be reaped");
}

fn read_first_json_line(reader: &mut impl Read) -> Value {
    let mut line = Vec::new();
    let start = Instant::now();
    while start.elapsed() < RUN_TIMEOUT {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => line.push(byte[0]),
            Err(err) => panic!("read auth stdout: {err}"),
        }
    }
    serde_json::from_slice(&line).unwrap_or_else(|err| {
        panic!(
            "auth command should emit a JSON document first: {err}; line: {}",
            String::from_utf8_lossy(&line)
        )
    })
}

fn http_get_follow_once(url: &str) -> String {
    let first = http_get(url);
    if first.status == 302 {
        let location = first
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.clone())
            .expect("redirect location");
        return http_get(&location).body;
    }
    first.body
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn http_get(url: &str) -> HttpResponse {
    let rest = url
        .strip_prefix("http://")
        .expect("only loopback http URLs");
    let (authority, target) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse::<u16>().expect("port")))
        .unwrap_or((authority, 80));
    let mut stream = TcpStream::connect((host, port)).expect("connect http server");
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write http request");
    let mut raw_bytes = Vec::new();
    loop {
        let mut buffer = [0u8; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => raw_bytes.extend_from_slice(&buffer[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(err) => panic!("read http response: {err}"),
        }
    }
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .expect("status code");
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    HttpResponse {
        status,
        headers,
        body: body.to_owned(),
    }
}

#[test]
fn mcp_list_remote_fails_closed_under_no_net_before_connecting() {
    let scratch = Scratch::new("list-no-net");
    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:18864/mcp"
"#,
    );

    let mut command = cortex_command(&scratch, &["mcp", "list", "--format", "json"]);
    command.env("CORTEX_NO_NET", "1");
    let output = run_command(command);
    assert_eq!(
        output.status.code(),
        Some(NET_DISABLED_CODE),
        "mcp list should fail closed with the net-disabled exit code"
    );
    assert!(
        output.stdout.is_empty(),
        "a fail-closed refusal keeps stdout empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("network disabled") && stderr.contains("CORTEX_NO_NET"),
        "stderr should report a net-disabled refusal: {stderr}"
    );

    if strace_usable() {
        let trace = strace_connect_trace(&scratch, &["mcp", "list", "--format", "json"], true);
        assert_eq!(
            count_connect(&trace),
            0,
            "mcp list under CORTEX_NO_NET=1 must issue zero connect() syscalls:\n{trace}"
        );
    }
}

#[test]
fn mcp_auth_remote_fails_closed_under_no_net_before_discovery() {
    let scratch = Scratch::new("auth-no-net");
    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:18864/mcp"
"#,
    );

    let mut command = cortex_command(
        &scratch,
        &["mcp", "auth", "api", "--format", "json", "--timeout", "1"],
    );
    command.env("CORTEX_NO_NET", "1");
    let output = run_command(command);
    assert_eq!(
        output.status.code(),
        Some(NET_DISABLED_CODE),
        "mcp auth should fail closed with the net-disabled exit code"
    );
    assert!(
        output.stdout.is_empty(),
        "a fail-closed refusal keeps stdout empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("network disabled") && stderr.contains("CORTEX_NO_NET"),
        "stderr should report a net-disabled refusal: {stderr}"
    );

    if strace_usable() {
        let trace = strace_connect_trace(
            &scratch,
            &["mcp", "auth", "api", "--format", "json", "--timeout", "1"],
            true,
        );
        assert_eq!(
            count_connect(&trace),
            0,
            "mcp auth under CORTEX_NO_NET=1 must issue zero connect() syscalls:\n{trace}"
        );
    }
}

#[test]
fn mcp_list_applies_configured_cwd_env_and_strips_parent_secrets() {
    let Some(bin) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture not built");
        return;
    };
    let scratch = Scratch::new("stdio-env");
    let server_cwd = scratch.root.join("server-cwd");
    fs::create_dir_all(&server_cwd).expect("server cwd");
    let log = scratch.root.join("echo.log");
    scratch.write_project_config(&format!(
        r#"
[mcp.demo]
type = "local"
command = ["{}"]
cwd = "{}"
[mcp.demo.environment]
CORTEX_MCP_ECHO_LOG = "{}"
CORTEX_MCP_ECHO_CONFIGURED_FLAG = "configured-ok"
CORTEX_MCP_ECHO_STARTUP_ENV_NAME = "CORTEX_MCP_PARENT_SECRET"
"#,
        toml_string(&bin),
        toml_string(&server_cwd),
        toml_string(&log)
    ));

    let mut command = cortex_command(&scratch, &["mcp", "list", "--format", "json"]);
    command
        .env("CORTEX_MCP_PARENT_SECRET", "plain-parent-secret")
        .env("ANTHROPIC_API_KEY", "plain-anthropic-secret")
        .env("LEAKED_COPY", fake_openrouter_key());
    let output = run_command(command);
    assert_eq!(
        output.status.code(),
        Some(0),
        "mcp list failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let list: Value = serde_json::from_slice(&output.stdout).expect("mcp list JSON");
    let server = &list["servers"][0];
    assert_eq!(server["status"], "connected", "server connected: {list}");
    let pid = server["pid"].as_u64().expect("stdio pid is reported");

    let log_body = fs::read_to_string(&log).expect("configured env log is written");
    assert!(
        log_body.contains(&format!("cwd={}", server_cwd.display())),
        "configured cwd should reach the child startup log: {log_body}"
    );
    assert!(
        log_body.contains("CORTEX_MCP_ECHO_CONFIGURED_FLAG=configured-ok"),
        "configured environment entries should reach the child: {log_body}"
    );
    assert!(
        log_body.contains("CORTEX_MCP_PARENT_SECRET=(unset)"),
        "synthetic parent secret should be stripped: {log_body}"
    );
    assert!(
        !log_body.contains("plain-parent-secret")
            && !log_body.contains("plain-anthropic-secret")
            && !log_body.contains("deadbeef"),
        "parent secret values must not reach the MCP fixture log: {log_body}"
    );
    assert_pid_reaped(pid);
}

#[test]
fn mcp_list_leaves_disabled_stdio_entries_unspawned() {
    let Some(bin) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture not built");
        return;
    };
    let scratch = Scratch::new("disabled");
    let log = scratch.root.join("disabled.log");
    scratch.write_project_config(&format!(
        r#"
[mcp.off]
type = "local"
command = ["{}"]
enabled = false
[mcp.off.environment]
CORTEX_MCP_ECHO_LOG = "{}"
"#,
        toml_string(&bin),
        toml_string(&log)
    ));

    let list = mcp_list_json(&scratch);
    let server = &list["servers"][0];
    assert_eq!(server["name"], "off");
    assert_eq!(server["status"], "disabled");
    assert!(
        server["pid"].is_null(),
        "disabled server has no PID: {list}"
    );
    assert!(
        server["tools"].as_array().unwrap().is_empty(),
        "disabled server exposes no tools: {list}"
    );
    assert!(!log.exists(), "disabled server must never be spawned");
}

#[test]
fn mcp_add_persists_hyphen_prefixed_command_vector_and_options() {
    let scratch = Scratch::new("add-hyphen");
    let output = run_command(cortex_command(
        &scratch,
        &[
            "mcp",
            "add",
            "demo",
            "--cwd",
            "/tmp/server-work",
            "--env",
            "FOO=bar",
            "--disabled",
            "--force",
            "--format",
            "json",
            "--command",
            "npx",
            "-y",
            "@scope/server",
            "--stdio",
        ],
    ));
    assert_eq!(
        output.status.code(),
        Some(0),
        "mcp add failed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).expect("mcp add JSON");
    assert_eq!(document["status"], "added");

    let config_path = scratch.home().join(".config/cortex/config.toml");
    let body = fs::read_to_string(&config_path).expect("global config written");
    let parsed: toml::Value = toml::from_str(&body).expect("valid TOML");
    let demo = parsed
        .get("mcp")
        .and_then(|mcp| mcp.get("demo"))
        .expect("demo server persisted");
    let command = demo
        .get("command")
        .and_then(toml::Value::as_array)
        .expect("command array")
        .iter()
        .map(|value| value.as_str().expect("string arg").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        command,
        vec![
            "npx".to_owned(),
            "-y".to_owned(),
            "@scope/server".to_owned(),
            "--stdio".to_owned()
        ],
        "full command vector should be persisted exactly: {body}"
    );
    assert_eq!(
        demo.get("cwd").and_then(toml::Value::as_str),
        Some("/tmp/server-work")
    );
    assert_eq!(
        demo.get("environment")
            .and_then(|env| env.get("FOO"))
            .and_then(toml::Value::as_str),
        Some("bar")
    );
    assert_eq!(
        demo.get("enabled").and_then(toml::Value::as_bool),
        Some(false)
    );
}

#[test]
fn mcp_list_connects_remote_streamable_and_sse_fallback() {
    let Some(_bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("list");
    let http_log = scratch.root.join("http.log");
    let sse_log = scratch.root.join("sse.log");
    let _http = Fixture::spawn("streamable", 3224, &http_log).expect("http fixture");
    let _sse = Fixture::spawn("sse", 3225, &sse_log).expect("sse fixture");
    scratch.write_project_config(
        r#"
[mcp.http]
type = "remote"
url = "http://127.0.0.1:3224/mcp"

[mcp.legacy]
type = "remote"
url = "http://127.0.0.1:3225/sse"
"#,
    );

    let list = mcp_list_json(&scratch);
    let servers = list["servers"].as_array().expect("servers array");
    let by_name = |name: &str| {
        servers
            .iter()
            .find(|server| server["name"] == name)
            .unwrap_or_else(|| panic!("missing server {name}: {list}"))
    };

    let http = by_name("http");
    assert_eq!(http["status"], "connected");
    assert!(
        http["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "http_add")
    );
    let legacy = by_name("legacy");
    assert_eq!(legacy["status"], "connected");
    assert!(
        legacy["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "legacy_echo")
    );

    assert!(
        fs::read_to_string(&http_log)
            .unwrap_or_default()
            .contains("HTTP MCP session established (streamable)")
    );
    assert!(
        fs::read_to_string(&sse_log)
            .unwrap_or_default()
            .contains("SSE endpoint opened")
    );
}

#[test]
fn mcp_auth_authorizes_persists_reuses_and_does_not_leak_token() {
    let Some(_bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("oauth");
    let log = scratch.root.join("oauth.log");
    let _fixture = Fixture::spawn("oauth", 3226, &log).expect("oauth fixture");
    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:3226/mcp"
"#,
    );

    let before = mcp_list_json(&scratch);
    assert_eq!(before["servers"][0]["status"], "needs_auth");

    let mut auth = cortex_command(
        &scratch,
        &[
            "mcp",
            "auth",
            "api",
            "--callback-port",
            "3227",
            "--timeout",
            "10",
            "--format",
            "json",
        ],
    );
    let mut child = auth
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp auth");
    let mut stdout = child.stdout.take().expect("auth stdout");
    let mut stderr = child.stderr.take().expect("auth stderr");
    let start = read_first_json_line(&mut stdout);
    let authorize_url = start["authorize_url"]
        .as_str()
        .expect("authorize_url")
        .to_owned();

    let callback_body = http_get_follow_once(&authorize_url);
    assert!(
        callback_body.contains("Authorization complete"),
        "callback body: {callback_body}"
    );

    let status = child.wait().expect("wait mcp auth");
    assert_eq!(status.code(), Some(0), "mcp auth must exit successfully");
    let mut auth_stdout_rest = Vec::new();
    let mut auth_stderr = Vec::new();
    let _ = stdout.read_to_end(&mut auth_stdout_rest);
    let _ = stderr.read_to_end(&mut auth_stderr);

    let store = scratch.data().join("mcp-oauth.json");
    let mode = fs::metadata(&store)
        .expect("credential store exists")
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600, "store must be 0600");
    }

    let after = mcp_list_json(&scratch);
    assert_eq!(after["servers"][0]["status"], "connected");
    assert!(
        after["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "api_add")
    );

    let store_body = fs::read_to_string(&store).expect("read credential store");
    let token =
        serde_json::from_str::<Value>(&store_body).expect("credential JSON")["api"]["access_token"]
            .as_str()
            .expect("access token")
            .to_owned();
    let mut observed = Vec::new();
    observed.extend_from_slice(start.to_string().as_bytes());
    observed.extend_from_slice(&auth_stdout_rest);
    observed.extend_from_slice(&auth_stderr);
    observed.extend_from_slice(before.to_string().as_bytes());
    observed.extend_from_slice(after.to_string().as_bytes());
    observed.extend_from_slice(fs::read_to_string(&log).unwrap_or_default().as_bytes());
    assert!(
        !String::from_utf8_lossy(&observed).contains(&token),
        "OAuth access token must never appear in CLI output or logs"
    );
}

#[test]
fn mcp_list_surfaces_needs_auth_without_exposing_a_real_oauth_challenge() {
    let scratch = Scratch::new("needs-auth-safe-status");
    let fixture = NeedsAuthFixture::start(2);

    scratch.write_project_config(&format!(
        r#"
[mcp.api]
type = "remote"
url = "{}"
"#,
        fixture.url()
    ));

    let json = run_command(cortex_command(
        &scratch,
        &["mcp", "list", "--format", "json"],
    ));
    let text = run_command(cortex_command(&scratch, &["mcp", "list"]));
    fixture.finish();

    assert_eq!(json.status.code(), Some(0));
    assert_eq!(text.status.code(), Some(0));
    let document: Value = serde_json::from_slice(&json.stdout).expect("mcp list JSON");
    assert_eq!(document["servers"][0]["status"], "needs_auth");
    let observed = format!(
        "{}\n{}\n{}\n{}",
        String::from_utf8_lossy(&json.stdout),
        String::from_utf8_lossy(&json.stderr),
        String::from_utf8_lossy(&text.stdout),
        String::from_utf8_lossy(&text.stderr),
    );
    assert!(
        observed.contains("authorization required"),
        "both surfaces retain the safe actionable state: {observed}"
    );
    assert!(
        !observed.contains(OPAQUE_AUTH_CHALLENGE) && !observed.contains("challenge-must-not-leak"),
        "OAuth challenges must not appear in CLI JSON or text: {observed}"
    );
}

#[test]
fn mcp_list_redacts_an_opaque_oauth_token_reflected_by_a_server_error() {
    let Some(bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("opaque-oauth-token");
    let log = scratch.root.join("opaque-oauth-token.log");
    let token = "opaque-oauth-access-token-value";
    let mut fixture = Command::new(bin);
    fixture
        .env("CORTEX_MCP_MODE", "streamable")
        .env("CORTEX_MCP_PORT", "3237")
        .env("CORTEX_MCP_LOG", &log)
        .env(
            "CORTEX_MCP_REQUIRE_AUTHORIZATION",
            format!("Bearer {token}"),
        )
        .env("CORTEX_MCP_ECHO_AUTHORIZATION_IN_ERROR", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut fixture = fixture.spawn().expect("spawn remote MCP fixture");
    wait_for_port(3237);

    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:3237/mcp"
"#,
    );
    fs::create_dir_all(scratch.data()).expect("credential store directory");
    fs::write(
        scratch.data().join("mcp-oauth.json"),
        format!(
            r#"{{"api":{{"access_token":"{token}","token_endpoint":"http://127.0.0.1:3237/token","client_id":"cortex"}}}}"#
        ),
    )
    .expect("write opaque OAuth credential");

    let output = run_command(cortex_command(
        &scratch,
        &["mcp", "list", "--format", "json"],
    ));
    let _ = fixture.kill();
    let _ = fixture.wait();

    assert_eq!(
        output.status.code(),
        Some(0),
        "mcp list should report failure"
    );
    let observed = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !observed.contains(token),
        "an opaque OAuth token reflected by an MCP error must never escape:\n{observed}"
    );
    assert!(
        observed.contains(redaction::PLACEHOLDER),
        "the reflected opaque OAuth token must be replaced by the canonical placeholder:\n{observed}"
    );
}

#[test]
fn mcp_auth_honors_oauth_config_overrides_without_cli_mutation() {
    let Some(_bin) = remote_server_bin() else {
        eprintln!("skipping: cortex-mcp-remote-server fixture not built");
        return;
    };
    let scratch = Scratch::new("oauth-overrides");
    let log = scratch.root.join("oauth-overrides.log");
    let _fixture = Fixture::spawn("oauth", 3231, &log).expect("oauth fixture");
    scratch.write_project_config(
        r#"
[mcp.api]
type = "remote"
url = "http://127.0.0.1:3231/mcp"
[mcp.api.oauth]
client_id = "fixture-client"
scopes = ["openid", "profile"]
callback_port = 3232
"#,
    );

    let mut auth = cortex_command(
        &scratch,
        &["mcp", "auth", "api", "--timeout", "10", "--format", "json"],
    );
    let mut child = auth
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp auth");
    let mut stdout = child.stdout.take().expect("auth stdout");
    let mut stderr = child.stderr.take().expect("auth stderr");
    let start = read_first_json_line(&mut stdout);
    let authorize_url = start["authorize_url"].as_str().expect("authorize_url");
    let redirect_uri = start["redirect_uri"].as_str().expect("redirect_uri");
    assert_eq!(redirect_uri, "http://127.0.0.1:3232/callback");
    assert!(
        authorize_url.contains("client_id=fixture-client"),
        "authorize URL must carry configured client_id, got: {authorize_url}"
    );
    assert!(
        authorize_url.contains("scope=openid")
            && (authorize_url.contains("%20profile") || authorize_url.contains("+profile")),
        "authorize URL must carry configured scopes, got: {authorize_url}"
    );

    let callback_body = http_get_follow_once(authorize_url);
    assert!(
        callback_body.contains("Authorization complete"),
        "callback body: {callback_body}"
    );

    let status = child.wait().expect("wait mcp auth");
    assert_eq!(status.code(), Some(0), "mcp auth must exit successfully");
    let mut auth_stdout_rest = Vec::new();
    let mut auth_stderr = Vec::new();
    let _ = stdout.read_to_end(&mut auth_stdout_rest);
    let _ = stderr.read_to_end(&mut auth_stderr);
    assert!(
        auth_stderr.is_empty(),
        "auth stderr should stay clean: {}",
        String::from_utf8_lossy(&auth_stderr)
    );
    let list = mcp_list_json(&scratch);
    assert_eq!(list["servers"][0]["status"], "connected");
    assert!(
        list["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == "api_add")
    );
}

#[test]
fn mcp_list_reports_resources_and_prompts() {
    let Some(bin) = echo_server_bin() else {
        eprintln!("skipping: cortex-mcp-echo-server fixture not built");
        return;
    };
    let scratch = Scratch::new("list-resources-prompts");
    scratch.write_project_config(&format!(
        r#"
[mcp.demo]
type = "local"
command = ["{}"]
[mcp.demo.environment]
CORTEX_MCP_ECHO_ENABLE_RESOURCES_PROMPTS = "1"
CORTEX_MCP_ECHO_RESOURCE_CONTENT = "RESOURCE-CONTENT-CLI"
"#,
        toml_string(&bin)
    ));

    let list = mcp_list_json(&scratch);
    let server = &list["servers"][0];
    assert_eq!(server["status"], "connected");
    assert!(
        server["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == "demo_add"),
        "tools are still listed: {list}"
    );
    assert!(
        server["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["uri"] == "notes://demo"),
        "resources must be enumerated in mcp list JSON: {list}"
    );
    assert!(
        server["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|prompt| prompt["name"] == "greet"),
        "prompts must be enumerated in mcp list JSON: {list}"
    );
}
