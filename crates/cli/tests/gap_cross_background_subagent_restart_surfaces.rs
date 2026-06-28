//! VAL-GAP-CROSS-008 (headless legs): one durable background subagent task spans a host restart
//! and surfaces consistently across the CLI task tool, the `cortex session` store, and (proven in a
//! sibling TUI test) the TUI subagent UI.
//!
//! This drives REAL `cortex run` / `cortex session` subprocesses against an OpenAI-compatible
//! loopback fixture, so the durable runner, the `task` tool, the CLI subagent runner (background
//! spawn + at-exit drain + by-id rejoin), and the SQLite session store are all the shipped headless
//! code path; the fixture only supplies deterministic streamed model events. The flow is:
//!
//!   1. Process A: a turn asks for a BACKGROUND subagent task. The model calls `task` with
//!      `background:true`; the tool returns immediately with `state: running` and a stable task id.
//!      The CLI then drains the in-flight child to completion at shutdown, so its transcript is
//!      durably persisted before the process exits. Process A exiting IS the host restart boundary.
//!   2. The child session is listable (`cortex session list`) and its transcript persisted
//!      (`cortex session show <task_id>`) under the same stable id, with `subagent:`/`parent:`
//!      linkage in the title.
//!   3. Process B (a brand-new process): the model rejoins the task BY ID (`task` with `task_id`,
//!      no prompt). The rejoin reads the persisted transcript from disk and feeds the child's result
//!      back into the parent session's final answer.
//!   4. No orphan process is left behind by either run.
//!
//! The TUI leg (the subagent UI showing the same persisted child) lives with its surface in
//! `crates/tui/src/app.rs` (`subagent_statuses` / `subagent_footer_summary_counts_*` /
//! `session_picker_lists_subagent_sessions_with_parent_linkage`), matching the established
//! per-surface cross-flow proof layout, and is exercised live via tuistory during validation.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(45);
const SERVER_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const PARENT_SESSION: &str = "ses_parent_cross008";

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
        let root = std::env::temp_dir().join(format!("cross-bg-subagent-{tag}-{unique}"));
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

    /// A unique marker present in every cortex process's environment/working dir for this sandbox,
    /// used to scan `/proc` for any orphaned process still referencing this run.
    fn orphan_marker(&self) -> String {
        self.data.to_string_lossy().into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A content-routed OpenAI-compatible loopback fixture. It is stateless: every response is chosen
/// purely from the request body, so concurrent parent + background-child requests (which interleave
/// on the parent's current-thread runtime) are each answered correctly regardless of arrival order.
struct ScriptedServer {
    base_url: String,
    shutdown: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl ScriptedServer {
    fn start(nonce: &str) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let nonce_owned = nonce.to_owned();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_thread = shutdown.clone();
        let handle = thread::spawn(move || {
            let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
            let idle_deadline = Instant::now() + SERVER_IDLE_TIMEOUT;
            while Instant::now() < idle_deadline && !shutdown_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let nonce = nonce_owned.clone();
                        workers.push(thread::spawn(move || handle_connection(stream, &nonce)));
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            shutdown,
            handle,
        }
    }

    fn finish(self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

fn handle_connection(mut stream: TcpStream, nonce: &str) {
    let Ok(body) = read_http_request(&mut stream) else {
        return;
    };
    let response = route(&body, nonce);
    let _ = write_http_sse(&mut stream, &response);
}

/// Choose the model response from the request body. Order matters: tool-result markers
/// (`state: completed`/`state: running`) are checked before the prompt markers they may quote.
fn route(body: &str, nonce: &str) -> String {
    if body.contains("state: completed") {
        // Process B continuation: the rejoin tool result is in history -> parent wraps up.
        final_text_body(&format!("REJOINED {nonce}"))
    } else if body.contains("state: running") {
        // Process A continuation: the background launch tool result is in history -> parent wraps up.
        final_text_body(&format!("PARENT_DONE {nonce}"))
    } else if let Some(task_id) = extract_marker(body, "REJOIN_TASK ") {
        // Process B initial: rejoin the durable task purely by id (no prompt, foreground).
        task_call_rejoin_body(&task_id)
    } else if body.contains("CHILD_WORK") {
        // The background child's own turn -> its durable answer.
        final_text_body(&format!("CHILD_ANSWER {nonce}"))
    } else {
        // Process A initial: spawn the background explore subtask.
        task_call_background_body("explore", &format!("do CHILD_WORK {nonce}"))
    }
}

/// Extract the token immediately following `marker` in the (JSON-escaped) request body, reading the
/// id-shaped characters of a session id (`ses_...`).
fn extract_marker(body: &str, marker: &str) -> Option<String> {
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let id: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
        .collect();
    (!id.is_empty()).then_some(id)
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in mission range 3200-3299");
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(String::from_utf8_lossy(&buffer).into_owned());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
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
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    Ok(String::from_utf8_lossy(&buffer[header_end..]).into_owned())
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

fn task_call_background_body(agent: &str, prompt: &str) -> String {
    let arguments = serde_json::json!({
        "subagent_type": agent,
        "prompt": prompt,
        "description": "durable background work",
        "background": true,
    })
    .to_string();
    tool_call_body("call_bg_task", &arguments)
}

fn task_call_rejoin_body(task_id: &str) -> String {
    let arguments = serde_json::json!({ "task_id": task_id }).to_string();
    tool_call_body("call_rejoin_task", &arguments)
}

fn tool_call_body(call_id: &str, arguments: &str) -> String {
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": "task", "arguments": arguments }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        serde_json::json!({
            "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
        })
        .to_string(),
    ])
}

fn final_text_body(text: &str) -> String {
    sse_body(&[
        serde_json::json!({
            "choices": [{ "delta": { "content": text }, "finish_reason": null }]
        })
        .to_string(),
        serde_json::json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })
        .to_string(),
    ])
}

fn cortex_command(sandbox: &Sandbox, base_url: &str, args: &[String]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("CORTEX_DATA_DIR", &sandbox.data);
    command.env("CORTEX_CHECKPOINT_DIR", &sandbox.checkpoints);
    command.env("OPENROUTER_API_KEY", "test-openrouter-key");
    command.env("CORTEX_OPENROUTER_BASE_URL", base_url);
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

/// Run a cortex subprocess to completion, panicking (rather than hanging) if it does not terminate
/// within the bounded timeout, so a background-drain regression that wedged process exit FAILS here.
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
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?} (pid {pid})");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout reader"),
        stderr: stderr_thread.join().expect("join stderr reader"),
    }
}

fn parse_json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be one strict JSON document: {err}; stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    })
}

fn task_result_text(doc: &serde_json::Value) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool_results array")
        .iter()
        .find(|result| result["name"] == "task")
        .map(|result| {
            result["parts"]
                .as_array()
                .expect("parts array")
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| panic!("missing task tool result in {doc}"))
}

fn field_after(text: &str, prefix: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(prefix).map(str::to_owned))
}

/// Scan `/proc` for any live process whose cmdline/environ references this sandbox, proving neither
/// run left an orphaned child behind. Read-only: it never signals or kills anything.
fn orphan_processes(marker: &str) -> Vec<u32> {
    let mut orphans = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return orphans;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let environ = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        let haystack =
            String::from_utf8_lossy(&cmdline).into_owned() + &String::from_utf8_lossy(&environ);
        if haystack.contains(marker) {
            orphans.push(pid);
        }
    }
    orphans
}

#[test]
fn background_subagent_task_survives_restart_lists_rejoins_and_feeds_parent() {
    let sandbox = Sandbox::new("flow");
    let nonce = format!("n{}", std::process::id());
    let server = ScriptedServer::start(&nonce);

    // --- Process A: spawn the BACKGROUND subagent task within the parent session. -----------------
    let spawn_args = vec![
        "run".to_owned(),
        "--session".to_owned(),
        PARENT_SESSION.to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        "Start a background explore subtask to investigate the repo, then tell me you did."
            .to_owned(),
    ];
    let spawn_out = run(cortex_command(&sandbox, &server.base_url, &spawn_args));
    assert!(
        spawn_out.status.success(),
        "spawn run should exit 0 (stderr: {})",
        String::from_utf8_lossy(&spawn_out.stderr)
    );
    let spawn_doc = parse_json(&spawn_out);
    let spawn_task_text = task_result_text(&spawn_doc);
    assert!(
        spawn_task_text.contains("state: running"),
        "background task launch must report running state: {spawn_task_text}"
    );
    let task_id = field_after(&spawn_task_text, "task_id: ")
        .expect("background task result exposes a stable task_id");
    assert!(
        task_id.starts_with("ses_"),
        "task id should be a durable session id: {task_id}"
    );
    assert_eq!(
        spawn_doc["session"], PARENT_SESSION,
        "the parent turn ran in the requested session"
    );

    // No orphan left by Process A (the background child is an in-process task, not an OS child).
    assert!(
        orphan_processes(&sandbox.orphan_marker()).is_empty(),
        "Process A must leave no orphaned process referencing the sandbox"
    );

    // --- The child task is LISTABLE and its TRANSCRIPT PERSISTED under the same stable id. --------
    let list_out = run(cortex_command(
        &sandbox,
        &server.base_url,
        &[
            "session".to_owned(),
            "list".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
    ));
    assert!(list_out.status.success(), "session list should exit 0");
    let list_doc = parse_json(&list_out);
    let sessions = list_doc["sessions"].as_array().expect("sessions array");
    let child = sessions
        .iter()
        .find(|session| session["id"] == serde_json::Value::String(task_id.clone()))
        .unwrap_or_else(|| {
            panic!("child task {task_id} must be listable after restart: {list_doc}")
        });
    let child_title = child["title"].as_str().unwrap_or_default();
    assert!(
        child_title.contains("subagent:explore")
            && child_title.contains(&format!("parent:{PARENT_SESSION}")),
        "child session title must carry subagent/parent linkage: {child_title}"
    );

    let show_out = run(cortex_command(
        &sandbox,
        &server.base_url,
        &[
            "session".to_owned(),
            "show".to_owned(),
            task_id.clone(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
    ));
    assert!(show_out.status.success(), "session show should exit 0");
    let show_doc = parse_json(&show_out);
    let messages = show_doc["messages"].as_array().expect("messages array");
    assert!(
        messages.iter().any(|m| m["role"] == "user"
            && m["content"]
                .as_str()
                .is_some_and(|c| c.contains("CHILD_WORK"))),
        "child user prompt must persist across restart: {show_doc}"
    );
    assert!(
        messages.iter().any(|m| m["role"] == "assistant"
            && m["content"]
                .as_str()
                .is_some_and(|c| c.contains(&format!("CHILD_ANSWER {nonce}")))),
        "child assistant answer must persist across restart: {show_doc}"
    );

    // --- Process B (a fresh process == a restart): rejoin the task BY ID and feed the parent. -----
    let rejoin_args = vec![
        "run".to_owned(),
        "--session".to_owned(),
        PARENT_SESSION.to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
        format!("REJOIN_TASK {task_id} and report the result"),
    ];
    let rejoin_out = run(cortex_command(&sandbox, &server.base_url, &rejoin_args));
    assert!(
        rejoin_out.status.success(),
        "rejoin run should exit 0 (stderr: {})",
        String::from_utf8_lossy(&rejoin_out.stderr)
    );
    let rejoin_doc = parse_json(&rejoin_out);
    let rejoin_task_text = task_result_text(&rejoin_doc);
    assert!(
        rejoin_task_text.contains("state: completed"),
        "rejoin by id must return the completed task: {rejoin_task_text}"
    );
    assert_eq!(
        field_after(&rejoin_task_text, "task_id: ").as_deref(),
        Some(task_id.as_str()),
        "rejoin must target the SAME stable task id"
    );
    assert!(
        rejoin_task_text.contains(&format!("CHILD_ANSWER {nonce}")),
        "rejoin must surface the durable child result: {rejoin_task_text}"
    );
    // The child's result fed the parent session's final answer.
    assert!(
        rejoin_doc["content"]
            .as_str()
            .is_some_and(|c| c.contains(&format!("REJOINED {nonce}"))),
        "the rejoined result must feed the parent session's final answer: {rejoin_doc}"
    );
    assert_eq!(rejoin_doc["session"], PARENT_SESSION);

    // The parent transcript now durably records both turns (spawn + rejoin) for this session.
    let parent_show = run(cortex_command(
        &sandbox,
        &server.base_url,
        &[
            "session".to_owned(),
            "show".to_owned(),
            PARENT_SESSION.to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ],
    ));
    assert!(parent_show.status.success(), "parent show should exit 0");
    let parent_doc = parse_json(&parent_show);
    let parent_msgs = parent_doc["messages"].as_array().expect("messages array");
    assert!(
        parent_msgs.iter().any(|m| m["role"] == "assistant"
            && m["content"]
                .as_str()
                .is_some_and(|c| c.contains(&format!("REJOINED {nonce}")))),
        "parent session must persist the rejoined final answer: {parent_doc}"
    );

    // No orphan left by Process B either.
    assert!(
        orphan_processes(&sandbox.orphan_marker()).is_empty(),
        "Process B must leave no orphaned process referencing the sandbox"
    );

    server.finish();

    // Sanity: the task id is identical everywhere it surfaced (spawn, list, show, rejoin).
    let listed_ids: BTreeSet<String> = sessions
        .iter()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect();
    assert!(
        listed_ids.contains(&task_id),
        "the stable task id must appear in the session listing"
    );
}
