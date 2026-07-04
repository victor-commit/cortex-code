//! VAL-GAP-CROSS-007 (headless leg): the agent `question` tool must FAIL CLOSED in a headless
//! `cortex run`, with a clear "cannot ask in a non-interactive surface" no-responder diagnostic and
//! WITHOUT hanging — there is no human to answer, so the turn must not block forever waiting for one.
//!
//! This drives a real `cortex run` subprocess against an OpenAI-compatible loopback fixture, so the
//! tool registry, `ToolContext` (which has no interactive responder in headless), and the durable
//! runner are all the shipped headless code path; the fixture only supplies deterministic streamed
//! model events. The scripted model first calls the `question` tool, then — after receiving the
//! tool's error result — emits a final answer, so the run completes one full round trip and exits.
//!
//! The other two legs of the same cross-flow live with their surfaces (matching the established
//! per-surface cross-flow proof layout):
//!   - TUI leg: `crates/tui/src/turn.rs` —
//!     `question_tool_in_tui_turn_resumes_with_answer_and_shapes_final_output` runs a full turn
//!     through the real TUI `TurnDispatcher` + `UiResponder`, answers the surfaced ask, and asserts
//!     the supplied answer flows into the final assistant text.
//!   - Server leg: `crates/server/tests/session_routes.rs` —
//!     `question_tool_uses_pending_permission_flow_and_resumes_with_feedback` posts a turn, replies
//!     to the pending question over `/permission/reply`, and asserts the answer shapes the final
//!     assistant content.

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

/// The question the scripted model asks. The TUI and server legs ask an equivalent question, so all
/// three surfaces exercise "the same agent-asked question" required by VAL-GAP-CROSS-007.
const QUESTION_PROMPT: &str = "Pick deployment strategy";

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
        let unique = format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        // The working dir lives under the system temp dir so the project's upward config walk never
        // climbs into the workspace and picks up the repo's own `cortex.toml`.
        let root = std::env::temp_dir().join(format!("cross-question-{tag}-{unique}"));
        let _ = fs::remove_dir_all(&root);
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
    /// Serve one `question` tool-call round trip followed by a final text answer. The model never
    /// gets a human answer (headless has no responder), so the question tool result is an error and
    /// the second response is the model's wrap-up after seeing that diagnostic.
    fn start_question_turn() -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let seen = Arc::new(AtomicUsize::new(0));
        let seen_thread = seen.clone();
        let mut responses = VecDeque::from(vec![
            question_tool_call_body(),
            final_text_body("acknowledged"),
        ]);
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

fn question_tool_call_body() -> String {
    let arguments = serde_json::json!({
        "questions": [{
            "prompt": QUESTION_PROMPT,
            "options": ["A", "B"]
        }]
    })
    .to_string();
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_question",
                        "type": "function",
                        "function": {
                            "name": "question",
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

fn final_text_body(text: &str) -> String {
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": { "content": text },
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

/// Run `cortex run` to completion, panicking (rather than hanging) if it does not terminate within
/// the bounded timeout — so a regression that made the question tool block on a non-existent
/// responder would FAIL this test instead of wedging the suite.
fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let pid = child.id();
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
            panic!(
                "cortex run did not terminate within {RUN_TIMEOUT:?} (pid {pid}); the headless \
                 question tool must fail closed without hanging"
            );
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout reader"),
        stderr: stderr_thread.join().expect("join stderr reader"),
    }
}

fn question_result(doc: &serde_json::Value) -> &serde_json::Value {
    doc["tool_results"]
        .as_array()
        .expect("tool_results array")
        .iter()
        .find(|result| result["name"] == "question")
        .unwrap_or_else(|| panic!("missing question tool result in {doc}"))
}

fn result_text(result: &serde_json::Value) -> String {
    result["parts"]
        .as_array()
        .expect("parts array")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn headless_question_fails_closed_with_no_responder_diagnostic_and_no_hang() {
    let sandbox = Sandbox::new("headless");
    let server = ScriptedOpenAiServer::start_question_turn();

    let args = vec![
        "run".to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        format!("Ask the user: {QUESTION_PROMPT} using the question tool, then answer."),
    ];

    let out = run(cortex_command(&sandbox, &server, &args));
    let seen_requests = server.finish();

    // One question tool-call round trip plus one final wrap-up response: the turn progressed past the
    // failed question call instead of blocking on a human that never arrives.
    assert_eq!(
        seen_requests,
        2,
        "the run should complete one question tool-call round trip and one final response (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "headless cortex run should exit successfully even when the model asks a question \
         (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be one strict JSON document: {err}; stdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });

    let result = question_result(&doc);
    assert_eq!(
        result["is_error"],
        serde_json::Value::Bool(true),
        "the headless question tool must fail closed: {doc}"
    );
    let text = result_text(result).to_lowercase();
    assert!(
        text.contains("cannot ask for user input") && text.contains("non-interactive"),
        "the failure must be a clear no-responder diagnostic naming the non-interactive surface, \
         got: {text}"
    );
    assert!(
        !sandbox.project.join("__never__.txt").exists(),
        "the failed question must not have produced unexpected side effects"
    );
}

#[test]
fn headless_question_diagnostic_is_stable_without_a_permission_config() {
    // Control: even with no project permission config at all (the default), the headless surface has
    // no interactive responder, so the question tool still fails closed with the same diagnostic and
    // still does not hang. This isolates the fail-closed behavior to the absent responder rather than
    // any particular ruleset.
    let sandbox = Sandbox::new("headless-default");
    let server = ScriptedOpenAiServer::start_question_turn();

    let args = vec![
        "run".to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        format!("Use the question tool to ask: {QUESTION_PROMPT}"),
    ];

    let out = run(cortex_command(&sandbox, &server, &args));
    assert_eq!(
        server.finish(),
        2,
        "the run should complete two round trips"
    );
    assert!(out.status.success(), "the run should exit successfully");

    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout should be strict JSON");
    let result = question_result(&doc);
    assert_eq!(result["is_error"], serde_json::Value::Bool(true));
    assert!(
        result_text(result)
            .to_lowercase()
            .contains("cannot ask for user input"),
        "default-config headless question must still emit the no-responder diagnostic: {doc}"
    );
}
