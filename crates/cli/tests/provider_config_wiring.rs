//! Black-box checks for provider-configuration wiring (VAL-GAP-CONFIG-001).
//!
//! Evidence captured here:
//! - disabled providers are absent/refused;
//! - configured providers can be added and used by `cortex run` with a local fixture;
//! - machine output reports resolved provider config and redacts secrets.

use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(15);

struct Scratch {
    root: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-provider-config-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("home/project");
        fs::create_dir_all(&project).expect("create scratch project");
        Self { root, project }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    fn write_project_config(&self, body: &str) {
        fs::write(self.project.join("cortex.toml"), body).expect("write project config");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.current_dir(&self.project);
        command.env("HOME", self.home());
        command.args(args);
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
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
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn bind_loopback_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return String::new();
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
        let read = stream.read(&mut chunk).expect("read request body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    String::from_utf8_lossy(&buffer[header_end..]).to_string()
}

fn start_provider_fixture(
    model_id: &str,
    answer: &str,
) -> (String, Arc<Mutex<Vec<String>>>, std::thread::JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_thread = Arc::clone(&requests);
    let model_id = model_id.to_owned();
    let answer = answer.to_owned();

    let handle = std::thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let body = read_http_request_body(&mut stream);
                    requests_thread.lock().expect("request lock").push(body);
                    let chunk = serde_json::json!({
                        "id": "provider-config-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{
                            "index": 0,
                            "delta": { "content": answer },
                            "finish_reason": serde_json::Value::Null
                        }]
                    })
                    .to_string();
                    let done = serde_json::json!({
                        "id": "provider-config-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 4,
                            "completion_tokens": 2,
                            "total_tokens": 6
                        }
                    })
                    .to_string();
                    let body = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                    let _ = std::io::Write::flush(&mut stream);
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    (format!("http://127.0.0.1:{port}"), requests, handle)
}

fn synthetic_config_api_key(label: &str) -> String {
    format!("cfg-{}-provider-value", label.replace('_', "-"))
}

#[test]
fn disabled_provider_is_absent_and_refused() {
    let scratch = Scratch::new("disabled-refused");
    let local_api_key = synthetic_config_api_key("disabled");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "local/test-model"

[provider.local]
base_url = "http://127.0.0.1:3299"
api_key = "{local_api_key}"
models = [{{ id = "local/test-model", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#,
    ));

    let models = run(scratch.command(&["models", "--format", "json"]));
    assert_eq!(
        models.status.code(),
        Some(0),
        "models should still succeed: {}",
        String::from_utf8_lossy(&models.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&models.stdout).expect("models json");
    let providers = doc["providers"].as_array().expect("providers array");
    assert!(
        !providers
            .iter()
            .any(|provider| provider["id"] == serde_json::Value::String("openrouter".to_owned())),
        "openrouter must be filtered when disabled: {doc}"
    );

    let run_out = run(scratch.command(&["run", "--model", "openrouter/auto", "hello"]));
    assert_eq!(
        run_out.status.code(),
        Some(5),
        "disabled provider should fail as config error"
    );
    let stderr = String::from_utf8_lossy(&run_out.stderr);
    assert!(
        stderr.contains("provider `openrouter` is disabled"),
        "stderr should explain provider disable policy, got: {stderr}"
    );
}

#[test]
fn configured_provider_addition_and_override_drives_run() {
    let scratch = Scratch::new("configured-provider");
    let model_id = "local/test-model";
    let local_api_key = synthetic_config_api_key("configured");
    let (base_url, requests, fixture) = start_provider_fixture(model_id, "configured provider ok");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{model_id}"

[provider.local]
base_url = "{base_url}"
api_key = "{local_api_key}"
models = [{{ id = "{model_id}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]

[provider.local.options.params]
top_p = 0.3
"#
    ));

    let models = run(scratch.command(&["models", "--format", "json"]));
    assert_eq!(models.status.code(), Some(0));
    let models_doc: serde_json::Value =
        serde_json::from_slice(&models.stdout).expect("models json");
    assert!(
        models_doc["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .any(|provider| provider["id"] == "local"),
        "configured provider should appear in models output: {models_doc}"
    );

    let run_out = run(scratch.command(&["run", "--format", "json", "say hi"]));
    fixture.join().expect("join fixture");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "run should succeed against configured provider: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: serde_json::Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(run_doc["model"], model_id);
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("configured provider ok")),
        "fixture answer should be returned, got: {run_doc}"
    );

    let requests = requests.lock().expect("requests lock");
    assert!(
        requests
            .first()
            .is_some_and(|body| body.contains("\"top_p\":0.3")),
        "provider request params from config should be wired into request body, got: {requests:?}"
    );
}

#[test]
fn machine_output_reports_provider_config_with_redacted_secrets() {
    let scratch = Scratch::new("redacted-provider-config");
    let configured_api_key = synthetic_config_api_key("redacted");
    scratch.write_project_config(&format!(
        r#"
[provider]
model_overrides = {{ "google/gemini-2.5-flash" = "openrouter/auto" }}

[provider.openrouter]
api_key = "{configured_api_key}"
base_url = "https://openrouter.ai/api/v1"
"#,
    ));

    let output = run(scratch.command(&["models", "--format", "json"]));
    assert_eq!(
        output.status.code(),
        Some(0),
        "models should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        !stdout.contains(&configured_api_key),
        "raw secret must never appear in machine output"
    );
    let doc: serde_json::Value = serde_json::from_str(&stdout).expect("models json");
    assert_eq!(
        doc["provider_config"]["model_overrides"]["google/gemini-2.5-flash"],
        "openrouter/auto"
    );
    assert_eq!(
        doc["provider_config"]["providers"]["openrouter"]["auth"]["api_key"],
        "[REDACTED]"
    );
}
