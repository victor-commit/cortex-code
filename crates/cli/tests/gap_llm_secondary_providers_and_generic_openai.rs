use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    headers: String,
    body: String,
}

struct Scratch {
    root: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-llm-secondary-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("home/project");
        let data_dir = root.join("home/.local/share/cortex");
        fs::create_dir_all(&project).expect("create scratch project");
        fs::create_dir_all(&data_dir).expect("create scratch data dir");
        Self {
            root,
            project,
            data_dir,
        }
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
        command.env("CORTEX_DATA_DIR", &self.data_dir);
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

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return CapturedRequest {
                request_line: String::new(),
                headers: String::new(),
                body: String::new(),
            };
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let request_line = headers.lines().next().unwrap_or_default().to_owned();
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
    CapturedRequest {
        request_line,
        headers,
        body: String::from_utf8_lossy(&buffer[header_end..]).to_string(),
    }
}

fn start_chat_fixture(
    model_id: &str,
    answer: &str,
) -> (
    String,
    Arc<Mutex<Vec<CapturedRequest>>>,
    std::thread::JoinHandle<()>,
) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let captures = Arc::new(Mutex::new(Vec::new()));
    let captures_thread = Arc::clone(&captures);
    let model_id = model_id.to_owned();
    let answer = answer.to_owned();

    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    captures_thread.lock().expect("captures lock").push(request);
                    let chunk = serde_json::json!({
                        "id": "gap-llm-secondary-fixture",
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
                        "id": "gap-llm-secondary-fixture",
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
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    (format!("http://127.0.0.1:{port}"), captures, handle)
}

fn serve_json_once(body: &'static str) -> (String, std::thread::JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("fixture addr").port();
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut req = [0_u8; 4096];
                    let _ = stream.read(&mut req);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (format!("http://127.0.0.1:{port}/api.json"), handle)
}

fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

fn synthetic_config_api_key(label: &str) -> String {
    format!("cfg-{}-provider-value", label.replace('_', "-"))
}

#[test]
fn generic_openai_provider_routes_without_openrouter_and_honors_headers() {
    let scratch = Scratch::new("generic-openai");
    let model_id = "local/test-model";
    let api_key = synthetic_config_api_key("generic-openai");
    let (base_url, captures, handle) = start_chat_fixture(model_id, "generic provider ok");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{model_id}"

[provider.local]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "openai-chat"
models = [{{ id = "{model_id}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]

[provider.local.options.headers]
"x-provider-route" = "local"
"#,
    ));

    let out = run(scratch.command(&["run", "--format", "json", "say hi"]));
    handle.join().expect("join fixture");
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert_eq!(doc["model"], model_id);
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("generic provider ok")),
        "expected fixture content, got: {doc}"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("openrouter"),
        "machine output for this turn should not reference OpenRouter"
    );

    let captures = captures.lock().expect("captures lock");
    let first = captures.first().expect("captured request");
    assert!(
        first.request_line.contains("POST /chat/completions"),
        "request should route to chat completions: {}",
        first.request_line
    );
    assert!(
        first
            .headers
            .to_ascii_lowercase()
            .contains("x-provider-route: local"),
        "configured provider header must be forwarded, got: {}",
        first.headers
    );
    assert!(
        first.body.contains("\"model\":\"local/test-model\""),
        "request should keep the configured local model id, got: {}",
        first.body
    );
}

#[test]
fn generic_openai_provider_uses_only_loopback_connects() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }

    let scratch = Scratch::new("generic-openai-strace");
    let model_id = "local/test-model";
    let api_key = synthetic_config_api_key("generic-openai-strace");
    let (base_url, _captures, handle) = start_chat_fixture(model_id, "ok");
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{model_id}"

[provider.local]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "openai-chat"
models = [{{ id = "{model_id}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#,
    ));

    let mut command = Command::new("strace");
    command
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "run",
            "--format",
            "json",
            "say hi",
        ])
        .current_dir(&scratch.project)
        .env("HOME", scratch.home())
        .env("CORTEX_DATA_DIR", &scratch.data_dir)
        .env_remove("OPENROUTER_API_KEY");
    let output = run(command);
    handle.join().expect("join fixture");
    assert_eq!(
        output.status.code(),
        Some(0),
        "straced run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace = String::from_utf8_lossy(&output.stderr);
    let inet_connects: Vec<&str> = trace
        .lines()
        .filter(|line| {
            line.contains("connect(") && (line.contains("AF_INET") || line.contains("AF_INET6"))
        })
        .collect();
    assert!(
        !inet_connects.is_empty(),
        "expected at least one inet connect in trace: {trace}"
    );
    assert!(
        inet_connects
            .iter()
            .all(|line| line.contains("127.0.0.1") || line.contains("::1")),
        "all inet connects must stay on loopback: {inet_connects:?}"
    );
}

#[test]
fn metacharacter_parent_instruction_glob_never_reaches_the_provider_request() {
    let scratch = Scratch::new("instruction-glob-metachar-parent");
    let model_id = "local/test-model";
    let api_key = synthetic_config_api_key("instruction-glob-metachar-parent");
    let outside = scratch
        .project
        .parent()
        .expect("project parent")
        .join("outside.md");
    let sentinel = "OUTSIDE-GLOB-SENTINEL";
    fs::write(&outside, sentinel).expect("write outside sentinel");
    let (base_url, captures, handle) = start_chat_fixture(model_id, "contained response");
    scratch.write_project_config(&format!(
        r#"
instructions = [".[.]/outside.md"]

[provider]
enabled = ["local"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{model_id}"

[provider.local]
base_url = "{base_url}"
api_key = "{api_key}"
protocol = "openai-chat"
models = [{{ id = "{model_id}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#,
    ));

    let out = run(scratch.command(&["run", "--format", "json", "say hi"]));
    handle.join().expect("join fixture");
    assert_eq!(
        out.status.code(),
        Some(0),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let outside_path = outside.display().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ignoring local instructions glob `.[.]/outside.md`"),
        "unsafe pattern must be rejected before expansion: {stderr}"
    );
    assert!(
        !stderr.contains(&outside_path),
        "external match path must not be reported: {stderr}"
    );

    let captures = captures.lock().expect("captures lock");
    let request = captures.first().expect("provider request");
    assert!(
        !request.body.contains(sentinel),
        "external instruction content must never reach the provider: {}",
        request.body
    );
    assert!(
        !request.body.contains(&outside_path),
        "external path must never reach the provider: {}",
        request.body
    );
}

#[test]
fn models_refresh_lists_secondary_provider_namespaces_and_caches_offline() {
    const FIXTURE: &str = r#"{
      "openrouter": {
        "id": "openrouter",
        "name": "OpenRouter",
        "api": "https://openrouter.ai/api/v1",
        "env": ["OPENROUTER_API_KEY"],
        "models": {
          "google/gemini-2.5-flash": { "id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true }
        }
      },
      "amazon-bedrock": {
        "id": "amazon-bedrock",
        "name": "Amazon Bedrock",
        "models": {
          "amazon.nova-2-lite-v1:0": { "id": "amazon.nova-2-lite-v1:0", "name": "Nova 2 Lite", "tool_call": true }
        }
      },
      "azure": {
        "id": "azure",
        "name": "Azure OpenAI",
        "models": {
          "gpt-4.1-mini": { "id": "gpt-4.1-mini", "name": "GPT-4.1 mini", "tool_call": true }
        }
      },
      "google-vertex": {
        "id": "google-vertex",
        "name": "Google Vertex",
        "models": {
          "gemini-2.5-flash": { "id": "gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true }
        }
      },
      "github-copilot": {
        "id": "github-copilot",
        "name": "GitHub Copilot",
        "models": {
          "claude-fable-5": { "id": "claude-fable-5", "name": "Claude Fable 5", "tool_call": true }
        }
      },
      "xai": {
        "id": "xai",
        "name": "xAI",
        "models": {
          "grok-4.20-0309-non-reasoning": { "id": "grok-4.20-0309-non-reasoning", "name": "xAI model", "tool_call": true }
        }
      }
    }"#;

    let scratch = Scratch::new("models-refresh-secondary");
    let (url, handle) = serve_json_once(FIXTURE);

    let mut refresh = scratch.command(&["models", "--refresh", "--format", "json"]);
    refresh.env("CORTEX_MODELS_DEV_URL", &url);
    let refreshed = run(refresh);
    handle.join().expect("join fixture");
    assert_eq!(
        refreshed.status.code(),
        Some(0),
        "refresh should succeed: {}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&refreshed.stdout).expect("refresh json");
    let provider_ids: Vec<&str> = doc["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .filter_map(|provider| provider["id"].as_str())
        .collect();
    for required in ["openrouter", "bedrock", "azure", "vertex", "copilot", "xai"] {
        assert!(
            provider_ids.contains(&required),
            "refreshed providers should include {required}, got: {provider_ids:?}"
        );
    }

    let mut offline = scratch.command(&["models", "--format", "json"]);
    offline.env("CORTEX_NO_NET", "1");
    let offline_out = run(offline);
    assert_eq!(
        offline_out.status.code(),
        Some(0),
        "offline listing should succeed from cache: {}",
        String::from_utf8_lossy(&offline_out.stderr)
    );
    let offline_doc: serde_json::Value =
        serde_json::from_slice(&offline_out.stdout).expect("offline json");
    let offline_ids: Vec<&str> = offline_doc["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .filter_map(|provider| provider["id"].as_str())
        .collect();
    assert!(
        offline_ids.contains(&"bedrock") && offline_ids.contains(&"vertex"),
        "offline cached listing should preserve refreshed secondary providers: {offline_ids:?}"
    );
}
