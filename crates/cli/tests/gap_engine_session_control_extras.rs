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
                        "arguments": arguments.to_string()
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
        "cortex-gap-engine-extras-{tag}-{}-{}",
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

fn cortex_run_json(home: &Path, base_url: &str, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .arg("run")
        .arg("--format")
        .arg("json")
        .args(args)
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

fn tool_result_text(doc: &serde_json::Value, tool_name: &str) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .find(|result| result["name"] == tool_name)
        .expect("target tool result")
        .get("parts")
        .and_then(serde_json::Value::as_array)
        .expect("parts")
        .iter()
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn session_show_json(home: &Path, id: &str) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["session", "show", id, "--format", "json"])
        .env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE");
    let out = run(command);
    assert_eq!(
        out.status.code(),
        Some(0),
        "session show failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("valid session json")
}

#[test]
fn structured_schema_is_enforced_for_json_run_output() {
    let home = scratch_home("schema-valid");
    let fixture = Fixture::start(vec![ScriptedResponse::success(&[
        text_delta_frame(r#"{"name":"cortex","ok":true}"#),
        done_frame("stop"),
    ])]);

    let out = cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--schema",
            r#"{"type":"object","required":["name","ok"],"properties":{"name":{"type":"string"},"ok":{"type":"boolean"}},"additionalProperties":false}"#,
            "return strict JSON",
        ],
    );
    fixture.finish();

    assert_eq!(
        out.status.code(),
        Some(0),
        "schema-valid run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert_eq!(doc["structured_output"]["name"], "cortex");
    assert_eq!(doc["structured_output"]["ok"], true);
}

#[test]
fn structured_schema_failure_returns_typed_schema_diagnostic() {
    let home = scratch_home("schema-invalid");
    let fixture = Fixture::start(vec![ScriptedResponse::success(&[
        text_delta_frame("not json"),
        done_frame("stop"),
    ])]);

    let out = cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--schema",
            r#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}"#,
            "return strict JSON",
        ],
    );
    fixture.finish();

    assert_ne!(out.status.code(), Some(0), "schema-invalid run must fail");
    assert!(
        out.stdout.is_empty(),
        "schema-invalid run must not emit a success JSON document"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("schema-validation"),
        "failure should be typed as schema-validation: {stderr}"
    );
}

#[test]
fn repeated_identical_tool_calls_trigger_loop_stop_before_extra_side_effects() {
    let home = scratch_home("doom-loop");
    let target = home.join("doom-loop.txt");
    let command = format!("echo LOOP >> {}", target.display());
    let args = serde_json::json!({ "command": command });
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame("call_1", "bash", args.clone()),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame("call_2", "bash", args.clone()),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame("call_3", "bash", args),
            done_frame("tool_calls"),
        ]),
    ]);

    let out = cortex_run_json(&home, &fixture.base_url, &["run repeated command"]);
    fixture.finish();

    assert_ne!(
        out.status.code(),
        Some(0),
        "doom-loop run should stop with a non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        stderr.contains("doom-loop"),
        "loop stop should be explicit in stderr: {stderr}"
    );
    let content = fs::read_to_string(&target).expect("side-effect file exists");
    assert_eq!(
        content.lines().count(),
        2,
        "only the first two calls should execute before loop stop"
    );
}

#[test]
fn advisory_question_and_plan_tools_are_permission_gated_deterministically() {
    // VAL-GAP-PERM-004 deterministic fixture: scripted provider emits explicit advisory tool calls
    // so coverage does not depend on live model tool-call luck.
    let cases = [
        (
            "question",
            serde_json::json!({"questions":[{"prompt":"Which option?"}]}),
            "question:*",
            "non-interactive surface",
        ),
        (
            "plan-enter",
            serde_json::json!({}),
            "plan-enter:*",
            "permission denied",
        ),
        (
            "plan-exit",
            serde_json::json!({}),
            "plan-exit:*",
            "permission denied",
        ),
    ];

    for (tool_name, args, deny_rule, expected) in cases {
        let home = scratch_home(&format!("perm004-{tool_name}-deny"));
        let fixture = Fixture::start(vec![
            ScriptedResponse::success(&[
                tool_call_frame("call_advisory", tool_name, args.clone()),
                done_frame("tool_calls"),
            ]),
            ScriptedResponse::success(&[text_delta_frame("parent-done"), done_frame("stop")]),
        ]);

        let doc = assert_success_json(cortex_run_json(
            &home,
            &fixture.base_url,
            &["--permission-deny", deny_rule, "exercise advisory deny"],
        ));
        fixture.finish();

        let text = tool_result_text(&doc, tool_name).to_ascii_lowercase();
        assert!(
            text.contains(expected),
            "{tool_name} deny should be surfaced deterministically: {text}"
        );
    }
}

#[test]
fn loop_safeguard_permission_deny_allows_repeated_rounds_to_continue() {
    // VAL-GAP-PERM-004 deterministic fixture: repeated identical tool-call rounds are scripted so
    // loop-safeguard permission behavior is deterministic and reproducible.
    let home = scratch_home("perm004-loop-safeguard-deny");
    let target = home.join("loop-safeguard.txt");
    let command = format!("echo LOOP >> {}", target.display());
    let args = serde_json::json!({ "command": command });
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame("call_1", "bash", args.clone()),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame("call_2", "bash", args.clone()),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame("call_3", "bash", args),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("completed"), done_frame("stop")]),
    ]);

    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--permission-allow",
            "bash:*",
            "--permission-deny",
            "loop-safeguard:*",
            "run repeated command with safeguard denied",
        ],
    ));
    fixture.finish();

    assert_eq!(doc["content"], "completed");
    let content = fs::read_to_string(&target).expect("side-effect file exists");
    assert_eq!(
        content.lines().count(),
        3,
        "denied safeguard must not stop repeated rounds from executing"
    );
}

#[test]
fn session_switch_keeps_prior_transcript_and_applies_new_model_to_later_turns() {
    let home = scratch_home("switch");
    let session = "ses_switch_models";
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[text_delta_frame("first-turn"), done_frame("stop")]),
        ScriptedResponse::success(&[text_delta_frame("second-turn"), done_frame("stop")]),
    ]);

    let first = cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--session",
            session,
            "--model",
            "google/gemini-2.5-flash",
            "first prompt",
        ],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "first run failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_doc: serde_json::Value = serde_json::from_slice(&first.stdout).expect("first json");
    assert_eq!(first_doc["model"], "google/gemini-2.5-flash");
    assert_eq!(first_doc["content"], "first-turn");

    let second = cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--session",
            session,
            "--model",
            "anthropic/claude-sonnet-4",
            "second prompt",
        ],
    );
    fixture.finish();
    assert_eq!(
        second.status.code(),
        Some(0),
        "second run failed: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_doc: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second json");
    assert_eq!(second_doc["model"], "anthropic/claude-sonnet-4");
    assert_eq!(second_doc["content"], "second-turn");

    let detail = session_show_json(&home, session);
    let messages = detail["messages"].as_array().expect("messages array");
    let assistant_contents: Vec<&str> = messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .filter_map(|message| message["content"].as_str())
        .collect();
    assert_eq!(
        assistant_contents,
        vec!["first-turn", "second-turn"],
        "switch should affect later turns without rewriting earlier transcript"
    );
}

#[test]
fn generated_titles_and_diff_summaries_are_persisted_and_visible() {
    let home = scratch_home("title-diff");
    let session = "ses_title_diff";
    let target = home.join("summary-target.txt");
    let tool_args = serde_json::json!({
        "path": target.file_name().expect("filename").to_string_lossy(),
        "content": "DIFF-SUMMARY-MARKER"
    });
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame("call_write", "write", tool_args),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("done"), done_frame("stop")]),
    ]);

    let out = cortex_run_json(
        &home,
        &fixture.base_url,
        &[
            "--session",
            session,
            "Create summary-target.txt with DIFF-SUMMARY-MARKER",
        ],
    );
    fixture.finish();
    assert_eq!(
        out.status.code(),
        Some(0),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        target.exists(),
        "the scripted tool call should produce a changed file"
    );

    let mut list = Command::new(env!("CARGO_BIN_EXE_cortex"));
    list.current_dir(&home)
        .args(["session", "list", "--format", "json"])
        .env("HOME", &home);
    let listed = run(list);
    assert_eq!(
        listed.status.code(),
        Some(0),
        "session list failed: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let list_doc: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("list json");
    let listed_session = list_doc["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .find(|item| item["id"] == session)
        .expect("session row");
    assert!(
        listed_session["title"]
            .as_str()
            .is_some_and(|title| !title.trim().is_empty()),
        "session should have an auto-generated title: {listed_session:?}"
    );

    let detail = session_show_json(&home, session);
    let patch_parts = detail["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .flat_map(|message| {
            message["parts"]
                .as_array()
                .into_iter()
                .flat_map(|parts| parts.iter())
        })
        .filter(|part| part["type"] == "patch")
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !patch_parts.is_empty(),
        "changed turn should persist diff-summary patch parts: {detail:?}"
    );
    assert!(
        patch_parts.iter().any(|part| {
            part["diff"]
                .as_str()
                .is_some_and(|diff| diff.contains("summary-target.txt"))
        }),
        "diff summary should name changed file path: {patch_parts:?}"
    );
}
