use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct ScriptedResponse {
    body: String,
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    body: String,
}

struct Fixture {
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start(mut responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("fixture addr").port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_thread = Arc::clone(&captured);
        let handle = thread::spawn(move || {
            let idle_deadline = Instant::now() + Duration::from_secs(10);
            while !responses.is_empty() && Instant::now() < idle_deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_http_request(&mut stream);
                        captured_thread.lock().expect("captures lock").push(request);
                        let response = responses.remove(0);
                        let _ = write_sse_response(&mut stream, &response.body);
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
            captured,
            handle,
        }
    }

    fn finish(self) -> Vec<CapturedRequest> {
        let _ = self.handle.join();
        self.captured.lock().expect("captures lock").clone()
    }
}

fn bind_loopback_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
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

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return CapturedRequest {
                body: String::new(),
            };
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
    CapturedRequest {
        body: String::from_utf8_lossy(&buffer[header_end..]).to_string(),
    }
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

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-gap-prompt-cache-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let root = std::env::temp_dir().join(unique);
    let project = root.join("home/project");
    fs::create_dir_all(&project).expect("create scratch project");
    root
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

fn cortex_command(root: &std::path::Path) -> Command {
    let home = root.join("home");
    let project = home.join("project");
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .env_clear()
        .current_dir(project)
        .args([
            "run",
            "--format",
            "json",
            "Use todo once, then reply with exactly CACHED-OK.",
        ])
        .env("HOME", home)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

#[test]
fn anthropic_runtime_emits_prompt_cache_hints_and_reports_cache_usage_tokens() {
    let first_turn = sse_body(&[
        serde_json::json!({
            "type": "message_start",
            "message": {
                "usage": { "input_tokens": 5 }
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "call_1",
                "name": "todo",
                "input": {
                    "todos": [
                        { "content": "cache probe", "status": "completed" }
                    ]
                }
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "message_delta",
            "delta": { "stop_reason": "tool_use" },
            "usage": { "output_tokens": 1 }
        })
        .to_string(),
    ]);
    let second_turn = sse_body(&[
        serde_json::json!({
            "type": "message_start",
            "message": {
                "usage": {
                    "input_tokens": 6,
                    "cache_creation_input_tokens": 9
                }
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "text",
                "text": "CACHED-OK"
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn" },
            "usage": {
                "output_tokens": 2,
                "cache_read_input_tokens": 4
            }
        })
        .to_string(),
    ]);
    let fixture = Fixture::start(vec![
        ScriptedResponse { body: first_turn },
        ScriptedResponse { body: second_turn },
    ]);
    let root = scratch_home();
    let project = root.join("home/project");
    fs::write(
        project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["anthropic"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "anthropic/cache-fixture"

[provider.anthropic]
base_url = "{base_url}"
api_key = "fixture-anthropic-key"
protocol = "anthropic-messages"
models = [
  {{ id = "anthropic/cache-fixture", name = "Cache Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}
]

[provider.anthropic.options.params]
prompt_cache_ttl_seconds = 3600
"#,
            base_url = fixture.base_url
        ),
    )
    .expect("write config");

    let out = run(cortex_command(&root));
    let captured = fixture.finish();
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output_doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout should be strict json");
    assert_eq!(
        output_doc["usage"]["cache_creation_input_tokens"].as_u64(),
        Some(9)
    );
    assert_eq!(
        output_doc["usage"]["cache_read_input_tokens"].as_u64(),
        Some(4)
    );
    assert_eq!(output_doc["content"].as_str(), Some("CACHED-OK"));

    assert!(captured.len() >= 2, "expected two provider requests");
    let second_request: serde_json::Value =
        serde_json::from_str(&captured[1].body).expect("second request json");
    assert_eq!(
        second_request["system"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(second_request["system"][0]["cache_control"]["ttl"], "1h");

    let messages = second_request["messages"]
        .as_array()
        .expect("messages array");
    let has_cached_tool_result = messages.iter().any(|message| {
        message["content"].as_array().is_some_and(|content| {
            content.iter().any(|block| {
                block["type"] == "tool_result" && block["cache_control"]["type"] == "ephemeral"
            })
        })
    });
    assert!(
        has_cached_tool_result,
        "last tool result block should carry cache_control: {second_request}"
    );

    let last_user_text = messages
        .iter()
        .filter(|message| message["role"] == "user")
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .rfind(|block| block["type"] == "text")
        .expect("at least one user text block");
    assert_eq!(last_user_text["cache_control"]["type"], "ephemeral");
    assert!(
        !captured[1].body.contains("prompt_cache_ttl_seconds"),
        "request-specific cache params must be consumed locally, not forwarded"
    );
}
