//! Live black-box proof for the built-in `webfetch` tool through `cortex run`.
//!
//! The test is gated on `OPENROUTER_API_KEY`: without a key it skips cleanly, while mission
//! validation runs it with the real key. The fetched page is served by a real loopback HTTP server
//! in the reserved 3200-3299 range, and the model must call the real built-in tool to read the
//! page-specific marker.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_LOOPBACK_PORT: AtomicU16 = AtomicU16::new(3220);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn isolated_home() -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "cortex-webfetch-live-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".config")).expect("create config dir");
    std::fs::create_dir_all(root.join(".local/share")).expect("create data dir");
    std::fs::create_dir_all(root.join("project")).expect("create project dir");
    root
}

fn bind_loopback() -> (TcpListener, u16) {
    for _ in 0..80 {
        let port = NEXT_LOOPBACK_PORT.fetch_add(1, Ordering::Relaxed);
        if port > 3299 {
            NEXT_LOOPBACK_PORT.store(3220, Ordering::Relaxed);
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            listener
                .set_nonblocking(true)
                .expect("set listener nonblocking");
            return (listener, port);
        }
    }
    panic!("no available loopback port in the reserved Cortex range 3200-3299");
}

fn serve_marker_page(marker: String) -> (String, thread::JoinHandle<Option<String>>) {
    let (listener, port) = bind_loopback();
    let handle = thread::spawn(move || {
        let start = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    let body = format!(
                        "<!doctype html><html><head><title>webfetch live</title></head>\
                         <body><h1>Webfetch Live Fixture</h1>\
                         <p>The validation marker is <strong>{marker}</strong>.</p>\
                         <p><a href=\"/source\">fixture source</a></p>\
                         <script>console.log('do-not-include')</script></body></html>"
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                    return Some(request);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() > Duration::from_secs(90) {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(err) => panic!("accept failed: {err}"),
            }
        }
    });
    (format!("http://127.0.0.1:{port}/fixture"), handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set request timeout");
    let mut request = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let read = stream.read(&mut buf).expect("read request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buf[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn cortex(home: &std::path::Path, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(home.join("project"));
    command.args(args);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("XDG_CONFIG_HOME", home.join(".config"));
    command.env("XDG_DATA_HOME", home.join(".local/share"));
    command.env("CORTEX_DATA_DIR", home.join(".local/share/cortex"));
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
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
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn unique_marker() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    format!("WEBFETCH-LIVE-{nanos}")
}

#[test]
fn real_openrouter_turn_uses_webfetch_content_and_sends_no_credentials() {
    let Some(key) = live_key() else {
        eprintln!("skipping webfetch live proof: OPENROUTER_API_KEY is not set");
        return;
    };

    let marker = unique_marker();
    let (url, server) = serve_marker_page(marker.clone());
    let home = isolated_home();
    let prompt = format!(
        "You must call the webfetch tool exactly once for this URL: {url}\n\
         After reading the fetched page, reply with exactly the validation marker and nothing else."
    );

    let output = run(
        cortex(&home, &key, &["run", "--format", "json", &prompt]),
        Duration::from_secs(120),
    );
    let request = server
        .join()
        .expect("server thread")
        .expect("webfetch request was received");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run JSON should parse");
    let answer = doc["content"].as_str().expect("content string");
    assert!(
        answer.contains(&marker),
        "assistant answer must be grounded in fetched content: {doc}"
    );
    let tool_results = doc["tool_results"].as_array().expect("tool results array");
    assert!(
        tool_results
            .iter()
            .any(|result| result["name"] == "webfetch" && result["is_error"] == false),
        "webfetch tool call must be recorded: {doc}"
    );
    let rendered_tool_text = serde_json::to_string(tool_results).expect("tool JSON");
    assert!(rendered_tool_text.contains(&marker), "{rendered_tool_text}");
    assert!(
        rendered_tool_text.contains("Webfetch Live Fixture"),
        "{rendered_tool_text}"
    );
    assert!(
        !rendered_tool_text.contains("<script>") && !rendered_tool_text.contains("console.log"),
        "tool output should not contain script payloads: {rendered_tool_text}"
    );

    let lower_request = request.to_ascii_lowercase();
    assert!(
        !lower_request.contains("authorization:"),
        "webfetch request must not send Authorization headers:\n{request}"
    );
    assert!(
        !request.contains(&key),
        "webfetch request must never contain the provider key"
    );
}
