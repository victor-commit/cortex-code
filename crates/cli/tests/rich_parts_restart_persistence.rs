//! Deterministic black-box coverage for VAL-GAP-ENGINE-003 rich-part restart evidence.
//!
//! Uses only shipped CLI/provider/session/export surfaces with a local OpenAI-compatible fixture:
//! - checkpoint snapshot part from a real mutating tool turn
//! - retry part from bounded runner retries
//! - compaction part from low-budget history compaction
//! - reasoning/tool/step/patch/file parts from streamed tool turns

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(45);

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

    fn retryable_503() -> Self {
        Self {
            status_line: "HTTP/1.1 503 Service Unavailable",
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: r#"{"error":"transient upstream"}"#.to_owned(),
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
            let idle_deadline = Instant::now() + Duration::from_secs(15);
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

fn tool_call_frame(call_id: &str, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "id": "rich-parts-fixture",
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
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn text_delta_frame(text: &str) -> String {
    serde_json::json!({
        "id": "rich-parts-fixture",
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

fn reasoning_delta_frame(text: &str) -> String {
    serde_json::json!({
        "id": "rich-parts-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": { "reasoning": text },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn done_frame(reason: &str, usage: Option<serde_json::Value>) -> String {
    let mut value = serde_json::json!({
        "id": "rich-parts-fixture",
        "object": "chat.completion.chunk",
        "model": "google/gemini-2.5-flash",
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": reason
        }]
    });
    if let Some(usage) = usage {
        value["usage"] = usage;
    }
    value.to_string()
}

fn scratch_home(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-rich-parts-restart-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch home");
    dir
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

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
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
        stdout: out_thread.join().expect("stdout join"),
        stderr: err_thread.join().expect("stderr join"),
    }
}

fn cortex(
    home: &Path,
    args: &[&str],
    base_url: Option<&str>,
    extra_env: &[(&str, &str)],
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(home).args(args).env("HOME", home);
    if let Some(base_url) = base_url {
        command
            .env("OPENROUTER_API_KEY", "test-openrouter-key")
            .env("CORTEX_OPENROUTER_BASE_URL", base_url);
    } else {
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_OPENROUTER_BASE_URL");
    }
    for (key, value) in extra_env {
        command.env(key, value);
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

fn run_json(
    home: &Path,
    args: &[&str],
    base_url: &str,
    extra_env: &[(&str, &str)],
) -> serde_json::Value {
    let out = run(cortex(home, args, Some(base_url), extra_env));
    assert_eq!(
        out.status.code(),
        Some(0),
        "command failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("strict json stdout")
}

fn run_json_without_provider(home: &Path, args: &[&str]) -> serde_json::Value {
    let out = run(cortex(home, args, None, &[]));
    assert_eq!(
        out.status.code(),
        Some(0),
        "command failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("strict json stdout")
}

fn assistant_parts_from_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .rev()
        .find(|message| message["role"] == "assistant")
        .and_then(|message| message["parts"].as_array().cloned())
        .unwrap_or_default()
}

fn session_show_parts(home: &Path, session: &str) -> Vec<serde_json::Value> {
    let doc = run_json_without_provider(home, &["session", "show", session, "--format", "json"]);
    let messages = doc["messages"].as_array().expect("messages array");
    assistant_parts_from_messages(messages)
}

fn export_parts(home: &Path, session: &str) -> Vec<serde_json::Value> {
    let doc = run_json_without_provider(home, &["export", session]);
    let messages = doc["messages"].as_array().expect("messages array");
    assistant_parts_from_messages(messages)
}

fn part_types(parts: &[serde_json::Value]) -> Vec<String> {
    parts
        .iter()
        .filter_map(|part| part["type"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn without_created_at(parts: &[serde_json::Value]) -> Vec<serde_json::Value> {
    parts
        .iter()
        .map(|part| {
            let mut copy = part.clone();
            if let Some(object) = copy.as_object_mut() {
                object.remove("created_at");
            }
            copy
        })
        .collect()
}

fn write_tiny_png(path: &Path) {
    // 1x1 transparent PNG.
    let bytes: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', 0x78, 0x9C, 0x63, 0x60,
        0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE5, 0x27, 0xD4, 0xA2, 0x00, 0x00, 0x00, 0x00, b'I',
        b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
    ];
    fs::write(path, bytes).expect("write tiny png");
}

#[test]
fn snapshot_patch_file_and_reasoning_parts_persist_across_restart_surfaces() {
    let home = scratch_home("snapshot");
    write_tiny_png(&home.join("proof.png"));
    let session = "ses_snapshot_restart";
    let secret = "SECRET-CONTENT-9371";

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_write",
                "write",
                serde_json::json!({
                    "path": "notes.txt",
                    "content": secret,
                }),
            ),
            done_frame("tool_calls", None),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_read",
                "read",
                serde_json::json!({ "path": "proof.png" }),
            ),
            done_frame("tool_calls", None),
        ]),
        ScriptedResponse::success(&[
            reasoning_delta_frame("reasoning trail"),
            text_delta_frame("final answer"),
            done_frame(
                "stop",
                Some(serde_json::json!({
                    "prompt_tokens": 9,
                    "completion_tokens": 4,
                    "total_tokens": 13
                })),
            ),
        ]),
    ]);

    let run_doc = run_json(
        &home,
        &[
            "run",
            "--format",
            "json",
            "--session",
            session,
            "Use tools and then answer.",
        ],
        &fixture.base_url,
        &[],
    );
    fixture.finish();

    let run_parts = run_doc["parts"].as_array().expect("run parts").to_vec();
    let run_types = part_types(&run_parts);
    for required in [
        "text",
        "reasoning",
        "tool",
        "step",
        "patch",
        "file",
        "snapshot",
    ] {
        assert!(
            run_types.iter().any(|kind| kind == required),
            "run output is missing required part type `{required}`: {run_types:?}"
        );
    }

    let run_snapshot = run_parts
        .iter()
        .find(|part| part["type"] == "snapshot")
        .expect("snapshot part in run output");
    assert_eq!(run_snapshot["created"], serde_json::json!(["notes.txt"]));
    assert!(
        !run_snapshot.to_string().contains(secret),
        "snapshot part must not leak full file content: {run_snapshot}"
    );

    let show_parts_first = session_show_parts(&home, session);
    let export_parts_first = export_parts(&home, session);
    let show_parts_second = session_show_parts(&home, session);
    let export_parts_second = export_parts(&home, session);

    assert_eq!(
        show_parts_first, show_parts_second,
        "session show rich parts should be stable across fresh process restarts"
    );
    assert_eq!(
        export_parts_first, export_parts_second,
        "export rich parts should be stable across fresh process restarts"
    );
    assert_eq!(
        show_parts_first, export_parts_first,
        "session show and export should expose the same assistant rich parts"
    );

    let show_snapshot = show_parts_first
        .iter()
        .find(|part| part["type"] == "snapshot")
        .expect("snapshot part in session show");
    assert_eq!(show_snapshot["id"], run_snapshot["id"]);
    assert_eq!(
        show_snapshot["created_at"]
            .as_str()
            .map(str::is_empty)
            .unwrap_or(true),
        false,
        "persisted snapshot part should carry a timestamp"
    );
    assert_eq!(
        without_created_at(std::slice::from_ref(show_snapshot))[0],
        without_created_at(std::slice::from_ref(run_snapshot))[0],
        "snapshot payload should match across run and persisted surfaces"
    );
}

#[test]
fn retry_parts_are_exposed_by_run_session_show_and_export_after_restart() {
    let home = scratch_home("retry");
    let session = "ses_retry_restart";
    let fixture = Fixture::start(vec![
        ScriptedResponse::retryable_503(),
        ScriptedResponse::retryable_503(),
        ScriptedResponse::retryable_503(),
        ScriptedResponse::success(&[
            text_delta_frame("recovered"),
            done_frame(
                "stop",
                Some(serde_json::json!({
                    "prompt_tokens": 2,
                    "completion_tokens": 1,
                    "total_tokens": 3
                })),
            ),
        ]),
    ]);

    let run_doc = run_json(
        &home,
        &[
            "run",
            "--format",
            "json",
            "--session",
            session,
            "Reply with recovered",
        ],
        &fixture.base_url,
        &[],
    );
    fixture.finish();

    let run_parts = run_doc["parts"].as_array().expect("run parts").to_vec();
    let run_retry = run_parts
        .iter()
        .find(|part| part["type"] == "retry")
        .expect("retry part in run output");
    assert_eq!(run_retry["attempt"], 1);
    assert_eq!(run_retry["max_attempts"], 2);

    let show_parts = session_show_parts(&home, session);
    let export_parts = export_parts(&home, session);
    let retry_show = show_parts
        .iter()
        .find(|part| part["type"] == "retry")
        .expect("retry part in session show");
    let retry_export = export_parts
        .iter()
        .find(|part| part["type"] == "retry")
        .expect("retry part in export");

    assert_eq!(retry_show["id"], run_retry["id"]);
    assert_eq!(
        without_created_at(std::slice::from_ref(retry_show))[0],
        without_created_at(std::slice::from_ref(retry_export))[0],
        "retry payload should stay stable between session show and export"
    );
}

#[test]
fn compaction_parts_are_deterministically_induced_and_persisted_across_restart() {
    let home = scratch_home("compaction");
    let session_doc = run_json_without_provider(&home, &["session", "new", "--format", "json"]);
    let session = session_doc["id"].as_str().expect("session id").to_owned();

    let filler = "compaction-seed ".repeat(120);
    for i in 0..20 {
        let out = run(cortex(
            &home,
            &[
                "session",
                "message",
                &session,
                &format!("{filler} turn-{i}"),
                "--role",
                "user",
                "--format",
                "json",
            ],
            None,
            &[],
        ));
        assert_eq!(
            out.status.code(),
            Some(0),
            "failed to seed session history: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            text_delta_frame("history compacted summary"),
            done_frame("stop", None),
        ]),
        ScriptedResponse::success(&[
            text_delta_frame("COMPACTION_OK"),
            done_frame(
                "stop",
                Some(serde_json::json!({
                    "prompt_tokens": 7,
                    "completion_tokens": 2,
                    "total_tokens": 9
                })),
            ),
        ]),
    ]);

    let run_doc = run_json(
        &home,
        &[
            "run",
            "--format",
            "json",
            "--session",
            &session,
            "Reply with exactly COMPACTION_OK",
        ],
        &fixture.base_url,
        &[("CORTEX_CONTEXT_LIMIT_TOKENS", "700")],
    );
    fixture.finish();

    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("COMPACTION_OK")),
        "final response should come from the post-compaction continued request: {run_doc}"
    );
    assert!(
        run_doc["compactions"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "run output should expose compaction metadata: {run_doc}"
    );
    let run_parts = run_doc["parts"].as_array().expect("run parts").to_vec();
    let run_compaction = run_parts
        .iter()
        .find(|part| part["type"] == "compaction")
        .expect("compaction part in run output");

    let show_parts = session_show_parts(&home, &session);
    let export_parts = export_parts(&home, &session);
    let show_compaction = show_parts
        .iter()
        .find(|part| part["type"] == "compaction")
        .expect("compaction part in session show");
    let export_compaction = export_parts
        .iter()
        .find(|part| part["type"] == "compaction")
        .expect("compaction part in export");

    assert_eq!(show_compaction["id"], run_compaction["id"]);
    assert_eq!(
        without_created_at(std::slice::from_ref(show_compaction))[0],
        without_created_at(std::slice::from_ref(export_compaction))[0],
        "compaction payload should stay stable between session show and export"
    );
}
