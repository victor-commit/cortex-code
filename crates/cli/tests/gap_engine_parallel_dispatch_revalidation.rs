//! End-to-end re-proof for the engine's concurrent/parallel tool-dispatch behavior, driven through
//! real `cortex run` subprocesses (NOT in-process construction).
//!
//! - VAL-GAP-ENGINE-001: two independent slow tools requested in one model step execute
//!   concurrently. The deterministic local-provider proof emits two `bash` sleep tools in one step
//!   and asserts (a) both tool calls are admitted before either finishes, (b) the tools' own
//!   high-resolution clocks overlap, and (c) the combined execution span is close to the slowest
//!   call, not the sum. A second, runtime-gated proof exercises the same behavior live via real
//!   OpenRouter.
//! - VAL-GAP-ENGINE-002: out-of-order tool completions are reinserted into the transcript in a
//!   stable order. A local OpenAI-compatible loopback provider fixture (selected via
//!   `CORTEX_OPENROUTER_BASE_URL`, no external network) plus a slow and a fast tool sees paired
//!   tool results with correct call ids and deterministic original-call ordering across repeated
//!   runs.
//!
//! Process hygiene: the loopback fixture binds an OS-assigned ephemeral port (127.0.0.1:0); the
//! `cortex` child is reaped by its exact PID (killed on timeout). No external network is used by the
//! deterministic tests.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(45);

/// One scripted HTTP/SSE response the loopback provider returns for the next request it receives.
#[derive(Debug, Clone)]
struct ScriptedResponse {
    body: String,
}

impl ScriptedResponse {
    fn sse(frames: &[String]) -> Self {
        Self {
            body: sse_body(frames),
        }
    }
}

/// A minimal OpenAI-compatible loopback provider. It serves a fixed list of scripted SSE responses
/// in order and records every request body it receives so tests can inspect the reinjected
/// transcript the runner sends on the post-tool continuation round.
struct Fixture {
    base_url: String,
    handle: Option<thread::JoinHandle<()>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Fixture {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_thread = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            let mut remaining = responses;
            let deadline = Instant::now() + Duration::from_secs(40);
            while !remaining.is_empty() && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let body = read_http_request(&mut stream).unwrap_or_default();
                        requests_for_thread.lock().unwrap().push(body);
                        let response = remaining.remove(0);
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
            handle: Some(handle),
            requests,
        }
    }

    fn request_bodies(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    fn finish(mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn bind_loopback_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
}

/// Read one HTTP request, returning the request body (the chat-completions JSON payload).
fn read_http_request(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(String::new());
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
    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn write_http_response(stream: &mut TcpStream, response: &ScriptedResponse) -> std::io::Result<()> {
    let mut rendered = String::new();
    rendered.push_str("HTTP/1.1 200 OK\r\n");
    rendered.push_str("content-type: text/event-stream\r\n");
    rendered.push_str("cache-control: no-cache\r\n");
    rendered.push_str("connection: close\r\n");
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

/// One streamed tool-call delta at `index`, advertising a complete function name + arguments.
fn tool_call_frame(
    index: usize,
    call_id: &str,
    name: &str,
    arguments: &serde_json::Value,
) -> String {
    serde_json::json!({
        "id": "parallel-dispatch-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": index,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string(),
                    }
                }]
            }
        }]
    })
    .to_string()
}

fn tool_calls_done_frame() -> String {
    serde_json::json!({
        "id": "parallel-dispatch-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }]
    })
    .to_string()
}

fn text_frame(text: &str) -> String {
    serde_json::json!({
        "id": "parallel-dispatch-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{ "index": 0, "delta": { "content": text }, "finish_reason": null }]
    })
    .to_string()
}

fn final_done_frame() -> String {
    serde_json::json!({
        "id": "parallel-dispatch-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
        "usage": { "prompt_tokens": 11, "completion_tokens": 5, "total_tokens": 16 }
    })
    .to_string()
}

fn bash_args(command: &str) -> serde_json::Value {
    serde_json::json!({ "command": command })
}

fn scratch_home(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-parallel-dispatch-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn cortex_jsonl_command(home: &Path, base_url: Option<&str>, prompt: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["run", "--jsonl", prompt])
        .env("HOME", home);
    match base_url {
        Some(base_url) => {
            command
                .env("OPENROUTER_API_KEY", "test-openrouter-key")
                .env("CORTEX_OPENROUTER_BASE_URL", base_url);
        }
        None => {
            command.env_remove("CORTEX_OPENROUTER_BASE_URL");
        }
    }
    command
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("stdout join"),
        stderr: err_thread.join().expect("stderr join"),
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

/// Concatenated text payload of a `tool_result` event's text parts.
fn tool_result_text(event: &serde_json::Value) -> String {
    event["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Extract the floating-point epoch seconds following `marker ` in `text`.
fn marker_time(text: &str, marker: &str) -> Option<f64> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(marker)?;
        rest.split_whitespace().next()?.parse::<f64>().ok()
    })
}

// VAL-GAP-ENGINE-001 (deterministic local-provider proof): two independent slow `bash` tools
// requested in one model step run concurrently. Proven through a real `cortex run --jsonl`
// subprocess: both tool calls are admitted before either finishes, the tools' own clocks overlap,
// and the combined span is close to the slowest call (~2s), not the sum (~4s).
#[test]
fn two_independent_slow_tools_in_one_step_run_concurrently() {
    let home = scratch_home("concurrent");
    let alpha = "echo ALPHA_START $(date +%s.%N); sleep 2; echo ALPHA_END $(date +%s.%N)";
    let beta = "echo BETA_START $(date +%s.%N); sleep 2; echo BETA_END $(date +%s.%N)";

    let fixture = Fixture::start(vec![
        ScriptedResponse::sse(&[
            tool_call_frame(0, "call_alpha", "bash", &bash_args(alpha)),
            tool_call_frame(1, "call_beta", "bash", &bash_args(beta)),
            tool_calls_done_frame(),
        ]),
        ScriptedResponse::sse(&[
            text_frame("Both slow tools finished; results were collected."),
            final_done_frame(),
        ]),
    ]);

    let output = run(cortex_jsonl_command(
        &home,
        Some(&fixture.base_url),
        "run two slow tools at once",
    ));
    let request_bodies = fixture.request_bodies();
    fixture.finish();

    assert_eq!(
        output.status.code(),
        Some(0),
        "run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let events = parse_jsonl(&output.stdout);
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();

    // Admission-before-finish: both tool calls are emitted before either tool result.
    let last_tool_call = kinds
        .iter()
        .rposition(|kind| *kind == "tool_call")
        .expect("two tool_call events present");
    let first_tool_result = kinds
        .iter()
        .position(|kind| *kind == "tool_result")
        .expect("tool_result events present");
    let tool_call_count = kinds.iter().filter(|kind| **kind == "tool_call").count();
    let tool_result_count = kinds.iter().filter(|kind| **kind == "tool_result").count();
    assert_eq!(
        tool_call_count, 2,
        "exactly two tool calls admitted: {kinds:?}"
    );
    assert_eq!(tool_result_count, 2, "exactly two tool results: {kinds:?}");
    assert!(
        last_tool_call < first_tool_result,
        "both tool calls must be admitted before either finishes: {kinds:?}"
    );

    // Collect each tool's own high-resolution start/end timestamps from its captured stdout.
    let mut alpha_start = None;
    let mut alpha_end = None;
    let mut beta_start = None;
    let mut beta_end = None;
    for event in events.iter().filter(|event| event["type"] == "tool_result") {
        let text = tool_result_text(event);
        if let Some(t) = marker_time(&text, "ALPHA_START") {
            alpha_start = Some(t);
        }
        if let Some(t) = marker_time(&text, "ALPHA_END") {
            alpha_end = Some(t);
        }
        if let Some(t) = marker_time(&text, "BETA_START") {
            beta_start = Some(t);
        }
        if let Some(t) = marker_time(&text, "BETA_END") {
            beta_end = Some(t);
        }
    }
    let alpha_start = alpha_start.expect("alpha start timestamp in tool result");
    let alpha_end = alpha_end.expect("alpha end timestamp in tool result");
    let beta_start = beta_start.expect("beta start timestamp in tool result");
    let beta_end = beta_end.expect("beta end timestamp in tool result");

    // Concurrency: the two execution intervals overlap (each starts before the other ends). A
    // sequential dispatcher would start the second tool only after the first one finished.
    assert!(
        alpha_start < beta_end && beta_start < alpha_end,
        "tool execution intervals must overlap (concurrent): \
         alpha=[{alpha_start}, {alpha_end}] beta=[{beta_start}, {beta_end}]"
    );

    // Wall time close to the slowest call (~2s), not the sum (~4s), measured on the tools' own
    // clocks so the bound is immune to process-startup noise.
    let span = alpha_end.max(beta_end) - alpha_start.min(beta_start);
    assert!(
        span < 3.5,
        "combined execution span must be close to the slowest 2s call, not the 4s sum: {span}s"
    );

    // Final answer is present and the post-tool continuation request carried BOTH tool results,
    // so the model's final answer is synthesized from both.
    let answer: String = events
        .iter()
        .filter(|event| event["type"] == "text_delta")
        .filter_map(|event| event["text"].as_str())
        .collect();
    assert!(
        answer.contains("Both slow tools finished"),
        "final assistant answer should be present: {answer:?}"
    );

    let continuation = request_bodies
        .iter()
        .find(|body| body.contains("\"role\":\"tool\"") || body.contains("\"role\": \"tool\""))
        .expect("a continuation request carrying tool results");
    assert!(
        continuation.contains("ALPHA_END") && continuation.contains("BETA_END"),
        "continuation request must carry both tool results to the model"
    );
}

// VAL-GAP-ENGINE-002: out-of-order tool completions are reinserted into the transcript in a stable
// (original tool-call) order. A local OpenAI-compatible loopback fixture plus a slow and a fast
// `bash` tool sees paired tool results with correct call ids and deterministic ordering across
// repeated runs (no external network).
#[test]
fn out_of_order_completions_reinsert_in_stable_order_deterministically() {
    let mut reinjected_orders = Vec::new();

    for attempt in 0..3 {
        let home = scratch_home(&format!("stable-{attempt}"));
        // `call_slow` is requested first but finishes last; `call_fast` is requested second but
        // finishes first.
        let slow = "sleep 0.5; echo SLOW_RESULT";
        let fast = "echo FAST_RESULT";

        let fixture = Fixture::start(vec![
            ScriptedResponse::sse(&[
                tool_call_frame(0, "call_slow", "bash", &bash_args(slow)),
                tool_call_frame(1, "call_fast", "bash", &bash_args(fast)),
                tool_calls_done_frame(),
            ]),
            ScriptedResponse::sse(&[text_frame("Done with both tools."), final_done_frame()]),
        ]);

        let output = run(cortex_jsonl_command(
            &home,
            Some(&fixture.base_url),
            "run a slow and a fast tool",
        ));
        let request_bodies = fixture.request_bodies();
        fixture.finish();

        assert_eq!(
            output.status.code(),
            Some(0),
            "run {attempt} must succeed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );

        let events = parse_jsonl(&output.stdout);

        // The fast tool completes first, so its result surfaces first in the stream.
        let completion_order: Vec<String> = events
            .iter()
            .filter(|event| event["type"] == "tool_result")
            .filter_map(|event| event["id"].as_str().map(ToOwned::to_owned))
            .collect();
        assert_eq!(
            completion_order,
            vec!["call_fast".to_owned(), "call_slow".to_owned()],
            "run {attempt}: tool results should surface in completion order (fast then slow)"
        );

        // The post-tool continuation request reinserts tool results in original tool-call order,
        // with each call id paired to its own output.
        let continuation = request_bodies
            .iter()
            .find(|body| body.contains("\"role\":\"tool\"") || body.contains("\"role\": \"tool\""))
            .expect("a continuation request carrying tool results");
        let parsed: serde_json::Value =
            serde_json::from_str(continuation).expect("continuation request is valid json");
        let tool_messages: Vec<&serde_json::Value> = parsed["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();

        let reinjected: Vec<String> = tool_messages
            .iter()
            .filter_map(|message| message["tool_call_id"].as_str().map(ToOwned::to_owned))
            .collect();
        assert_eq!(
            reinjected,
            vec!["call_slow".to_owned(), "call_fast".to_owned()],
            "run {attempt}: tool results must reinsert in original tool-call order (slow then fast)"
        );

        // Correct call id / payload pairing.
        let slow_message = tool_messages
            .iter()
            .find(|message| message["tool_call_id"] == "call_slow")
            .expect("slow tool message");
        let fast_message = tool_messages
            .iter()
            .find(|message| message["tool_call_id"] == "call_fast")
            .expect("fast tool message");
        assert!(
            slow_message["content"]
                .as_str()
                .is_some_and(|content| content.contains("SLOW_RESULT")),
            "run {attempt}: call_slow must be paired with the slow tool output"
        );
        assert!(
            fast_message["content"]
                .as_str()
                .is_some_and(|content| content.contains("FAST_RESULT")),
            "run {attempt}: call_fast must be paired with the fast tool output"
        );

        reinjected_orders.push(reinjected);
    }

    // Determinism across repeated runs.
    assert!(
        reinjected_orders.windows(2).all(|pair| pair[0] == pair[1]),
        "reinjected transcript order must be deterministic across runs: {reinjected_orders:?}"
    );
}

// VAL-GAP-ENGINE-001 (live OpenRouter proof): the same concurrent slow-tool behavior holds with a
// real provider. Runtime-gated on OPENROUTER_API_KEY so the offline workspace suite stays green;
// runs automatically when the key is exported during mission validation.
#[test]
fn live_openrouter_two_slow_tools_run_end_to_end() {
    if std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .is_none()
    {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    }

    let home = scratch_home("live");
    let prompt = "You have a bash tool. In a SINGLE step, call the bash tool TWICE in parallel \
        (two tool calls in the same step). First command exactly: \
        echo ALPHA_START $(date +%s.%N); sleep 2; echo ALPHA_END $(date +%s.%N). \
        Second command exactly: \
        echo BETA_START $(date +%s.%N); sleep 2; echo BETA_END $(date +%s.%N). \
        After both finish, reply with the two ALPHA_END and BETA_END epoch values you observed.";

    let output = run(cortex_jsonl_command(&home, None, prompt));

    assert_eq!(
        output.status.code(),
        Some(0),
        "live run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let events = parse_jsonl(&output.stdout);
    let tool_results: Vec<&serde_json::Value> = events
        .iter()
        .filter(|event| event["type"] == "tool_result")
        .collect();
    assert!(
        tool_results.len() >= 2,
        "live model should have invoked the bash tool at least twice; got {}",
        tool_results.len()
    );

    let combined: String = tool_results
        .iter()
        .map(|event| tool_result_text(event))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("ALPHA_END") && combined.contains("BETA_END"),
        "both slow tools must have completed end-to-end live: {combined}"
    );

    // The runner emits all tool calls of a step before any of that step's results. When the model
    // issued both calls in one step, prove concurrency from the tools' own clocks; otherwise note
    // that the live model serialized the calls (the deterministic test already proves the engine
    // dispatches concurrently).
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    let two_calls_before_any_result = kinds
        .iter()
        .take_while(|kind| **kind != "tool_result")
        .filter(|kind| **kind == "tool_call")
        .count()
        >= 2;

    if two_calls_before_any_result {
        let alpha_start = marker_time(&combined, "ALPHA_START");
        let alpha_end = marker_time(&combined, "ALPHA_END");
        let beta_start = marker_time(&combined, "BETA_START");
        let beta_end = marker_time(&combined, "BETA_END");
        if let (Some(a_s), Some(a_e), Some(b_s), Some(b_e)) =
            (alpha_start, alpha_end, beta_start, beta_end)
        {
            assert!(
                a_s < b_e && b_s < a_e,
                "live concurrent tool intervals should overlap: \
                 alpha=[{a_s}, {a_e}] beta=[{b_s}, {b_e}]"
            );
            let span = a_e.max(b_e) - a_s.min(b_s);
            assert!(
                span < 3.5,
                "live concurrent span should be close to the slowest 2s call, not the 4s sum: {span}s"
            );
        }
    } else {
        eprintln!(
            "live model serialized the two tool calls across steps; concurrency proven by the \
             deterministic local-provider test"
        );
    }

    let answer: String = events
        .iter()
        .filter(|event| event["type"] == "text_delta")
        .filter_map(|event| event["text"].as_str())
        .collect();
    assert!(
        !answer.trim().is_empty(),
        "live run should produce a final assistant answer"
    );
}
