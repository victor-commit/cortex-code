//! Black-box regression tests for live MCP tool-list changes reaching `cortex run`.
//!
//! The shipped path is still the real CLI, real MCP remote transport, and the real OpenAI-compatible
//! HTTP adapter. The local provider below is a deterministic loopback fixture used only in tests so
//! we can force multi-step tool continuations and inspect the exact tool definitions sent on each
//! provider request.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
const CONTROL_SETTLE: Duration = Duration::from_millis(250);

static NEXT_PORT: AtomicU16 = AtomicU16::new(3240);

#[derive(Clone, Copy)]
enum Scenario {
    AddOnly,
    AddThenRemove,
}

struct Scratch {
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cortex-mcp-live-toolset-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create scratch home");
        std::fs::create_dir_all(&project).expect("create scratch project");
        Self {
            home,
            project,
            db: root.join("data/cortex.db"),
        }
    }
}

struct RemoteMcpFixture {
    child: Child,
}

impl RemoteMcpFixture {
    fn spawn(port: u16) -> Self {
        let mut command = Command::new(remote_server_bin());
        command
            .env("CORTEX_MCP_MODE", "streamable")
            .env("CORTEX_MCP_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().expect("spawn remote MCP fixture");
        wait_for_port(port);
        Self { child }
    }
}

impl Drop for RemoteMcpFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ProviderFixture {
    port: u16,
    captures: Arc<Mutex<Vec<Value>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProviderFixture {
    fn spawn(mcp_port: u16, scenario: Scenario) -> Self {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral provider loopback fixture");
        let port = listener.local_addr().expect("local addr").port();
        let captures = Arc::new(Mutex::new(Vec::new()));
        let thread_captures = captures.clone();
        let thread = thread::spawn(move || {
            let requests = match scenario {
                Scenario::AddOnly => 2,
                Scenario::AddThenRemove => 3,
            };
            for round in 0..requests {
                let (mut stream, _) = listener.accept().expect("accept provider request");
                let body = read_http_body(&mut stream);
                let request: Value = serde_json::from_slice(&body).expect("provider request JSON");
                thread_captures.lock().unwrap().push(request);

                match (scenario, round) {
                    (Scenario::AddOnly | Scenario::AddThenRemove, 0) => {
                        post_control(mcp_port, "/control/add-tool");
                        thread::sleep(CONTROL_SETTLE);
                        write_sse_response(&mut stream, &tool_call_frames("call_1", "api_echo"));
                    }
                    (Scenario::AddThenRemove, 1) => {
                        post_control(mcp_port, "/control/remove-tool");
                        thread::sleep(CONTROL_SETTLE);
                        write_sse_response(&mut stream, &tool_call_frames("call_2", "api_echo"));
                    }
                    _ => {
                        write_sse_response(&mut stream, &final_answer_frames());
                    }
                }
            }
        });
        Self {
            port,
            captures,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn captured_requests(&self) -> Vec<Value> {
        self.captures.lock().unwrap().clone()
    }
}

impl Drop for ProviderFixture {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn format_json_uses_the_refreshed_mcp_tool_definitions_after_add() {
    let mcp_port = free_port();
    let _mcp = RemoteMcpFixture::spawn(mcp_port);
    let provider = ProviderFixture::spawn(mcp_port, Scenario::AddOnly);
    let scratch = Scratch::new("json-add");
    write_mcp_config(&scratch, mcp_port);

    let out = run(cortex_in(
        &scratch,
        &provider,
        &["run", "--format", "json", "exercise live MCP add"],
    ));
    assert_success(&out);

    let requests = provider.captured_requests();
    assert_eq!(requests.len(), 2, "the provider should see two round-trips");
    assert!(
        !tool_names(&requests[0]).contains(&"api_extra".to_owned()),
        "the initial provider request must not advertise the runtime-added MCP tool"
    );
    assert!(
        tool_names(&requests[1]).contains(&"api_extra".to_owned()),
        "the continuation provider request must advertise the added MCP tool"
    );

    let doc: Value = serde_json::from_slice(&out.stdout).expect("strict JSON document");
    assert!(
        doc_tool_names(&doc).contains(&"api_extra".to_owned()),
        "`--format json` must expose the refreshed tool names"
    );
    assert!(
        doc["tool_definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|def| {
                def["name"] == "api_extra"
                    && def["description"]
                        .as_str()
                        .is_some_and(|description| description.contains("runtime"))
            }),
        "`--format json` must expose the refreshed MCP tool definition"
    );
}

#[test]
fn jsonl_emits_refreshed_mcp_tool_definitions_for_add_and_remove() {
    let mcp_port = free_port();
    let _mcp = RemoteMcpFixture::spawn(mcp_port);
    let provider = ProviderFixture::spawn(mcp_port, Scenario::AddThenRemove);
    let scratch = Scratch::new("jsonl-add-remove");
    write_mcp_config(&scratch, mcp_port);

    let out = run(cortex_in(
        &scratch,
        &provider,
        &["run", "--jsonl", "exercise live MCP add and remove"],
    ));
    assert_success(&out);

    let requests = provider.captured_requests();
    assert_eq!(
        requests.len(),
        3,
        "the provider should see three round-trips"
    );
    let request_tools = requests.iter().map(tool_names).collect::<Vec<_>>();
    assert!(
        !request_tools[0].contains(&"api_extra".to_owned()),
        "initial request unexpectedly advertised api_extra: {:?}",
        request_tools[0]
    );
    assert!(
        request_tools[1].contains(&"api_extra".to_owned()),
        "second request did not advertise the added api_extra tool: {:?}",
        request_tools[1]
    );
    assert!(
        !request_tools[2].contains(&"api_extra".to_owned()),
        "third request should reflect removal of api_extra: {:?}",
        request_tools[2]
    );

    let stdout = String::from_utf8(out.stdout).expect("utf8 JSONL");
    let events = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("each JSONL line parses"))
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["type"], "session");
    assert_eq!(events.last().unwrap()["type"], "done");

    let changed = events
        .iter()
        .filter(|event| event["type"] == "tool_set_changed")
        .collect::<Vec<_>>();
    assert!(
        changed
            .iter()
            .any(|event| doc_tool_names(event).contains(&"api_extra".to_owned())),
        "JSONL must emit a refreshed tool-set event when api_extra is added: {stdout}"
    );
    assert!(
        changed
            .iter()
            .any(|event| !doc_tool_names(event).contains(&"api_extra".to_owned())),
        "JSONL must emit a refreshed tool-set event when api_extra is removed: {stdout}"
    );
}

fn cortex_in(scratch: &Scratch, provider: &ProviderFixture, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", provider.base_url());
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "cortex run failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_mcp_config(scratch: &Scratch, mcp_port: u16) {
    let config = format!(
        r#"[mcp.api]
type = "remote"
url = "http://127.0.0.1:{mcp_port}/mcp"
"#
    );
    std::fs::write(scratch.project.join("cortex.toml"), config).expect("write MCP config");
}

fn remote_server_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(resolve_remote_server_bin).clone()
}

fn resolve_remote_server_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_cortex-mcp-remote-server") {
        return PathBuf::from(path);
    }
    let current = std::env::current_exe().expect("current test exe");
    let debug_dir = current
        .parent()
        .and_then(|deps| deps.parent())
        .expect("target debug dir");
    let bin = debug_dir.join("cortex-mcp-remote-server");
    if bin.exists() {
        return bin;
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest_dir
        .parent()
        .and_then(|crates| crates.parent())
        .expect("workspace root");
    let status = Command::new(env!("CARGO"))
        .current_dir(workspace)
        .args(["build", "-p", "mcp", "--bin", "cortex-mcp-remote-server"])
        .status()
        .expect("build remote MCP fixture binary");
    assert!(
        status.success(),
        "failed to build cortex-mcp-remote-server fixture"
    );
    assert!(
        bin.exists(),
        "remote MCP fixture binary was not produced at {}",
        bin.display()
    );
    bin
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}

fn free_port() -> u16 {
    for _ in 0..60 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > 3299 {
            NEXT_PORT.store(3240, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}")
            .parse()
            .expect("valid loopback addr");
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }
    panic!("no free port available in 3240-3299");
}

fn wait_for_port(port: u16) {
    let addr: SocketAddr = format!("127.0.0.1:{port}")
        .parse()
        .expect("valid loopback addr");
    for _ in 0..200 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(25)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("fixture never listened on 127.0.0.1:{port}");
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let n = stream.read(&mut buffer).expect("read HTTP request");
        assert_ne!(n, 0, "connection closed before request body was complete");
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(header_end) = find_header_end(&bytes) {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = content_length(&headers);
            let body_start = header_end + 4;
            if bytes.len() >= body_start + content_length {
                return bytes[body_start..body_start + content_length].to_vec();
            }
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().expect("valid content-length"))
        })
        .expect("content-length header")
}

fn write_sse_response(stream: &mut TcpStream, frames: &[String]) {
    let body = frames.join("");
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .expect("write provider SSE response");
    stream.flush().expect("flush provider response");
}

fn tool_call_frames(id: &str, name: &str) -> Vec<String> {
    let arguments = json!({ "message": format!("hello from {id}") }).to_string();
    vec![
        format!(
            "data: {}\n\n",
            json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": id,
                            "function": {
                                "name": name,
                                "arguments": arguments
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })
        ),
        "data: [DONE]\n\n".to_owned(),
    ]
}

fn final_answer_frames() -> Vec<String> {
    vec![
        format!(
            "data: {}\n\n",
            json!({
                "choices": [{
                    "delta": { "content": "MCP live toolset refreshed." },
                    "finish_reason": "stop"
                }]
            })
        ),
        "data: [DONE]\n\n".to_owned(),
    ]
}

fn post_control(port: u16, path: &str) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect control endpoint");
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("write control request");
    stream.flush().expect("flush control request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read control response");
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "control endpoint failed: {response}"
    );
}

fn tool_names(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .map(ToOwned::to_owned)
        .collect()
}

fn doc_tool_names(doc: &Value) -> Vec<String> {
    doc["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}
