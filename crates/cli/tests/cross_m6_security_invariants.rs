//! Cross-M6 security invariant coverage for headless machine output.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
static NEXT_PORT: AtomicU16 = AtomicU16::new(3260);

#[test]
fn headless_json_redacts_secret_and_audit_is_surface_attributed() {
    let scratch = Scratch::new("headless");
    let secret = fake_secret();
    let provider = SecretAnswerProvider::spawn(secret.clone());

    let out = run(cortex(
        &scratch,
        &provider,
        &["run", "--format", "json", "return the secret"],
    ));
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains(&secret), "headless JSON leaked: {stdout}");
    assert!(
        stdout.contains(redaction::PLACEHOLDER),
        "headless JSON should carry placeholder: {stdout}"
    );

    let records = read_audit(&scratch.audit);
    assert!(records.iter().any(|record| {
        record["type"] == "network_call"
            && record["surface"] == "run"
            && record["source"] == "headless"
            && record["kind"] == "provider_request"
    }));
}

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    audit: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-m6-security-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            db: root.join("data/cortex.db"),
            audit: root.join("audit.jsonl"),
            root,
            home,
            project,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct SecretAnswerProvider {
    base_url: String,
    thread: Option<thread::JoinHandle<()>>,
}

impl SecretAnswerProvider {
    fn spawn(secret: String) -> Self {
        let (port, listener) = bind_provider_listener();
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let thread = thread::spawn(move || {
            let mut stream = accept_with_timeout(&listener);
            let _ = read_http_body(&mut stream);
            let body = sse_body(&[
                serde_json::json!({
                    "choices": [{
                        "delta": { "content": format!("secret: {secret}") },
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
            ]);
            write_sse_response(&mut stream, &body).expect("write provider response");
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            thread: Some(thread),
        }
    }
}

impl Drop for SecretAnswerProvider {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn cortex(scratch: &Scratch, provider: &SecretAnswerProvider, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.args(args);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("CORTEX_AUDIT_LOG", &scratch.audit);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
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
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
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
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run timed out");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout_thread.join().expect("stdout join"),
        stderr: stderr_thread.join().expect("stderr join"),
    }
}

fn read_audit(path: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).expect("audit file");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("audit JSONL record"))
        .collect()
}

fn accept_with_timeout(listener: &TcpListener) -> TcpStream {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    start.elapsed() <= RUN_TIMEOUT,
                    "provider fixture timed out waiting for request"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("accept provider request: {err}"),
        }
    }
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

fn write_sse_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return buffer;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&buffer) {
            break end;
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
        let read = stream.read(&mut chunk).expect("read body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    buffer
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn bind_provider_listener() -> (u16, TcpListener) {
    for _ in 0..40 {
        let port = NEXT_PORT.fetch_add(1, Ordering::SeqCst);
        if port > 3299 {
            NEXT_PORT.store(3260, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if let Ok(listener) = TcpListener::bind(addr) {
            return (port, listener);
        }
    }
    panic!("no free port in 3260-3299");
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}

fn fake_secret() -> String {
    format!("sk-{}-{}", "or-v1", "badc0ffe".repeat(8))
}
