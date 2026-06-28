//! VAL-GAP-CROSS-005: a saved "always allow" for a file mutator must never let the agent cross the
//! external-directory boundary. This locks the headless half of the cross-flow: a previously saved
//! `write` allow is honored for an inside-workspace mutation, yet an outside-workspace write is
//! still refused by the external-directory gate with no byte written.
//!
//! The test drives real `cortex run` subprocesses against an OpenAI-compatible loopback fixture, so
//! the saved-decision store, permission engine, external-directory gate, and write tool are all the
//! shipped headless code path; the fixture only supplies deterministic streamed model events.
//!
//! Two scratch locations are used on purpose. The project (the run's working dir) lives under the
//! system temp dir so the upward config walk does not climb into the workspace and pick up the
//! repo's own `cortex.toml`. The "outside" directory lives under `CARGO_TARGET_TMPDIR` (the
//! workspace `target/` tree, NOT under the system temp dir), because the external-directory gate
//! allowlists `std::env::temp_dir()` — an "outside" sibling under `/tmp` would slip through the
//! allowlist and defeat the gate.

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

/// The project asks before any write (so a saved allow is the load-bearing decision for an inside
/// mutation) and hard-denies every external-directory access (so the saved write allow cannot be
/// stretched across the workspace boundary).
const PROJECT_CONFIG: &str = "\
[[permission]]
tool = \"write\"
pattern = \"*\"
action = \"ask\"

[[permission]]
tool = \"external_directory\"
pattern = \"*\"
action = \"deny\"
";

struct Sandbox {
    root: PathBuf,
    outside_root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    outside: PathBuf,
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
        let root = std::env::temp_dir().join(format!("cross-external-{tag}-{unique}"));
        // The outside dir lives under the workspace target tree, NOT the system temp dir, so it is a
        // genuine out-of-workspace path that the external-directory gate (which allowlists the
        // system temp dir) actually gates.
        let outside_root =
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("outside-{tag}-{unique}"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside_root);
        let sandbox = Self {
            home: root.join("home"),
            project: root.join("project"),
            data: root.join("data"),
            checkpoints: root.join("checkpoints"),
            outside: outside_root.join("dir"),
            root,
            outside_root,
        };
        for dir in [
            &sandbox.home,
            &sandbox.project,
            &sandbox.data,
            &sandbox.checkpoints,
            &sandbox.outside,
        ] {
            fs::create_dir_all(dir).expect("create sandbox dir");
        }
        assert!(
            !sandbox.outside.starts_with(std::env::temp_dir()),
            "the outside dir must live outside the system temp dir so the external-directory gate \
             applies; got {:?}",
            sandbox.outside
        );
        fs::write(sandbox.project.join("cortex.toml"), PROJECT_CONFIG)
            .expect("write project config");
        sandbox
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.outside_root);
    }
}

struct ScriptedOpenAiServer {
    base_url: String,
    seen: Arc<AtomicUsize>,
    handle: thread::JoinHandle<()>,
}

impl ScriptedOpenAiServer {
    /// Serve one write tool-call round trip followed by a final text answer.
    fn start_write_turn(path: &str, content: &str) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let seen = Arc::new(AtomicUsize::new(0));
        let seen_thread = seen.clone();
        let mut responses =
            VecDeque::from(vec![write_tool_call_body(path, content), final_text_body()]);
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

fn write_tool_call_body(path: &str, content: &str) -> String {
    let arguments = serde_json::json!({ "path": path, "content": content }).to_string();
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_write",
                        "type": "function",
                        "function": {
                            "name": "write",
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

fn final_text_body() -> String {
    sse_body(&[
        serde_json::json!({
            "choices": [{
                "delta": { "content": "DONE" },
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
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
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout reader"),
        stderr: stderr_thread.join().expect("join stderr reader"),
    }
}

/// Drive one `cortex run` that asks the scripted model to write `content` to `path`, returning the
/// parsed strict-JSON result document.
fn run_scripted_write(
    sandbox: &Sandbox,
    extra_args: &[&str],
    path: &str,
    content: &str,
) -> serde_json::Value {
    let server = ScriptedOpenAiServer::start_write_turn(path, content);
    let mut args = vec![
        "run".to_owned(),
        "--model".to_owned(),
        MODEL.to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    args.extend(extra_args.iter().map(|arg| (*arg).to_owned()));
    args.push(format!(
        "Write {content:?} to {path} using the write tool, then answer DONE."
    ));

    let out = run(cortex_command(sandbox, &server, &args));
    let seen_requests = server.finish();
    assert_eq!(
        seen_requests,
        2,
        "the run should complete one write tool-call round trip and one final response (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "cortex run should exit successfully (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be one strict JSON document: {err}; stdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn write_result(doc: &serde_json::Value) -> &serde_json::Value {
    doc["tool_results"]
        .as_array()
        .expect("tool_results array")
        .iter()
        .find(|result| result["name"] == "write")
        .unwrap_or_else(|| panic!("missing write tool result in {doc}"))
}

fn write_text(doc: &serde_json::Value) -> String {
    write_result(doc)["parts"]
        .as_array()
        .expect("parts array")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_write_allowed(doc: &serde_json::Value) {
    assert_eq!(
        write_result(doc)["is_error"],
        serde_json::Value::Bool(false),
        "write should be allowed: {doc}"
    );
}

fn assert_write_denied(doc: &serde_json::Value) {
    let result = write_result(doc);
    assert_eq!(
        result["is_error"],
        serde_json::Value::Bool(true),
        "write should be denied: {doc}"
    );
    let text = write_text(doc).to_lowercase();
    assert!(
        text.contains("permission denied"),
        "denied write result should explain the permission block, got: {text}"
    );
}

#[test]
fn saved_write_allow_is_honored_inside_but_cannot_cross_the_external_directory_boundary() {
    let sandbox = Sandbox::new("headless");
    let outside_file = sandbox.outside.join("outside.txt");
    fs::write(&outside_file, "OUTSIDE-KEEP\n").expect("seed outside file");

    // --- Seed: ask for the inside write, answer "always allow", and persist the saved decision. ---
    let seed = run_scripted_write(
        &sandbox,
        &["--permission-allow-save", "write:**"],
        "seed.txt",
        "seeded",
    );
    assert_write_allowed(&seed);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("seed.txt"))
            .expect("seed file created")
            .trim(),
        "seeded"
    );

    // --- Inside: a later run with NO save flag honors the saved allow (no responder, no re-ask). ---
    let inside = run_scripted_write(&sandbox, &[], "inside.txt", "inside-allowed");
    assert_write_allowed(&inside);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("inside.txt"))
            .expect("inside file written through saved allow")
            .trim(),
        "inside-allowed",
        "the saved write allow continues to permit inside-workspace mutation"
    );

    // --- Outside: the external-directory hard deny blocks the same saved-allowed write tool. ---
    let outside_target = outside_file.to_string_lossy().to_string();
    let outside = run_scripted_write(&sandbox, &[], &outside_target, "OUTSIDE-CHANGED");
    assert_write_denied(&outside);
    assert_eq!(
        fs::read_to_string(&outside_file)
            .expect("outside file still present")
            .trim(),
        "OUTSIDE-KEEP",
        "a saved write allow must never let the agent mutate a file outside the workspace"
    );
}

#[test]
fn without_the_saved_allow_the_inside_write_fails_closed() {
    // Control: the inside write is gated to `ask` and there is no saved decision, so headless (which
    // has no responder) must fail closed. This proves the saved allow — not a permissive default —
    // is what lets the inside write through in the cross-flow above.
    let sandbox = Sandbox::new("headless-control");
    let inside = run_scripted_write(&sandbox, &[], "control.txt", "should-not-write");
    assert_write_denied(&inside);
    assert!(
        !sandbox.project.join("control.txt").exists(),
        "an un-saved inside write must fail closed in headless and leave no file"
    );
}
