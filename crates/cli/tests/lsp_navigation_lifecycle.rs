//! Black-box LSP navigation and lifecycle coverage through the real `cortex run` tool surface.
//!
//! The provider fixture only drives tool calls. Every navigation answer comes from the locally
//! installed `rust-analyzer` language server, which Cortex launches and reaps by its exact PID.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(120);
static NEXT_PORT: AtomicU16 = AtomicU16::new(3280);
static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let unique = NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-lsp-tool-nav-{tag}-{}-{unique}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        fs::create_dir_all(project.join("src")).expect("create project source directory");
        fs::create_dir_all(&home).expect("create isolated home");
        Self {
            db: root.join("data/cortex.db"),
            root,
            home,
            project,
        }
    }

    fn write_rust_project(&self) {
        fs::write(
            self.project.join("Cargo.toml"),
            "[package]\nname = \"lsp_navigation_probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(
            self.project.join("src/lib.rs"),
            concat!(
                "pub fn add_one(input: i32) -> i32 {\n",
                "    input + 1\n",
                "}\n",
                "\n",
                "pub fn call_add() -> i32 {\n",
                "    add_one(41)\n",
                "}\n",
                "\n",
                "pub fn call_add_again() -> i32 {\n",
                "    add_one(1)\n",
                "}\n"
            ),
        )
        .expect("write Rust navigation fixture");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone)]
enum ProviderStep {
    Tool {
        call_id: &'static str,
        name: &'static str,
        arguments: Value,
    },
    Final,
}

struct ScriptedProvider {
    base_url: String,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScriptedProvider {
    fn start(
        steps: Vec<ProviderStep>,
        before_response: impl Fn(usize) + Send + Sync + 'static,
    ) -> Self {
        let (port, listener) = bind_provider_listener();
        listener
            .set_nonblocking(true)
            .expect("set provider listener nonblocking");
        let before_response = Arc::new(before_response);
        let thread = thread::spawn(move || {
            for (index, step) in steps.into_iter().enumerate() {
                let Some(mut stream) = accept_with_idle_timeout(&listener, Duration::from_secs(20))
                else {
                    panic!("provider did not receive request {index}");
                };
                let _ = read_http_body(&mut stream);
                before_response(index);
                let body = match step {
                    ProviderStep::Tool {
                        call_id,
                        name,
                        arguments,
                    } => tool_call_body(call_id, name, arguments),
                    ProviderStep::Final => final_body(),
                };
                write_sse_response(&mut stream, &body).expect("write provider response");
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            thread: Some(thread),
        }
    }

    fn finish(mut self) {
        self.thread
            .take()
            .expect("provider thread")
            .join()
            .expect("provider thread should not panic");
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn cortex_run_lsp_tool_returns_real_navigation_and_reaps_server_pid() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping real navigation proof: rust-analyzer is not on PATH");
        return;
    }

    let scratch = Arc::new(Scratch::new("navigation"));
    scratch.write_rust_project();
    let observed_status = Arc::new(Mutex::new(None));
    let status_scratch = scratch.clone();
    let status_slot = observed_status.clone();
    let provider = ScriptedProvider::start(
        vec![
            lsp_step("hover"),
            lsp_step("definition"),
            lsp_step("references"),
            ProviderStep::Tool {
                call_id: "call_document_symbols",
                name: "lsp",
                arguments: json!({"action":"documentSymbols","filePath":"src/lib.rs"}),
            },
            ProviderStep::Tool {
                call_id: "call_workspace_symbols",
                name: "lsp",
                arguments: json!({
                    "action":"workspaceSymbols",
                    "query":"add_one",
                    "filePath":"src/lib.rs"
                }),
            },
            ProviderStep::Final,
        ],
        move |index| {
            if index == 1 {
                *status_slot.lock().expect("status slot") = Some(lsp_status_json(&status_scratch));
            }
        },
    );

    let output = run(cortex_run(
        &scratch,
        &provider,
        "exercise every LSP navigation operation",
    ));
    let doc = assert_success_json(&output);
    provider.finish();

    let hover = tool_result_text(&doc, "lsp", 0);
    assert!(
        hover.contains("add_one") && hover.contains("i32"),
        "hover must contain real rust-analyzer signature information: {hover}"
    );

    let definition = tool_result_text(&doc, "lsp", 1);
    assert!(
        definition.contains("src/lib.rs:line 1"),
        "definition must point to the declaration on line 1: {definition}"
    );

    let references = tool_result_text(&doc, "lsp", 2);
    for line in [1, 6, 10] {
        assert!(
            references.contains(&format!("src/lib.rs:line {line}")),
            "references must include known use/declaration line {line}: {references}"
        );
    }

    let document_symbols = tool_result_text(&doc, "lsp", 3);
    assert!(
        document_symbols.contains("function add_one"),
        "document symbols must include add_one and its kind: {document_symbols}"
    );

    let workspace_symbols = tool_result_text(&doc, "lsp", 4);
    assert!(
        workspace_symbols.contains("add_one") && workspace_symbols.contains("src/lib.rs"),
        "workspace symbols must include add_one's location: {workspace_symbols}"
    );

    let rows = observed_status
        .lock()
        .expect("status slot")
        .take()
        .expect("status was sampled while Cortex owned the server");
    let rust = rows
        .iter()
        .find(|row| row["language"] == "rust")
        .expect("rust status row");
    assert_eq!(
        rust["state"], "running",
        "status must report running: {rust}"
    );
    let pid = rust["pid"].as_u64().expect("running status PID") as u32;
    assert_pid_gone(pid);
}

#[cfg(unix)]
#[test]
fn cortex_run_status_keeps_wrapper_hosted_lsp_running_by_owned_identity() {
    if !has_command("python3") {
        eprintln!("skipping wrapper LSP identity proof: python3 is not on PATH");
        return;
    }

    let scratch = Arc::new(Scratch::new("wrapper-runtime-identity"));
    let fixture = scratch.project.join("fixture.wrp");
    fs::write(&fixture, "wrapper-hosted language server fixture\n").expect("write LSP fixture");

    let server_script = scratch.project.join("fixture_lsp.py");
    fs::write(
        &server_script,
        r#"import json
import sys

def read_message():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line not in (b"\r\n", b"\n", b""):
        name, value = line.decode("ascii", "replace").split(":", 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    body = sys.stdin.buffer.read(length)
    return json.loads(body) if body else None

def send(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

while True:
    message = read_message()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize" and "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{"textDocumentSync":1}}})
    elif method == "shutdown" and "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":None})
    elif "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":[]})
"#,
    )
    .expect("write wrapper-hosted LSP");

    let wrapper = scratch.project.join("fixture-wrapper.sh");
    fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec python3 \"{}\"\n", server_script.display()),
    )
    .expect("write LSP wrapper");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(&wrapper)
        .expect("wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper, permissions).expect("make LSP wrapper executable");

    fs::write(
        scratch.project.join("cortex.toml"),
        format!(
            r#"
[lsp.servers.wrapperfixture]
program = "{}"
extensions = ["wrp"]
"#,
            wrapper.display()
        ),
    )
    .expect("write wrapper LSP configuration");

    let observed_status = Arc::new(Mutex::new(None));
    let status_scratch = scratch.clone();
    let status_slot = observed_status.clone();
    let provider = ScriptedProvider::start(
        vec![
            ProviderStep::Tool {
                call_id: "call_wrapper_symbols",
                name: "lsp",
                arguments: json!({
                    "action":"documentSymbols",
                    "filePath":"fixture.wrp"
                }),
            },
            ProviderStep::Final,
        ],
        move |index| {
            if index == 1 {
                *status_slot.lock().expect("status slot") = Some(lsp_status_json(&status_scratch));
            }
        },
    );

    let output = run(cortex_run(
        &scratch,
        &provider,
        "inspect symbols through the wrapper-hosted language server",
    ));
    assert_success_json(&output);
    provider.finish();

    let rows = observed_status
        .lock()
        .expect("status slot")
        .take()
        .expect("status was sampled while Cortex owned the wrapper-hosted server");
    let wrapper_status = rows
        .iter()
        .find(|row| row["language"] == "wrapperfixture")
        .expect("wrapper LSP status row");
    assert_eq!(
        wrapper_status["state"], "running",
        "status must rely on Cortex-owned PID identity after wrapper exec: {wrapper_status}"
    );
    let pid = wrapper_status["pid"]
        .as_u64()
        .expect("wrapper server status PID") as u32;
    assert_pid_gone(pid);
}

#[test]
fn cortex_run_missing_lsp_is_clean_and_write_still_persists() {
    let scratch = Scratch::new("missing");
    let provider = ScriptedProvider::start(
        vec![
            ProviderStep::Tool {
                call_id: "call_write_unknown",
                name: "write",
                arguments: json!({"path":"unserved.unknownext","content":"still persisted\n"}),
            },
            ProviderStep::Tool {
                call_id: "call_unknown_symbols",
                name: "lsp",
                arguments: json!({
                    "action":"documentSymbols",
                    "filePath":"unserved.unknownext"
                }),
            },
            ProviderStep::Final,
        ],
        |_| {},
    );

    let output = run(cortex_run(
        &scratch,
        &provider,
        "write a file and inspect its unavailable language server",
    ));
    let doc = assert_success_json(&output);
    provider.finish();

    assert_eq!(
        fs::read_to_string(scratch.project.join("unserved.unknownext"))
            .expect("write must persist without an LSP server"),
        "still persisted\n"
    );
    let write = tool_result_text(&doc, "write", 0);
    assert!(
        write.contains("no language server available") && !write.contains("Diagnostics:"),
        "missing LSP must be reported as a note, not fabricated diagnostics: {write}"
    );
    let lsp = tool_result_text(&doc, "lsp", 0);
    assert!(
        lsp.contains("no server available") && lsp.contains("no diagnostics"),
        "navigation must return a clean no-server result: {lsp}"
    );
}

#[test]
fn cortex_run_survives_killed_lsp_and_keeps_other_tools_usable() {
    if !has_command("rust-analyzer") {
        eprintln!("skipping real crash-resilience proof: rust-analyzer is not on PATH");
        return;
    }

    let scratch = Arc::new(Scratch::new("crash"));
    scratch.write_rust_project();
    let killed_pid = Arc::new(Mutex::new(None));
    let kill_scratch = scratch.clone();
    let kill_slot = killed_pid.clone();
    let provider = ScriptedProvider::start(
        vec![
            ProviderStep::Tool {
                call_id: "call_start_server",
                name: "lsp",
                arguments: json!({"action":"documentSymbols","filePath":"src/lib.rs"}),
            },
            lsp_step("hover"),
            ProviderStep::Tool {
                call_id: "call_write_after_crash",
                name: "write",
                arguments: json!({
                    "path":"after_crash.rs",
                    "content":"pub fn still_works() -> i32 {\n    7\n}\n"
                }),
            },
            ProviderStep::Final,
        ],
        move |index| {
            if index == 1 {
                let rows = lsp_status_json(&kill_scratch);
                let rust = rows
                    .iter()
                    .find(|row| row["language"] == "rust")
                    .expect("rust status row before deliberate crash");
                assert_eq!(rust["state"], "running", "server must be running: {rust}");
                let pid = rust["pid"].as_u64().expect("running server PID") as u32;
                let status = Command::new("kill")
                    .arg(pid.to_string())
                    .status()
                    .expect("signal exact Cortex-owned LSP PID");
                assert!(status.success(), "kill exact LSP PID must succeed");
                *kill_slot.lock().expect("killed PID slot") = Some(pid);
                thread::sleep(Duration::from_millis(150));
            }
        },
    );

    let output = run(cortex_run(
        &scratch,
        &provider,
        "continue after a language server crash",
    ));
    let doc = assert_success_json(&output);
    provider.finish();

    let unavailable = tool_result_text(&doc, "lsp", 1);
    assert!(
        unavailable.contains("language server unavailable")
            && unavailable.contains("stopped responding or crashed"),
        "post-crash LSP operation must be a clean unavailable result: {unavailable}"
    );
    assert_eq!(
        fs::read_to_string(scratch.project.join("after_crash.rs"))
            .expect("write must remain usable after LSP crash"),
        "pub fn still_works() -> i32 {\n    7\n}\n"
    );
    let write = tool_result_text(&doc, "write", 0);
    assert!(
        write.contains("LSP diagnostics unavailable") && !write.contains("Diagnostics:"),
        "post-crash write must succeed without fabricated diagnostics: {write}"
    );
    let pid = killed_pid
        .lock()
        .expect("killed PID slot")
        .expect("test must kill its exact LSP PID");
    assert_pid_gone(pid);
}

fn lsp_step(action: &'static str) -> ProviderStep {
    ProviderStep::Tool {
        call_id: match action {
            "hover" => "call_hover",
            "definition" => "call_definition",
            "references" => "call_references",
            _ => panic!("unsupported position LSP action for fixture: {action}"),
        },
        name: "lsp",
        arguments: json!({
            "action":action,
            "filePath":"src/lib.rs",
            "line":6,
            "character":7,
            "includeDeclaration":true
        }),
    }
}

fn cortex_run(scratch: &Scratch, provider: &ScriptedProvider, prompt: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(&scratch.project)
        .args(["run", "--format", "json", prompt])
        .env("HOME", &scratch.home)
        .env("CORTEX_DB_PATH", &scratch.db)
        .env("OPENROUTER_API_KEY", "fixture-key")
        .env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url)
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn lsp_status_json(scratch: &Scratch) -> Vec<Value> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(&scratch.project)
        .args(["lsp", "status", "--format", "json"])
        .env("HOME", &scratch.home)
        .env("CORTEX_DB_PATH", &scratch.db)
        .env("CORTEX_NO_NET", "1")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command);
    assert_eq!(
        output.status.code(),
        Some(0),
        "cortex lsp status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("LSP status JSON")
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn Cortex");
    let mut stdout = child.stdout.take().expect("Cortex stdout");
    let mut stderr = child.stderr.take().expect("Cortex stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).expect("read Cortex stdout");
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).expect("read Cortex stderr");
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for Cortex") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Cortex command exceeded {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout_thread.join().expect("join Cortex stdout"),
        stderr: stderr_thread.join().expect("join Cortex stderr"),
    }
}

fn assert_success_json(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "Cortex run failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Cortex run JSON")
}

fn tool_result_text(doc: &Value, name: &str, occurrence: usize) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results array")
        .iter()
        .filter(|result| result["name"] == name)
        .nth(occurrence)
        .unwrap_or_else(|| panic!("missing {name} tool result #{occurrence}: {doc}"))["parts"]
        .as_array()
        .expect("tool result parts")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn assert_pid_gone(pid: u32) {
    assert!(
        Command::new("ps")
            .args(["-p", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| !status.success()),
        "Cortex-owned LSP PID {pid} must be gone after run shutdown"
    );
}

fn bind_provider_listener() -> (u16, TcpListener) {
    for _ in 0..20 {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        let port = 3280 + (port - 3280) % 20;
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return (port, listener);
        }
    }
    panic!("no test provider port available in the permitted 3280-3299 range");
}

fn accept_with_idle_timeout(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("accept provider request: {err}"),
        }
    }
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set provider request read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) => return bytes,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|index| index + 4)
                {
                    break end;
                }
            }
            Err(_) => return bytes,
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or_default();
    while bytes.len().saturating_sub(header_end) < content_length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
        }
    }
    bytes
}

fn tool_call_body(call_id: &str, name: &str, arguments: Value) -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": {"name":name,"arguments":arguments.to_string()}
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}).to_string(),
    ])
}

fn final_body() -> String {
    sse_body(&[
        json!({"choices":[{"delta":{"content":"completed"},"finish_reason":null}]}).to_string(),
        json!({
            "choices":[{"delta":{},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}
        })
        .to_string(),
    ])
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
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    )?;
    stream.flush()
}
