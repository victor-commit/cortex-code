//! Black-box retry/backoff checks for `cortex run` using a local OpenAI-compatible fixture.
//!
//! These tests verify the shipped CLI path (not internal helpers): `cortex run --format json`
//! points at a loopback fixture through `CORTEX_OPENROUTER_BASE_URL`, retries retryable HTTP
//! failures, honors Retry-After headers, and eventually returns a successful assistant response.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
    fn retryable_error(status_line: &'static str, headers: Vec<(String, String)>) -> Self {
        Self {
            status_line,
            headers,
            body: r#"{"error":"transient upstream"}"#.to_owned(),
        }
    }

    fn success(answer: &str) -> Self {
        let body = sse_body(&[
            serde_json::json!({
                "id": "retry-fixture",
                "object": "chat.completion.chunk",
                "model": "google/gemini-2.5-flash",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant", "content": answer },
                    "finish_reason": null
                }]
            })
            .to_string(),
            serde_json::json!({
                "id": "retry-fixture",
                "object": "chat.completion.chunk",
                "model": "google/gemini-2.5-flash",
                "choices": [{
                    "index": 0,
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
        ]);
        Self {
            status_line: "HTTP/1.1 200 OK",
            headers: vec![
                ("content-type".to_owned(), "text/event-stream".to_owned()),
                ("cache-control".to_owned(), "no-cache".to_owned()),
                ("connection".to_owned(), "close".to_owned()),
            ],
            body,
        }
    }
}

struct Fixture {
    base_url: String,
    attempts: Arc<AtomicUsize>,
    accepted_at: Arc<Mutex<Vec<Instant>>>,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start(mut responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_thread = attempts.clone();
        let accepted_at = Arc::new(Mutex::new(Vec::new()));
        let accepted_at_thread = accepted_at.clone();
        let handle = thread::spawn(move || {
            let idle_deadline = Instant::now() + Duration::from_secs(12);
            while !responses.is_empty() && Instant::now() < idle_deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        attempts_thread.fetch_add(1, Ordering::Relaxed);
                        accepted_at_thread
                            .lock()
                            .expect("timestamps lock")
                            .push(Instant::now());
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
            attempts,
            accepted_at,
            handle,
        }
    }

    fn finish(self) -> (usize, Vec<Instant>) {
        let _ = self.handle.join();
        (
            self.attempts.load(Ordering::Relaxed),
            self.accepted_at.lock().expect("timestamps lock").clone(),
        )
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

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-retry-backoff-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn cortex_command(home: &Path, base_url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["run", "--format", "json", "Reply with exactly RECOVERED"])
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

fn cortex_jsonl_command(home: &Path, base_url: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(home)
        .args(["run", "--jsonl", "Reply with exactly RECOVERED"])
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

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
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
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

fn parse_jsonl(stdout: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("valid jsonl line"))
        .collect()
}

#[test]
fn retries_retryable_503_and_eventually_succeeds() {
    let fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::success("RECOVERED"),
    ]);
    let home = scratch_home();
    let out = run(cortex_command(&home, &fixture.base_url));
    let (attempts, accepted_at) = fixture.finish();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be strict JSON");
    assert_eq!(doc["role"], "assistant");
    assert_eq!(doc["content"], "RECOVERED");

    assert_eq!(attempts, 2, "expected one retry then success");
    assert_eq!(
        accepted_at.len(),
        2,
        "fixture should observe two HTTP attempts"
    );
    let delay = accepted_at[1].duration_since(accepted_at[0]);
    assert!(
        delay >= Duration::from_millis(300),
        "expected jittered backoff before retry, got {delay:?}"
    );
    assert!(
        delay <= Duration::from_secs(5),
        "retry delay should be bounded, got {delay:?}"
    );
}

#[test]
fn retry_after_headers_control_retry_delay() {
    let ms_fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error(
            "HTTP/1.1 429 Too Many Requests",
            vec![("retry-after-ms".to_owned(), "180".to_owned())],
        ),
        ScriptedResponse::success("RECOVERED"),
    ]);
    let ms_home = scratch_home();
    let ms_out = run(cortex_command(&ms_home, &ms_fixture.base_url));
    let (ms_attempts, ms_accepted_at) = ms_fixture.finish();

    assert_eq!(
        ms_out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&ms_out.stderr)
    );
    assert_eq!(ms_attempts, 2, "retry-after-ms case should retry once");
    assert_eq!(
        ms_accepted_at.len(),
        2,
        "retry-after-ms fixture should observe two HTTP attempts"
    );
    let ms_delay = ms_accepted_at[1].duration_since(ms_accepted_at[0]);
    assert!(
        ms_delay >= Duration::from_millis(140),
        "retry-after-ms delay should be honored, got {ms_delay:?}"
    );

    let sec_fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error(
            "HTTP/1.1 429 Too Many Requests",
            vec![("retry-after".to_owned(), "1".to_owned())],
        ),
        ScriptedResponse::success("RECOVERED"),
    ]);
    let sec_home = scratch_home();
    let sec_out = run(cortex_command(&sec_home, &sec_fixture.base_url));
    let (sec_attempts, sec_accepted_at) = sec_fixture.finish();

    assert_eq!(
        sec_out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&sec_out.stderr)
    );
    assert_eq!(
        sec_attempts, 2,
        "retry-after seconds case should retry once"
    );
    assert_eq!(
        sec_accepted_at.len(),
        2,
        "retry-after seconds fixture should observe two HTTP attempts"
    );
    let sec_delay = sec_accepted_at[1].duration_since(sec_accepted_at[0]);
    assert!(
        sec_delay >= Duration::from_millis(800),
        "retry-after seconds delay should be honored, got {sec_delay:?}"
    );
    assert!(
        sec_delay <= Duration::from_secs(4),
        "retry-after seconds delay should remain bounded, got {sec_delay:?}"
    );

    let http_date = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(2));
    let date_fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error(
            "HTTP/1.1 429 Too Many Requests",
            vec![("retry-after".to_owned(), http_date)],
        ),
        ScriptedResponse::success("RECOVERED"),
    ]);
    let date_home = scratch_home();
    let date_out = run(cortex_command(&date_home, &date_fixture.base_url));
    let (date_attempts, date_accepted_at) = date_fixture.finish();

    assert_eq!(
        date_out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&date_out.stderr)
    );
    assert_eq!(
        date_attempts, 2,
        "retry-after http-date case should retry once"
    );
    assert_eq!(
        date_accepted_at.len(),
        2,
        "retry-after http-date fixture should observe two HTTP attempts"
    );
    let date_delay = date_accepted_at[1].duration_since(date_accepted_at[0]);
    assert!(
        date_delay >= Duration::from_millis(700),
        "retry-after http-date delay should be honored, got {date_delay:?}"
    );
    assert!(
        date_delay <= Duration::from_secs(6),
        "retry-after http-date delay should remain bounded, got {date_delay:?}"
    );
}

#[test]
fn runner_retry_events_are_visible_in_jsonl_and_recover_the_turn() {
    let fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::success("RECOVERED"),
    ]);
    let home = scratch_home();
    let out = run(cortex_jsonl_command(&home, &fixture.base_url));
    let (attempts, _accepted_at) = fixture.finish();

    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        attempts, 4,
        "expected one exhausted transport attempt (3 HTTP tries) then one runner retry"
    );

    let lines = parse_jsonl(&out.stdout);
    assert!(!lines.is_empty(), "jsonl output must contain events");
    let retry = lines
        .iter()
        .find(|line| line.get("type") == Some(&serde_json::Value::String("retry".to_owned())))
        .expect("runner retry lifecycle event in jsonl");
    assert_eq!(retry["attempt"], 1);
    assert_eq!(retry["max_attempts"], 2);
    assert_eq!(retry["error_kind"], "upstream");
    assert_eq!(retry["classification"], "service_unavailable");
    assert_eq!(
        lines
            .last()
            .and_then(|line| line.get("type"))
            .and_then(|value| value.as_str()),
        Some("done"),
        "jsonl stream must terminate with done"
    );
}

#[test]
fn persistent_retryable_failures_are_bounded_and_report_retry_exhaustion() {
    let fixture = Fixture::start(vec![
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
        ScriptedResponse::retryable_error("HTTP/1.1 503 Service Unavailable", vec![]),
    ]);
    let home = scratch_home();
    let started = Instant::now();
    let out = run(cortex_jsonl_command(&home, &fixture.base_url));
    let elapsed = started.elapsed();
    let (attempts, _accepted_at) = fixture.finish();

    assert_ne!(
        out.status.code(),
        Some(0),
        "run should fail after bounded retries"
    );
    assert_eq!(
        attempts, 9,
        "runner retries must be bounded (3 stream attempts x 3 HTTP attempts each)"
    );
    assert!(
        elapsed < Duration::from_secs(12),
        "retry exhaustion should be bounded in time, got {elapsed:?}"
    );

    let lines = parse_jsonl(&out.stdout);
    let retry_count = lines
        .iter()
        .filter(|line| line.get("type") == Some(&serde_json::Value::String("retry".to_owned())))
        .count();
    assert_eq!(retry_count, 2, "runner retry event count should be bounded");

    let error = lines
        .iter()
        .find(|line| line.get("type") == Some(&serde_json::Value::String("error".to_owned())))
        .expect("terminal error event");
    let error_message = error["message"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        error_message.contains("retry exhausted"),
        "error event should report retry exhaustion, got: {}",
        error["message"]
    );
    assert_eq!(
        lines
            .last()
            .and_then(|line| line.get("type"))
            .and_then(|value| value.as_str()),
        Some("done"),
        "jsonl stream must terminate with done even on retry exhaustion"
    );

    let stderr = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
    assert!(
        stderr.contains("retry exhausted"),
        "stderr should include retry exhaustion diagnostics, got: {stderr}"
    );
}
