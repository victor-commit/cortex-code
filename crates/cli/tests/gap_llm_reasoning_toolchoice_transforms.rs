use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    body: String,
}

#[derive(Clone, Debug)]
struct ScriptedResponse {
    status_code: u16,
    body: String,
}

struct Fixture {
    base_url: String,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set fixture listener nonblocking");
        let port = listener.local_addr().expect("fixture addr").port();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_thread = Arc::clone(&captured);
        let mut responses = responses;
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && !responses.is_empty() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_http_request(&mut stream);
                        captured_thread.lock().expect("captures lock").push(request);
                        let response = responses.remove(0);
                        let _ =
                            write_http_response(&mut stream, response.status_code, &response.body);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return,
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

struct Scratch {
    root: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-llm-reasoning-toolchoice-{tag}-{}-{}",
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

fn write_http_response(
    stream: &mut TcpStream,
    status_code: u16,
    body: &str,
) -> std::io::Result<()> {
    let status_text = match status_code {
        200 => "OK",
        400 => "Bad Request",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
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

fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be strict json")
}

fn synthetic_api_key(label: &str) -> String {
    format!("cfg-{}-provider-value", label.replace('_', "-"))
}

#[test]
fn reasoning_output_remains_separate_from_final_answer_content() {
    let scratch = Scratch::new("val-gap-llm-008");
    let api_key = synthetic_api_key("reasoning-separation");
    let fixture = Fixture::start(vec![ScriptedResponse {
        status_code: 200,
        body: sse_body(&[
            serde_json::json!({
                "id": "reasoning-separation",
                "object": "chat.completion.chunk",
                "model": "local/reasoning-model",
                "choices": [{"index":0,"delta":{"reasoning":"internal reasoning trail"},"finish_reason":serde_json::Value::Null}]
            })
            .to_string(),
            serde_json::json!({
                "id": "reasoning-separation",
                "object": "chat.completion.chunk",
                "model": "local/reasoning-model",
                "choices": [{"index":0,"delta":{"content":"VISIBLE-ANSWER"},"finish_reason":"stop"}],
                "usage": {"prompt_tokens": 6, "completion_tokens": 2, "total_tokens": 8}
            })
            .to_string(),
        ]),
    }]);
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "local/reasoning-model"

[provider.local]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "openai-chat"
models = [{{ id = "local/reasoning-model", name = "Reasoning Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#,
        base_url = fixture.base_url
    ));

    let out = run(scratch.command(&["run", "--format", "json", "separate reasoning from answer"]));
    let _captures = fixture.finish();
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = json_output(&out);
    assert_eq!(doc["content"], "VISIBLE-ANSWER");
    assert!(
        !doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("internal reasoning")),
        "final answer should exclude reasoning text: {doc}"
    );
    let reasoning_parts = doc["parts"]
        .as_array()
        .expect("parts array")
        .iter()
        .filter(|part| part["type"] == "reasoning")
        .collect::<Vec<_>>();
    assert!(
        reasoning_parts.iter().any(|part| {
            part["text"]
                .as_str()
                .is_some_and(|text| text.contains("internal reasoning trail"))
        }),
        "reasoning should be preserved as a separate structured part: {doc}"
    );
}

#[test]
fn provider_reasoning_signature_round_trips_without_leaking_blob_to_answer_output() {
    let scratch = Scratch::new("val-gap-llm-009");
    let api_key = synthetic_api_key("anthropic-signature");
    let signature = "sig_encrypted_123";
    let fixture = Fixture::start(vec![
        ScriptedResponse {
            status_code: 200,
            body: sse_body(&[
                serde_json::json!({
                    "type": "message_start",
                    "message": { "usage": { "input_tokens": 9 } }
                })
                .to_string(),
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "thinking_delta", "thinking": "private-thought" }
                })
                .to_string(),
                serde_json::json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "signature_delta", "signature": signature }
                })
                .to_string(),
                serde_json::json!({
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": { "type": "text", "text": "FIRST-TURN-OK" }
                })
                .to_string(),
                serde_json::json!({
                    "type": "message_delta",
                    "delta": { "stop_reason": "end_turn" },
                    "usage": { "output_tokens": 3 }
                })
                .to_string(),
            ]),
        },
        ScriptedResponse {
            status_code: 200,
            body: sse_body(&[
                serde_json::json!({
                    "type": "message_start",
                    "message": { "usage": { "input_tokens": 8 } }
                })
                .to_string(),
                serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": { "type": "text", "text": "SECOND-TURN-OK" }
                })
                .to_string(),
                serde_json::json!({
                    "type": "message_delta",
                    "delta": { "stop_reason": "end_turn" },
                    "usage": { "output_tokens": 2 }
                })
                .to_string(),
            ]),
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
        base_url = fixture.base_url
    ));

    let first = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--session",
        "reasoning-signature-session",
        "first turn",
    ]));
    let second = run(scratch.command(&[
        "run",
        "--format",
        "json",
        "--session",
        "reasoning-signature-session",
        "second turn",
    ]));
    let captures = fixture.finish();
    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    let first_doc = json_output(&first);
    let second_doc = json_output(&second);
    assert_eq!(first_doc["content"], "FIRST-TURN-OK");
    assert_eq!(second_doc["content"], "SECOND-TURN-OK");
    assert!(
        !first_doc["content"]
            .as_str()
            .is_some_and(|content| content.contains(signature))
            && !second_doc["content"]
                .as_str()
                .is_some_and(|content| content.contains(signature)),
        "encrypted signature blob must not leak into final assistant content"
    );
    assert!(
        captures.len() >= 2,
        "expected two anthropic requests for two session turns, got {}",
        captures.len()
    );
    let second_request: Value =
        serde_json::from_str(&captures[1].body).expect("second request json");
    let has_signed_thinking_block = second_request["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .any(|block| {
            block["type"] == "thinking"
                && block["signature"] == signature
                && block["thinking"]
                    .as_str()
                    .is_some_and(|text| text.contains("private-thought"))
        });
    assert!(
        has_signed_thinking_block,
        "second request should replay reasoning metadata as signed thinking block: {second_request}"
    );
}

#[test]
fn tool_choice_modes_are_honored_for_native_gemini_tool_calls() {
    let run_mode = |tag: &str, tool_choice_toml: &str, responses: Vec<ScriptedResponse>| {
        let scratch = Scratch::new(tag);
        let api_key = synthetic_api_key(tag);
        let fixture = Fixture::start(responses);
        scratch.write_project_config(&format!(
            r#"
[provider]
enabled = ["google"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "google/gemini-2.5-flash"

[provider.google]
base_url = "{base_url}/v1beta"
api_key = "{api_key}"
protocol = "gemini"

[provider.google.options.params]
tool_choice = {tool_choice_toml}
"#,
            base_url = fixture.base_url
        ));
        let out = run(scratch.command(&["run", "--format", "json", "exercise tool choice mode"]));
        let captures = fixture.finish();
        (out, captures)
    };

    let gemini_text_response = |text: &str| ScriptedResponse {
        status_code: 200,
        body: sse_body(&[serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": text }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 4,
                "candidatesTokenCount": 2,
                "totalTokenCount": 6
            }
        })
        .to_string()]),
    };
    let gemini_tool_call_response = ScriptedResponse {
        status_code: 200,
        body: sse_body(&[serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "todo",
                            "args": { "todos": "1. [completed] tool-choice-check" }
                        }
                    }]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 2,
                "totalTokenCount": 7
            }
        })
        .to_string()]),
    };

    // no tools
    let (none_out, none_captures) = run_mode(
        "toolchoice-none",
        "\"none\"",
        vec![gemini_text_response("NONE")],
    );
    assert_eq!(none_out.status.code(), Some(0));
    assert_eq!(json_output(&none_out)["content"], "NONE");
    let none_request: Value =
        serde_json::from_str(&none_captures[0].body).expect("none request json");
    assert_eq!(
        none_request["toolConfig"]["functionCallingConfig"]["mode"],
        "NONE"
    );
    assert_eq!(
        none_captures.len(),
        1,
        "none mode should not continue via tool call"
    );

    // auto mode
    let (auto_out, auto_captures) = run_mode(
        "toolchoice-auto",
        "\"auto\"",
        vec![gemini_text_response("AUTO")],
    );
    assert_eq!(auto_out.status.code(), Some(0));
    assert_eq!(json_output(&auto_out)["content"], "AUTO");
    let auto_request: Value =
        serde_json::from_str(&auto_captures[0].body).expect("auto request json");
    assert_eq!(
        auto_request["toolConfig"]["functionCallingConfig"]["mode"],
        "AUTO"
    );

    // required mode
    let (required_out, required_captures) = run_mode(
        "toolchoice-required",
        "\"required\"",
        vec![
            gemini_tool_call_response.clone(),
            gemini_text_response("REQUIRED"),
        ],
    );
    assert_eq!(required_out.status.code(), Some(0));
    assert_eq!(json_output(&required_out)["content"], "REQUIRED");
    let required_request: Value =
        serde_json::from_str(&required_captures[0].body).expect("required request json");
    assert_eq!(
        required_request["toolConfig"]["functionCallingConfig"]["mode"],
        "ANY"
    );
    assert!(
        required_captures.len() >= 2 && required_captures[1].body.contains("\"functionResponse\""),
        "required mode should produce a tool response continuation"
    );
    let required_follow_up: Value =
        serde_json::from_str(&required_captures[1].body).expect("required follow-up request json");
    assert!(
        required_follow_up["toolConfig"]["functionCallingConfig"].is_null(),
        "required mode should relax after one forced tool call: {required_follow_up}"
    );

    // specific tool mode
    let (specific_out, specific_captures) = run_mode(
        "toolchoice-specific",
        "{ type = \"tool\", name = \"todo\" }",
        vec![gemini_tool_call_response, gemini_text_response("SPECIFIC")],
    );
    assert_eq!(specific_out.status.code(), Some(0));
    assert_eq!(json_output(&specific_out)["content"], "SPECIFIC");
    let specific_request: Value =
        serde_json::from_str(&specific_captures[0].body).expect("specific request json");
    assert_eq!(
        specific_request["toolConfig"]["functionCallingConfig"]["mode"],
        "ANY"
    );
    assert_eq!(
        specific_request["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"],
        serde_json::json!(["todo"])
    );
    let specific_follow_up: Value =
        serde_json::from_str(&specific_captures[1].body).expect("specific follow-up request json");
    assert!(
        specific_follow_up["toolConfig"]["functionCallingConfig"].is_null(),
        "specific mode should relax after one forced tool call: {specific_follow_up}"
    );
}

#[test]
fn provider_transforms_scrub_remap_and_clamp_request_parameters() {
    let scratch = Scratch::new("val-gap-llm-014");
    let api_key = synthetic_api_key("provider-transforms");
    let fixture = Fixture::start(vec![ScriptedResponse {
        status_code: 200,
        body: sse_body(&[serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "TRANSFORM-OK" }] },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 4,
                "candidatesTokenCount": 2,
                "totalTokenCount": 6
            }
        })
        .to_string()]),
    }]);
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["google"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "google/gemini-2.5-flash"

[provider.google]
base_url = "{base_url}/v1beta"
api_key = "{api_key}"
protocol = "gemini"

[provider.google.options.params]
tool_choice = "auto"
top_p = 1.7
reasoning_effort = "high"
max_tokens = 77
unsupported_field = "must_be_scrubbed"
"#,
        base_url = fixture.base_url
    ));

    let out = run(scratch.command(&["run", "--format", "json", "apply provider transforms"]));
    let captures = fixture.finish();
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed after provider transforms: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(json_output(&out)["content"], "TRANSFORM-OK");

    let request: Value = serde_json::from_str(&captures[0].body).expect("gemini request json");
    assert!(
        request.get("unsupported_field").is_none(),
        "unsupported params must be scrubbed from outbound request: {request}"
    );
    assert_eq!(
        request["generationConfig"]["maxOutputTokens"],
        serde_json::json!(77)
    );
    assert_eq!(request["generationConfig"]["topP"], serde_json::json!(1.0));
    assert_eq!(
        request["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        serde_json::json!(8192)
    );
    assert_eq!(
        request["generationConfig"]["temperature"],
        serde_json::json!(1.0)
    );
    assert_eq!(
        request["toolConfig"]["functionCallingConfig"]["mode"],
        "AUTO"
    );
    assert!(
        captures[0]
            .request_line
            .contains(":streamGenerateContent?alt=sse"),
        "gemini fixture should receive native streamGenerateContent request"
    );
}
