//! Black-box proof that agent and mode directories compose in `cortex run`.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const SERVER_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-agent-mode-merge-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let home = root.join("home");
        let project = home.join("project");
        let data = root.join("data");
        fs::create_dir_all(project.join(".cortex/agents")).expect("agents dir");
        fs::create_dir_all(project.join(".cortex/modes")).expect("modes dir");
        fs::create_dir_all(&data).expect("data dir");
        Self {
            root,
            home,
            project,
            data,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ScriptedOpenAiServer {
    base_url: String,
    seen: Arc<AtomicUsize>,
    handle: thread::JoinHandle<()>,
}

impl ScriptedOpenAiServer {
    fn start_bash_turn(command: &str) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let seen = Arc::new(AtomicUsize::new(0));
        let seen_thread = seen.clone();
        let mut responses = VecDeque::from(vec![tool_call_body(command), final_text_body()]);
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + SERVER_IDLE_TIMEOUT;
            while !responses.is_empty() && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        seen_thread.fetch_add(1, Ordering::Relaxed);
                        let _ = read_http_request(&mut stream);
                        let body = responses.pop_front().expect("response");
                        let _ = write_http_sse(&mut stream, &body);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            seen,
            handle,
        }
    }

    fn finish(self) -> usize {
        let _ = self.handle.join();
        self.seen.load(Ordering::Relaxed)
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in 3200-3299");
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
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
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn write_http_sse(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
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

fn tool_call_body(command: &str) -> String {
    let arguments = serde_json::json!({ "command": command }).to_string();
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_bash",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        serde_json::json!({
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        })
        .to_string(),
    ])
}

fn final_text_body() -> String {
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": { "content": "done" },
                "finish_reason": null
            }]
        })
        .to_string(),
        serde_json::json!({
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    ])
}

fn cortex_command(sandbox: &Sandbox, server: &ScriptedOpenAiServer, args: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("CORTEX_DATA_DIR", &sandbox.data);
    command.env("OPENROUTER_API_KEY", "test-openrouter-key");
    command.env("CORTEX_OPENROUTER_BASE_URL", &server.base_url);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DB_PATH")
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

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
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
            panic!("cortex run timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn run_scripted_bash(sandbox: &Sandbox, mode: Option<&str>, command: &str) -> serde_json::Value {
    let server = ScriptedOpenAiServer::start_bash_turn(command);
    let mut args = vec![
        "run".to_owned(),
        "--agent".to_owned(),
        "writer".to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    if let Some(mode) = mode {
        args.push("--mode".to_owned());
        args.push(mode.to_owned());
    }
    args.push(format!(
        "Call bash exactly once with this command, then reply done: {command}"
    ));

    let out = run(cortex_command(sandbox, &server, &args));
    let seen = server.finish();
    assert_eq!(seen, 2, "run should include tool call and final response");
    assert!(
        out.status.success(),
        "run should succeed, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("strict json")
}

fn bash_result(doc: &serde_json::Value) -> &serde_json::Value {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .find(|result| result["name"] == "bash")
        .unwrap_or_else(|| panic!("missing bash result in {doc}"))
}

#[test]
fn mode_overlay_merges_with_custom_agent_and_is_reported_in_output() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.project.join(".cortex/agents/writer.md"),
        r#"---
name: writer
description: writes files
mode: primary
---
Always call bash exactly once when asked.
"#,
    )
    .expect("write custom agent");
    fs::write(
        sandbox.project.join(".cortex/modes/safe-plan.md"),
        r#"---
name: safe-plan
description: read-only planning mode
agent: plan
permission:
  - tool: bash
    pattern: "*"
    action: deny
---
Do not mutate files.
"#,
    )
    .expect("write mode");

    let marker = sandbox.project.join("mode-merge-marker.txt");
    let denied = run_scripted_bash(
        &sandbox,
        Some("safe-plan"),
        &format!("echo denied > {}", marker.display()),
    );
    assert_eq!(denied["agent"], "writer");
    assert_eq!(denied["mode"], "safe-plan");
    assert_eq!(
        bash_result(&denied)["is_error"],
        serde_json::Value::Bool(true),
        "mode deny must block bash"
    );
    assert!(
        !marker.exists(),
        "mode-level deny should prevent mutation in the requested command"
    );

    let allowed = run_scripted_bash(
        &sandbox,
        None,
        &format!("echo allowed > {}", marker.display()),
    );
    assert_eq!(allowed["agent"], "writer");
    assert!(
        allowed.get("mode").is_none() || allowed["mode"].is_null(),
        "mode is omitted when not selected: {allowed}"
    );
    assert_eq!(
        bash_result(&allowed)["is_error"],
        serde_json::Value::Bool(false),
        "without mode overlay the custom agent keeps its original permissions"
    );
    assert_eq!(
        fs::read_to_string(&marker)
            .expect("marker created by allowed run")
            .trim(),
        "allowed"
    );
}
