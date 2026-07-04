use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

struct Scratch {
    root: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-llm-token-cost-overflow-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("home/project");
        fs::create_dir_all(&project).expect("create scratch project");
        Self { root, project }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn write_project_config(&self, body: &str) {
        fs::write(self.project.join("cortex.toml"), body).expect("write project config");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.current_dir(&self.project);
        command.env("HOME", self.home());
        command.args(args);
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = thread::spawn(move || {
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
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn bind_loopback_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return String::new();
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
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
        let read = stream.read(&mut chunk).expect("read request body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    String::from_utf8_lossy(&buffer[header_end..]).to_string()
}

enum ScriptedResponse {
    Sse { frames: Vec<String> },
    Error { status: u16, body: String },
}

fn start_fixture(
    responses: Vec<ScriptedResponse>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = bind_loopback_listener();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let port = listener.local_addr().expect("fixture addr").port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_thread = Arc::clone(&requests);
    let mut responses = responses;
    let handle = thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) && !responses.is_empty() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let body = read_http_request_body(&mut stream);
                    requests_thread.lock().expect("request lock").push(body);
                    let response = responses.remove(0);
                    let _ = match response {
                        ScriptedResponse::Sse { frames } => {
                            write_sse_response(&mut stream, &frames)
                        }
                        ScriptedResponse::Error { status, body } => {
                            write_json_error_response(&mut stream, status, &body)
                        }
                    };
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (format!("http://127.0.0.1:{port}"), requests, handle)
}

fn write_sse_response(stream: &mut TcpStream, frames: &[String]) -> std::io::Result<()> {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn write_json_error_response(
    stream: &mut TcpStream,
    status: u16,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

#[test]
fn val_gap_llm_012_tiered_cache_and_reasoning_pricing_is_estimated() {
    let frames = vec![
        serde_json::json!({
            "id": "gap-012",
            "object": "chat.completion.chunk",
            "model": "local/tiered",
            "choices": [{
                "index": 0,
                "delta": { "content": "TIERED-COST-OK" },
                "finish_reason": serde_json::Value::Null
            }]
        })
        .to_string(),
        serde_json::json!({
            "id": "gap-012",
            "object": "chat.completion.chunk",
            "model": "local/tiered",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 250000,
                "completion_tokens": 100,
                "total_tokens": 250100,
                "cache_creation_input_tokens": 50,
                "prompt_tokens_details": { "cached_tokens": 100 },
                "completion_tokens_details": { "reasoning_tokens": 20 }
            }
        })
        .to_string(),
    ];
    let (base_url, requests, handle) = start_fixture(vec![ScriptedResponse::Sse { frames }]);

    let scratch = Scratch::new("val-gap-llm-012");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.local]
base_url = "{base_url}"
api_key = "local-fixture-key"
models = [{{ id = "local/tiered", name = "Tiered Local", tool_call = true, reasoning = true, context_length = 400000, max_output_tokens = 8192, cost = {{ input = 1, output = 2, cache_write = 0.5, cache_read = 0.25, reasoning = 3, over_input_threshold = {{ threshold_input_tokens = 200000, input = 4, output = 8, cache_write = 2, cache_read = 1, reasoning = 9 }} }} }}]
"#,
    ));

    let output = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--model",
        "local/tiered",
        "Reply with exactly TIERED-COST-OK.",
    ]));
    let _ = handle.join();
    assert_eq!(
        output.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.lock().expect("request lock").len(),
        1,
        "turn should complete in one request"
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("run json");
    assert_eq!(doc["content"], "TIERED-COST-OK");
    assert_eq!(doc["usage"]["cache_creation_input_tokens"], 50);
    assert_eq!(doc["usage"]["cache_read_input_tokens"], 100);
    assert_eq!(doc["usage"]["reasoning_output_tokens"], 20);
    let cost = doc["usage"]["cost"].as_f64().expect("numeric cost");
    let expected = ((249_850.0 * 4.0) + (50.0 * 2.0) + (100.0 * 1.0) + (80.0 * 8.0) + (20.0 * 9.0))
        / 1_000_000.0;
    assert!(
        (cost - expected).abs() < 1e-9,
        "expected estimated tiered cost {expected}, got {cost}"
    );
}

#[test]
fn val_gap_llm_012_known_zero_pricing_reports_numeric_zero_cost() {
    let frames = vec![
        serde_json::json!({
            "id": "gap-012-zero",
            "object": "chat.completion.chunk",
            "model": "local/free",
            "choices": [{
                "index": 0,
                "delta": { "content": "FREE-COST-OK" },
                "finish_reason": serde_json::Value::Null
            }]
        })
        .to_string(),
        serde_json::json!({
            "id": "gap-012-zero",
            "object": "chat.completion.chunk",
            "model": "local/free",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 30,
                "total_tokens": 150
            }
        })
        .to_string(),
    ];
    let (base_url, requests, handle) = start_fixture(vec![ScriptedResponse::Sse { frames }]);

    let scratch = Scratch::new("val-gap-llm-012-zero");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.local]
base_url = "{base_url}"
api_key = "local-fixture-key"
models = [{{ id = "local/free", name = "Free Local", tool_call = true, reasoning = false, context_length = 400000, max_output_tokens = 8192, cost = {{ input = 0, output = 0 }} }}]
"#,
    ));

    let output = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--model",
        "local/free",
        "Reply with exactly FREE-COST-OK.",
    ]));
    let _ = handle.join();
    assert_eq!(
        output.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.lock().expect("request lock").len(),
        1,
        "turn should complete in one request"
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("run json");
    assert_eq!(doc["content"], "FREE-COST-OK");
    assert!(
        doc["usage"]["cost"].is_number(),
        "known zero pricing must produce a numeric cost: {}",
        doc["usage"]
    );
    let cost = doc["usage"]["cost"].as_f64().expect("numeric zero cost");
    assert_eq!(cost, 0.0, "known zero-priced models must report explicit 0");
}

#[test]
fn val_gap_llm_012_unknown_pricing_remains_null() {
    let frames = vec![
        serde_json::json!({
            "id": "gap-012-unknown",
            "object": "chat.completion.chunk",
            "model": "local/unknown",
            "choices": [{
                "index": 0,
                "delta": { "content": "UNKNOWN-COST-OK" },
                "finish_reason": serde_json::Value::Null
            }]
        })
        .to_string(),
        serde_json::json!({
            "id": "gap-012-unknown",
            "object": "chat.completion.chunk",
            "model": "local/unknown",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 80,
                "completion_tokens": 20,
                "total_tokens": 100
            }
        })
        .to_string(),
    ];
    let (base_url, requests, handle) = start_fixture(vec![ScriptedResponse::Sse { frames }]);

    let scratch = Scratch::new("val-gap-llm-012-unknown");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.local]
base_url = "{base_url}"
api_key = "local-fixture-key"
models = [{{ id = "local/unknown", name = "Unknown Local", tool_call = true, reasoning = false, context_length = 400000, max_output_tokens = 8192 }}]
"#,
    ));

    let output = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--model",
        "local/unknown",
        "Reply with exactly UNKNOWN-COST-OK.",
    ]));
    let _ = handle.join();
    assert_eq!(
        output.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.lock().expect("request lock").len(),
        1,
        "turn should complete in one request"
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("run json");
    assert_eq!(doc["content"], "UNKNOWN-COST-OK");
    assert!(
        doc["usage"]["cost"].is_null(),
        "unknown pricing must stay null: {}",
        doc["usage"]
    );
}

#[test]
fn val_gap_llm_013_context_overflow_surfaces_specific_diagnostic() {
    let overflow_error = serde_json::json!({
        "error": {
            "message": "This model's maximum context length is 8192 tokens, however you requested 12000 tokens.",
            "code": "context_length_exceeded"
        }
    })
    .to_string();
    let responses = (0..6)
        .map(|_| ScriptedResponse::Error {
            status: 400,
            body: overflow_error.clone(),
        })
        .collect::<Vec<_>>();
    let (base_url, _requests, handle) = start_fixture(responses);

    let scratch = Scratch::new("val-gap-llm-013");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.local]
base_url = "{base_url}"
api_key = "local-fixture-key"
models = [{{ id = "local/overflow", name = "Overflow Local", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#,
    ));

    let big_prompt = "token ".repeat(20_000);
    let output = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--model",
        "local/overflow",
        &big_prompt,
    ]));
    let _ = handle.join();
    assert_ne!(
        output.status.code(),
        Some(0),
        "overflow run should fail with a typed error"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("context overflow"),
        "stderr must surface a context-overflow diagnostic, got: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "failed json run should keep stdout empty, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
