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
                    if responses.lock().expect("responses lock").is_empty() {
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
        "cortex-gap-task-todo-bash-{tag}-{}-{}",
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

fn cortex_run_json(
    home: &Path,
    base_url: &str,
    model: &str,
    prompt: &str,
    extra_args: &[&str],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(home).arg("run");
    for arg in extra_args {
        command.arg(arg);
    }
    command
        .arg("--model")
        .arg(model)
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

fn cortex_session_show_json(home: &Path, session_id: &str) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .arg("session")
        .arg("show")
        .arg(session_id)
        .arg("--format")
        .arg("json")
        .env("HOME", home)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    assert_success_json(run(command))
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

fn extract_task_id(task_text: &str) -> String {
    task_text
        .lines()
        .find_map(|line| line.strip_prefix("task_id: ").map(str::to_owned))
        .expect("task output includes task_id")
}

#[test]
fn task_supports_documented_parameters_and_task_id_resume() {
    let home = scratch_home("task-resume");
    let parent_session = "task-resume-parent";

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_task_1",
                "task",
                serde_json::json!({
                    "subagentType":"explore",
                    "description":"Inspect fixtures",
                    "command":"scan repo",
                    "prompt":"Reply with FIRST-123"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("FIRST-123"), done_frame("stop")]),
        ScriptedResponse::success(&[text_delta_frame("parent saw FIRST-123"), done_frame("stop")]),
    ]);
    let first = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "exercise task documented params",
        &["--session", parent_session],
    ));
    fixture.finish();

    let first_task = tool_result_texts(&first, "task").join("\n");
    let task_id = extract_task_id(&first_task);
    assert!(
        first_task.contains("subagent_type: explore"),
        "got: {first_task}"
    );
    assert!(
        first_task.contains("command: scan repo"),
        "got: {first_task}"
    );
    assert!(
        first_task.contains("description: Inspect fixtures"),
        "got: {first_task}"
    );
    assert!(first_task.contains("resumed: false"), "got: {first_task}");

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_task_2",
                "task",
                serde_json::json!({
                    "subagent_type":"explore",
                    "description":"Resume fixtures",
                    "command":"scan repo --resume",
                    "task_id": task_id,
                    "prompt":"Reply with SECOND-456"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("SECOND-456"), done_frame("stop")]),
        ScriptedResponse::success(&[
            text_delta_frame("parent saw SECOND-456"),
            done_frame("stop"),
        ]),
    ]);
    let second = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "resume task by task_id",
        &["--session", parent_session],
    ));
    fixture.finish();

    let second_task = tool_result_texts(&second, "task").join("\n");
    assert!(second_task.contains(&format!("task_id: {task_id}")));
    assert!(second_task.contains("resumed: true"), "got: {second_task}");
    assert!(second_task.contains("SECOND-456"), "got: {second_task}");

    let child = cortex_session_show_json(&home, &task_id);
    let messages = child["messages"].as_array().expect("messages");
    assert!(
        messages.iter().any(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("FIRST-123"))
        }),
        "{child}"
    );
    assert!(
        messages.iter().any(|message| {
            message["role"] == "user"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("SECOND-456"))
        }),
        "{child}"
    );
}

#[test]
fn task_advertisement_contains_exactly_the_parent_permitted_subagent_targets() {
    let home = scratch_home("task-advertisement");
    let fixture = Fixture::start(vec![ScriptedResponse::success(&[
        text_delta_frame("no delegation needed"),
        done_frame("stop"),
    ])]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "return a short confirmation",
        &[
            "--permission-allow",
            "task:plan",
            "--permission-allow",
            "task:explore",
            "--permission-deny",
            "task:general",
        ],
    ));
    fixture.finish();

    let task = doc["tool_definitions"]
        .as_array()
        .expect("tool definitions")
        .iter()
        .find(|definition| definition["name"] == "task")
        .expect("task should be advertised when any target is permitted");
    let targets = task["parameters"]["properties"]["subagent_type"]["description"]
        .as_str()
        .expect("task target description");
    assert!(
        targets.contains("Available targets in this session: plan, explore."),
        "only the explicitly allowed targets should be advertised: {targets}"
    );
    assert!(
        !targets.contains("general") && !targets.contains("build"),
        "forbidden and primary targets must not be offered: {targets}"
    );
}

#[test]
fn task_subagent_inherits_a_parent_write_deny_without_side_effects() {
    let home = scratch_home("task-parent-write-deny");
    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_parent_task",
                "task",
                serde_json::json!({
                    "subagent_type": "general",
                    "prompt": "Write DENIED_CHILD_WRITE to denied-child.txt."
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_child_write",
                "write",
                serde_json::json!({
                    "path": "denied-child.txt",
                    "content": "DENIED_CHILD_WRITE"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            text_delta_frame("the delegated write was denied"),
            done_frame("stop"),
        ]),
        ScriptedResponse::success(&[
            text_delta_frame("parent incorporated the denied child result"),
            done_frame("stop"),
        ]),
    ]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "delegate a file write to a general subagent",
        &["--permission-deny", "write:**"],
    ));
    fixture.finish();

    let task_result = tool_result_texts(&doc, "task").join("\n");
    assert!(
        task_result.contains("write: permission denied"),
        "the child result must report the parent-derived denial: {task_result}"
    );
    assert!(
        !home.join("denied-child.txt").exists(),
        "a parent-denied write must remain absent after the child attempts it"
    );
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("incorporated the denied child result")),
        "the parent must receive the child result before its final answer: {doc}"
    );
}

#[test]
fn todo_round_trips_priority_and_cancelled_status() {
    let home = scratch_home("todo-priority-cancelled");

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_todo_write",
                "todo",
                serde_json::json!({
                    "todos": [
                        {"content":"ship gap fix","status":"cancelled","priority":"high"},
                        {"content":"follow-up","status":"in_progress"}
                    ]
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("todo saved"), done_frame("stop")]),
    ]);
    let first = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "write todo with priority",
        &[],
    ));
    fixture.finish();

    let session_id = first["session"].as_str().expect("session id").to_owned();
    let first_todo = tool_result_texts(&first, "todo").join("\n");
    assert!(
        first_todo.contains("1. [cancelled][high] ship gap fix"),
        "got: {first_todo}"
    );
    assert!(
        first_todo.contains("2. [in_progress][medium] follow-up"),
        "got: {first_todo}"
    );

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame("call_todo_read", "todo", serde_json::json!({})),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("todo read"), done_frame("stop")]),
    ]);
    let second = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "read todo list",
        &["--session", &session_id],
    ));
    fixture.finish();

    let second_todo = tool_result_texts(&second, "todo").join("\n");
    assert!(
        second_todo.contains("1. [cancelled][high] ship gap fix"),
        "got: {second_todo}"
    );
    assert!(
        second_todo.contains("2. [in_progress][medium] follow-up"),
        "got: {second_todo}"
    );
}

#[test]
fn bash_workdir_enforces_external_directory_permissions() {
    let home = scratch_home("bash-workdir-perms");
    static OUTSIDE_COUNTER: AtomicU32 = AtomicU32::new(0);
    let outside = PathBuf::from("/projects/cortex-code/target").join(format!(
        "cortex-gap-bash-workdir-{}-{}",
        std::process::id(),
        OUTSIDE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&outside).expect("outside dir");

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_bash",
                "bash",
                serde_json::json!({
                    "command":"pwd",
                    "workdir": outside.to_string_lossy()
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("done"), done_frame("stop")]),
    ]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "bash external workdir should deny",
        &["--permission-deny", "external_directory:*"],
    ));
    fixture.finish();
    let _ = fs::remove_dir_all(&outside);

    let bash = tool_result_texts(&doc, "bash").join("\n");
    assert!(
        bash.contains("permission denied for workdir"),
        "got: {bash}"
    );
}
