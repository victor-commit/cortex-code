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
        "cortex-gap-tools-compat-{tag}-{}-{}",
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

fn cortex_run_json(home: &Path, base_url: &str, model: &str, prompt: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .arg("run")
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
fn read_accepts_file_path_alias_and_apply_patch_accepts_patch_text_alias() {
    let home = scratch_home("aliases");
    fs::write(home.join("sample.txt"), "hello\n").unwrap();

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_read",
                "read",
                serde_json::json!({ "filePath": "sample.txt" }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("read done"), done_frame("stop")]),
    ]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "exercise read alias",
    ));
    fixture.finish();
    let read = tool_result_texts(&doc, "read").join("\n");
    assert!(read.contains("1: hello"), "read alias result: {read}");

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_patch",
                "apply_patch",
                serde_json::json!({
                    "patchText": "*** Begin Patch\n*** Add File: via-alias.txt\n+ok\n*** End Patch"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("patch done"), done_frame("stop")]),
    ]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "openai/gpt-4o-mini",
        "exercise apply_patch alias",
    ));
    fixture.finish();
    let patch = tool_result_texts(&doc, "apply_patch").join("\n");
    assert!(
        patch.contains("applied"),
        "apply_patch alias result: {patch}"
    );
    assert_eq!(
        fs::read_to_string(home.join("via-alias.txt")).unwrap(),
        "ok\n"
    );
}

#[test]
fn bash_workdir_scopes_single_command_and_glob_grep_default_caps_apply() {
    let home = scratch_home("limits");
    fs::create_dir_all(home.join("sub")).unwrap();
    for index in 0..150 {
        fs::write(home.join(format!("m{index:03}.txt")), "x\n").unwrap();
    }
    let hits: String = (1..=250).map(|i| format!("token {i}\n")).collect();
    fs::write(home.join("hits.txt"), hits).unwrap();

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_pwd_scoped",
                "bash",
                serde_json::json!({"command":"pwd","workdir":"sub"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_pwd_default",
                "bash",
                serde_json::json!({"command":"pwd"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame("call_glob", "glob", serde_json::json!({"pattern":"*.txt"})),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_grep",
                "grep",
                serde_json::json!({"pattern":"token","include":"hits.txt"}),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("done"), done_frame("stop")]),
    ]);
    let doc = assert_success_json(cortex_run_json(
        &home,
        &fixture.base_url,
        "google/gemini-2.5-flash",
        "exercise bash workdir and search limits",
    ));
    fixture.finish();

    let bash_results = tool_result_texts(&doc, "bash");
    assert_eq!(
        bash_results.len(),
        2,
        "expected two bash results: {bash_results:?}"
    );
    let nested = std::fs::canonicalize(home.join("sub")).unwrap();
    assert!(
        bash_results[0].contains(&nested.display().to_string()),
        "scoped pwd should run in nested workdir: {}",
        bash_results[0]
    );
    let root = std::fs::canonicalize(&home).unwrap();
    assert!(
        bash_results[1].contains(&root.display().to_string()),
        "default pwd should stay in session cwd: {}",
        bash_results[1]
    );

    let glob = tool_result_texts(&doc, "glob").join("\n");
    assert!(
        glob.contains("output truncated"),
        "glob default cap should truncate broad matches: {glob}"
    );

    let grep = tool_result_texts(&doc, "grep").join("\n");
    assert!(
        grep.contains("output truncated"),
        "grep default cap should truncate broad matches: {grep}"
    );
}
