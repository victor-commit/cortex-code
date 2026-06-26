use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    body: String,
}

#[derive(Clone, Debug)]
struct ScriptedResponse {
    body: String,
}

struct Scratch {
    root: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-native-providers-{tag}-{}-{}",
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
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
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
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
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

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return CapturedRequest {
                request_line: String::new(),
                body: String::new(),
            };
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let request_line = headers.lines().next().unwrap_or_default().to_owned();
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
        request_line,
        body: String::from_utf8_lossy(&buffer[header_end..]).to_string(),
    }
}

fn start_fixture(
    responses: Vec<ScriptedResponse>,
) -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    std::thread::JoinHandle<()>,
) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let captures = Arc::new(Mutex::new(Vec::new()));
    let captures_thread = Arc::clone(&captures);
    let mut responses = responses;

    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !responses.is_empty() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    captures_thread.lock().expect("captures lock").push(request);
                    let response = responses.remove(0);
                    let response_text = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                        response.body.len(),
                        response.body
                    );
                    let _ = std::io::Write::write_all(&mut stream, response_text.as_bytes());
                    let _ = std::io::Write::flush(&mut stream);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    (format!("http://127.0.0.1:{port}"), captures, handle)
}

fn synthetic_config_api_key(label: &str) -> String {
    format!("cfg-{}-provider-value", label.replace('_', "-"))
}

#[test]
fn openai_responses_protocol_runs_through_runtime_provider_resolver() {
    let scratch = Scratch::new("openai-responses");
    let api_key = synthetic_config_api_key("openai-responses");
    let (base_url, captures, handle) = start_fixture(vec![ScriptedResponse {
        body: concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"openai responses ok\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":2,\"total_tokens\":6}}}\n\n",
            "data: [DONE]\n\n"
        )
        .to_owned(),
    }]);
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["openai"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "openai/gpt-4.1-mini"

[provider.openai]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "openai-responses"
"#,
    ));

    let out = run(scratch.command(&["run", "--format", "json", "say hi"]));
    handle.join().expect("join fixture");
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert_eq!(doc["model"], "openai/gpt-4.1-mini");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("openai responses ok")),
        "expected fixture text in output, got: {doc}"
    );
    let captures = captures.lock().expect("captures lock");
    let first = captures.first().expect("captured request");
    assert!(
        first.request_line.contains("POST /responses"),
        "request should target responses endpoint: {}",
        first.request_line
    );
    assert!(
        first.body.contains("\"model\":\"gpt-4.1-mini\""),
        "request should strip openai/ prefix for direct OpenAI model ids: {}",
        first.body
    );
}

#[test]
fn anthropic_messages_protocol_round_trips_tool_use_and_tool_result() {
    let scratch = Scratch::new("anthropic-messages");
    let api_key = synthetic_config_api_key("anthropic-messages");
    let (base_url, captures, handle) = start_fixture(vec![
        ScriptedResponse {
            body: concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"read\",\"input\":{}}}\n\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
        },
        ScriptedResponse {
            body: concat!(
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":6}}}\n\n",
                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"anthropic ok\"}}\n\n",
                "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
        },
    ]);
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["anthropic"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "anthropic/claude-sonnet-4"

[provider.anthropic]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "anthropic-messages"
"#,
    ));

    let out = run(scratch.command(&["run", "--format", "json", "use a tool then finish"]));
    handle.join().expect("join fixture");
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("anthropic ok")),
        "expected fixture text in output, got: {doc}"
    );
    let captures = captures.lock().expect("captures lock");
    assert!(
        captures.len() >= 2,
        "anthropic tool-use flow should issue two requests, got {}",
        captures.len()
    );
    assert!(
        captures[0].request_line.contains("POST /messages"),
        "request should target anthropic messages endpoint: {}",
        captures[0].request_line
    );
    assert!(
        captures[1].body.contains("\"type\":\"tool_result\"")
            && captures[1].body.contains("\"tool_use_id\":\"call_1\""),
        "second request should carry tool_result for call_1: {}",
        captures[1].body
    );
}

#[test]
fn gemini_protocol_round_trips_function_call_and_thought_parts() {
    let scratch = Scratch::new("gemini-native");
    let api_key = synthetic_config_api_key("gemini-native");
    let (base_url, captures, handle) = start_fixture(vec![
        ScriptedResponse {
            body: concat!(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"thinking\",\"thought\":true},{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"Cargo.toml\"}}}]}}],\"usageMetadata\":{\"promptTokenCount\":4,\"candidatesTokenCount\":2,\"totalTokenCount\":6}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
        },
        ScriptedResponse {
            body: concat!(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"gemini ok\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":2,\"totalTokenCount\":7}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_owned(),
        },
    ]);
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["google"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "google/gemini-2.5-flash"

[provider.google]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "gemini"
"#,
    ));

    let out = run(scratch.command(&["run", "--format", "json", "use a function call then finish"]));
    handle.join().expect("join fixture");
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("gemini ok")),
        "expected fixture text in output, got: {doc}"
    );
    let captures = captures.lock().expect("captures lock");
    assert!(
        captures.len() >= 2,
        "gemini tool-call flow should issue two requests, got {}",
        captures.len()
    );
    assert!(
        captures[0]
            .request_line
            .contains("/models/gemini-2.5-flash:streamGenerateContent?alt=sse"),
        "request should target gemini stream endpoint: {}",
        captures[0].request_line
    );
    assert!(
        captures[1].body.contains("\"functionResponse\""),
        "second request should carry a functionResponse tool result: {}",
        captures[1].body
    );
}
