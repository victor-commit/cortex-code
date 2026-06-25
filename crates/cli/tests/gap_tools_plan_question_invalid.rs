use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Debug, Clone)]
struct ScriptedResponse {
    status_line: &'static str,
    headers: Vec<(String, String)>,
    body: String,
}

impl ScriptedResponse {
    fn success(frames: &[String]) -> Self {
        Self {
            status_line: "HTTP/1.1 200 OK",
            headers: vec![
                ("content-type".to_owned(), "text/event-stream".to_owned()),
                ("cache-control".to_owned(), "no-cache".to_owned()),
                ("connection".to_owned(), "close".to_owned()),
            ],
            body: sse_body(frames),
        }
    }
}

struct Fixture {
    base_url: String,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let responses = Arc::new(Mutex::new(responses));
        let handle = {
            let responses = Arc::clone(&responses);
            thread::spawn(move || {
                let idle_deadline = Instant::now() + Duration::from_secs(12);
                while Instant::now() < idle_deadline {
                    let remaining = responses.lock().expect("responses lock").len();
                    if remaining == 0 {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = read_http_request(&mut stream);
                            let response = responses.lock().expect("responses lock").remove(0);
                            let _ = write_http_response(&mut stream, &response);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    fn finish(self) {
        let _ = self.handle.join();
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in mission range 3200-3299");
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
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn write_http_response(stream: &mut TcpStream, response: &ScriptedResponse) -> std::io::Result<()> {
    let mut rendered = String::new();
    rendered.push_str(response.status_line);
    rendered.push_str("\r\n");
    for (name, value) in &response.headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    rendered.push_str(&format!("content-length: {}\r\n\r\n", response.body.len()));
    rendered.push_str(&response.body);
    stream.write_all(rendered.as_bytes())?;
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

fn text_delta_frame(text: &str) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn done_frame(reason: &str) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": reason
        }]
    })
    .to_string()
}

fn tool_call_frame(call_id: &str, name: &str, arguments: serde_json::Value) -> String {
    tool_call_raw_arguments_frame(call_id, name, &arguments.to_string())
}

fn tool_call_raw_arguments_frame(call_id: &str, name: &str, arguments: &str) -> String {
    serde_json::json!({
        "id": "fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments
                    }
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn scratch_home(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-gap-tools-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut stdout = Vec::new();
            child
                .stdout
                .take()
                .expect("stdout")
                .read_to_end(&mut stdout)
                .expect("stdout bytes");
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("stderr")
                .read_to_end(&mut stderr)
                .expect("stderr bytes");
            return Output {
                status,
                stdout,
                stderr,
            };
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn cortex_run_json(home: &Path, base_url: &str, prompt: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .arg("run")
        .arg("--format")
        .arg("json")
        .arg(prompt)
        .env("HOME", home)
        .env("OPENROUTER_API_KEY", "test-openrouter-key")
        .env("CORTEX_OPENROUTER_BASE_URL", base_url)
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    run(command)
}

fn assert_success_json(out: Output) -> serde_json::Value {
    assert_eq!(
        out.status.code(),
        Some(0),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("valid run JSON")
}

fn tool_result_texts(doc: &serde_json::Value, tool_name: &str) -> Vec<String> {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .filter(|result| result["name"] == tool_name)
        .map(|result| {
            result
                .get("parts")
                .and_then(serde_json::Value::as_array)
                .expect("parts")
                .iter()
                .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[test]
fn plan_and_plan_exit_gate_mutations_until_exit() {
    let home = scratch_home("plan-mode");
    let target = home.join("plan-mode.txt");
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_plan",
                "plan",
                serde_json::json!({"plan":"1. Inspect\n2. Implement"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_write_blocked",
                "write",
                serde_json::json!({"path":"plan-mode.txt","content":"BLOCKED"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_plan_exit",
                "plan_exit",
                serde_json::json!({"approved":true}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_write_allowed",
                "write",
                serde_json::json!({"path":"plan-mode.txt","content":"ALLOWED"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("planning complete"), done_frame("stop")]),
    ]);

    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "exercise plan mode transitions",
    ));
    fixture.finish();

    let plan_text = tool_result_texts(&doc, "plan").join("\n");
    assert!(
        plan_text.contains("planning mode enabled"),
        "plan tool should enable planning mode: {plan_text}"
    );
    let plan_exit_text = tool_result_texts(&doc, "plan_exit").join("\n");
    assert!(
        plan_exit_text.contains("planning mode disabled"),
        "plan_exit should disable planning mode: {plan_exit_text}"
    );
    let writes = doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .filter(|result| result["name"] == "write")
        .collect::<Vec<_>>();
    assert_eq!(
        writes.len(),
        2,
        "expected one blocked and one allowed write"
    );
    assert!(
        writes[0]["is_error"] == true
            && serde_json::to_string(writes[0])
                .unwrap()
                .contains("planning mode is active"),
        "write while planning should be blocked: {writes:?}"
    );
    assert!(
        writes[1]["is_error"] == false,
        "write after plan_exit should succeed: {writes:?}"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("target file"),
        "ALLOWED",
        "only the post-plan-exit write should mutate disk"
    );
}

#[test]
fn question_tool_fails_closed_in_headless_runs() {
    let home = scratch_home("question-headless");
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_question",
                "question",
                serde_json::json!({"questions":[{"prompt":"Pick A or B","options":["A","B"]}]}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("headless recovery"), done_frame("stop")]),
    ]);

    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "exercise question in headless mode",
    ));
    fixture.finish();

    let question = tool_result_texts(&doc, "question").join("\n");
    assert!(
        question.contains("non-interactive"),
        "headless question should fail closed with cannot-ask diagnostics: {question}"
    );
}

#[test]
fn malformed_or_unknown_calls_use_invalid_fallback_and_turn_continues() {
    let home = scratch_home("invalid-fallback");
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_unknown",
                "totally_unknown_tool",
                serde_json::json!({}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_raw_arguments_frame("call_bad_args", "read", "{not json"),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            text_delta_frame("Recovered after invalid tool calls"),
            done_frame("stop"),
        ]),
    ]);

    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "exercise invalid tool fallback",
    ));
    fixture.finish();

    let unknown = tool_result_texts(&doc, "totally_unknown_tool").join("\n");
    assert!(
        unknown.contains("invalid tool call") && unknown.contains("unknown tool"),
        "unknown tools should use invalid fallback text: {unknown}"
    );
    let bad_args = tool_result_texts(&doc, "read").join("\n");
    assert!(
        bad_args.contains("invalid tool call") && bad_args.contains("invalid arguments JSON"),
        "malformed arguments should use invalid fallback text: {bad_args}"
    );
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("Recovered")),
        "turn should continue after invalid calls: {doc:?}"
    );
}
