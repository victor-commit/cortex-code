//! Regression coverage for built-in read-only agent modes and saved permission decisions.
//!
//! The test drives real `cortex run` subprocesses against an OpenAI-compatible loopback fixture so
//! the saved-decision store, permission engine, tool dispatcher, and bash tool are exercised through
//! the CLI surface. The fixture only supplies deterministic streamed model events; all permission
//! resolution and tool side effects are the shipped code path.

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
    checkpoints: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-agent-mode-hard-deny-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let sandbox = Self {
            home: root.join("home"),
            project: root.join("project"),
            data: root.join("data"),
            checkpoints: root.join("checkpoints"),
            root,
        };
        for dir in [
            &sandbox.home,
            &sandbox.project,
            &sandbox.data,
            &sandbox.checkpoints,
        ] {
            fs::create_dir_all(dir).expect("create sandbox dir");
        }
        sandbox
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
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let seen = Arc::new(AtomicUsize::new(0));
        let seen_thread = seen.clone();
        let mut responses = VecDeque::from(vec![tool_call_body(command), final_text_body()]);
        let handle = thread::spawn(move || {
            let idle_deadline = Instant::now() + SERVER_IDLE_TIMEOUT;
            while !responses.is_empty() && Instant::now() < idle_deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        seen_thread.fetch_add(1, Ordering::Relaxed);
                        let _ = read_http_request(&mut stream);
                        let body = responses
                            .pop_front()
                            .expect("response available for accepted request");
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
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
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
                "delta": { "content": "DONE" },
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
    ])
}

fn cortex_command(sandbox: &Sandbox, server: &ScriptedOpenAiServer, args: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("CORTEX_DATA_DIR", &sandbox.data);
    command.env("CORTEX_CHECKPOINT_DIR", &sandbox.checkpoints);
    command.env("OPENROUTER_API_KEY", "test-openrouter-key");
    command.env("CORTEX_OPENROUTER_BASE_URL", &server.base_url);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_PERMISSION_DIR")
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
    let mut stdout = child.stdout.take().expect("stdout handle");
    let mut stderr = child.stderr.take().expect("stderr handle");
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
        stdout: stdout_thread.join().expect("join stdout reader"),
        stderr: stderr_thread.join().expect("join stderr reader"),
    }
}

fn run_scripted_bash(
    sandbox: &Sandbox,
    agent: &str,
    extra_args: &[&str],
    command: &str,
) -> serde_json::Value {
    let server = ScriptedOpenAiServer::start_bash_turn(command);
    let mut args = vec![
        "run".to_owned(),
        "--agent".to_owned(),
        agent.to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    args.push(format!(
        "Call the bash tool exactly once with this command, then answer DONE: {command}"
    ));

    let out = run(cortex_command(sandbox, &server, &args));
    let seen_requests = server.finish();
    assert_eq!(
        seen_requests, 2,
        "the run should complete one tool-call round trip and one final response"
    );
    assert!(
        out.status.success(),
        "cortex run should exit successfully (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be one strict JSON document: {err}; stdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn bash_result(doc: &serde_json::Value) -> &serde_json::Value {
    doc["tool_results"]
        .as_array()
        .expect("tool_results array")
        .iter()
        .find(|result| result["name"] == "bash")
        .unwrap_or_else(|| panic!("missing bash tool result in {doc}"))
}

fn bash_text(doc: &serde_json::Value) -> String {
    bash_result(doc)["parts"]
        .as_array()
        .expect("parts array")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_bash_allowed(doc: &serde_json::Value) {
    assert_eq!(
        bash_result(doc)["is_error"],
        serde_json::Value::Bool(false),
        "bash should be allowed: {doc}"
    );
}

fn assert_bash_denied(doc: &serde_json::Value) {
    let result = bash_result(doc);
    assert_eq!(
        result["is_error"],
        serde_json::Value::Bool(true),
        "bash should be denied: {doc}"
    );
    let text = bash_text(doc);
    assert!(
        text.contains("permission denied"),
        "denied bash result should explain the permission block, got: {text}"
    );
}

#[test]
fn saved_bash_allow_cannot_bypass_plan_or_explore_but_build_still_honors_it() {
    let sandbox = Sandbox::new("bash");

    let seed = run_scripted_bash(
        &sandbox,
        "build",
        &["--permission-allow-save", "bash:echo *"],
        "echo seeded > seed.txt",
    );
    assert_bash_allowed(&seed);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("seed.txt"))
            .expect("seed file created")
            .trim(),
        "seeded"
    );

    let plan = run_scripted_bash(&sandbox, "plan", &[], "echo plan-blocked > plan_marker.txt");
    assert_bash_denied(&plan);
    assert!(
        !sandbox.project.join("plan_marker.txt").exists(),
        "plan mode hard deny must block the saved-allowed bash mutation"
    );

    let explore = run_scripted_bash(
        &sandbox,
        "explore",
        &[],
        "echo explore-blocked > explore_marker.txt",
    );
    assert_bash_denied(&explore);
    assert!(
        !sandbox.project.join("explore_marker.txt").exists(),
        "explore mode hard deny must block the saved-allowed bash mutation"
    );

    let build = run_scripted_bash(
        &sandbox,
        "build",
        &[],
        "echo build-still-allowed > build_marker.txt",
    );
    assert_bash_allowed(&build);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("build_marker.txt"))
            .expect("build marker created through saved allow")
            .trim(),
        "build-still-allowed",
        "build remains unrestricted and continues to honor the saved allow"
    );
}
