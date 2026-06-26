//! Black-box regression for gap-engine-streaming-delta-lifecycle.
//!
//! The shipped `cortex run --jsonl` surface must expose part lifecycle events with stable ids and
//! timestamps, and persisted assistant parts must carry ids/timestamps consistent with those
//! streamed lifecycle events.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(30);

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
    fn start(mut responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let handle = thread::spawn(move || {
            let idle_deadline = Instant::now() + Duration::from_secs(10);
            while !responses.is_empty() && Instant::now() < idle_deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = read_http_request(&mut stream);
                        let response = responses.remove(0);
                        let _ = write_http_response(&mut stream, &response);
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

fn first_round_tool_call_frame() -> String {
    serde_json::json!({
        "id": "streaming-lifecycle-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_todo_lifecycle",
                    "type": "function",
                    "function": {
                        "name": "todo",
                        "arguments": "{\"todos\":\"1. [pending] lifecycle\"}"
                    }
                }]
            }
        }]
    })
    .to_string()
}

fn first_round_done_frame() -> String {
    serde_json::json!({
        "id": "streaming-lifecycle-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "tool_calls"
        }]
    })
    .to_string()
}

fn second_round_reasoning_frame() -> String {
    serde_json::json!({
        "id": "streaming-lifecycle-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": { "reasoning": "thinking about the todo result" },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn second_round_text_frame() -> String {
    serde_json::json!({
        "id": "streaming-lifecycle-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": { "content": "Lifecycle complete." },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn second_round_done_frame() -> String {
    serde_json::json!({
        "id": "streaming-lifecycle-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": 8,
            "total_tokens": 20
        }
    })
    .to_string()
}

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-streaming-lifecycle-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn cortex_jsonl_command(home: &Path, base_url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["run", "--jsonl", "exercise streaming lifecycle"])
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
    command
}

fn session_show_command(home: &Path, session_id: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["session", "show", session_id, "--format", "json"])
        .env("HOME", home)
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let started = Instant::now();
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
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn parse_jsonl(stdout: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8(stdout.to_vec())
        .expect("utf8 stdout")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("valid jsonl line"))
        .collect()
}

#[test]
fn jsonl_part_lifecycle_events_have_stable_ids_and_persisted_alignment() {
    let home = scratch_home();
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[first_round_tool_call_frame(), first_round_done_frame()]),
        ScriptedResponse::success(&[
            second_round_reasoning_frame(),
            second_round_text_frame(),
            second_round_done_frame(),
        ]),
    ]);

    let out = run(cortex_jsonl_command(&home, &fixture.base_url));
    fixture.finish();

    assert_eq!(
        out.status.code(),
        Some(0),
        "jsonl run must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = parse_jsonl(&out.stdout);
    assert!(
        !events.is_empty(),
        "jsonl output must include lifecycle and runner events"
    );
    assert_eq!(
        events.last().and_then(|event| event["type"].as_str()),
        Some("done"),
        "runner done remains the terminal event"
    );

    let session_id = events
        .iter()
        .find_map(|event| {
            (event["type"] == "session")
                .then(|| event["session"].as_str().map(ToOwned::to_owned))
                .flatten()
        })
        .expect("session id in leading jsonl event");

    let lifecycle_events: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["type"] == "part_lifecycle")
        .collect();
    assert!(
        !lifecycle_events.is_empty(),
        "jsonl must include part lifecycle events"
    );
    for kind in ["text", "reasoning", "tool_input", "tool_progress", "step"] {
        for phase in ["start", "delta", "end"] {
            assert!(
                lifecycle_events
                    .iter()
                    .any(|event| event["part_kind"] == kind && event["phase"] == phase),
                "missing {kind}/{phase} lifecycle event: {lifecycle_events:?}"
            );
        }
    }
    assert!(
        lifecycle_events.iter().all(|event| {
            event["part_id"].as_str().is_some_and(|id| !id.is_empty())
                && event["timestamp"].as_str().is_some_and(|ts| !ts.is_empty())
        }),
        "all lifecycle events must carry stable part ids and timestamps: {lifecycle_events:?}"
    );

    let show = run(session_show_command(&home, &session_id));
    assert_eq!(
        show.status.code(),
        Some(0),
        "session show must succeed: {}",
        String::from_utf8_lossy(&show.stderr)
    );
    let detail: serde_json::Value = serde_json::from_slice(&show.stdout).expect("session json");
    let messages = detail["messages"].as_array().expect("messages array");
    let assistant = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .expect("assistant message");
    let parts = assistant["parts"].as_array().expect("assistant parts");
    let persisted_ids: HashSet<String> = parts
        .iter()
        .filter_map(|part| part["id"].as_str().map(ToOwned::to_owned))
        .collect();
    let lifecycle_ids: HashSet<String> = lifecycle_events
        .iter()
        .filter_map(|event| event["part_id"].as_str().map(ToOwned::to_owned))
        .collect();
    for id in &lifecycle_ids {
        assert!(
            persisted_ids.contains(id),
            "persisted parts should include streamed lifecycle id {id}; persisted={persisted_ids:?}"
        );
    }

    let lifecycle_start_timestamps: HashMap<String, String> = lifecycle_events
        .iter()
        .filter(|event| event["phase"] == "start")
        .filter_map(|event| {
            Some((
                event["part_id"].as_str()?.to_owned(),
                event["timestamp"].as_str()?.to_owned(),
            ))
        })
        .collect();
    for part in parts {
        let Some(id) = part["id"].as_str() else {
            continue;
        };
        if let Some(expected) = lifecycle_start_timestamps.get(id) {
            let actual = part["created_at"]
                .as_str()
                .expect("persisted part timestamp");
            assert_eq!(
                actual, expected,
                "persisted timestamp should align with streamed lifecycle start timestamp for {id}"
            );
        }
    }
}
