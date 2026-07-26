//! E2E: catalogue-only provider (no built-in profile, no config base_url/api_key)
//! resolves base URL from catalogue `api` and credential env from catalogue `env`,
//! then completes a `cortex run` turn against a loopback OpenAI-compatible fixture.
//!
//! Provider under test: `sambanova` (catalogue-only; not in W2 profile list).
//! Wave-1 unit coverage lives in `providers` `catalogue_runtime_fallback`; this file
//! proves the headless CLI path end-to-end.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(25);
const PROVIDER_ID: &str = "sambanova";
const ENV_NAME: &str = "SAMBANOVA_API_KEY";
/// Synthetic fixture key — never a live credential; must not appear on stdout/stderr.
const FIXTURE_API_KEY: &str = "sambanova-catalogue-e2e-fixture-key";
const MODEL_ID: &str = "sambanova/catalogue-fixture-model";
const FIXTURE_ANSWER: &str = "catalogue only provider ok";

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
            "cortex-gap-llm-catalogue-only-{tag}-{}-{}",
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
        // env_clear already drops ambient XDG; keep intentional so mission gate
        // wrappers cannot inject host model prefs that fight provider policy.
        command.env_remove("XDG_CONFIG_HOME");
        command.env_remove("XDG_DATA_HOME");
        command.env_remove("OPENROUTER_API_KEY");
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

/// Prefer the plan's reserved e2e band 3200–3299; fall back to ephemeral if busy.
fn bind_loopback_listener() -> TcpListener {
    for port in 3200..3300 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
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
                        "id": "gap-llm-catalogue-only-fixture",
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
                        "id": "gap-llm-catalogue-only-fixture",
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

fn serve_json_once(body: String) -> (String, std::thread::JoinHandle<()>) {
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

fn catalogue_fixture_json(base_url: &str) -> String {
    // models.dev-shaped snapshot: catalogue-only sambanova with api + env, no profile needed.
    format!(
        r#"{{
  "sambanova": {{
    "id": "sambanova",
    "name": "SambaNova",
    "api": "{base_url}",
    "env": ["SAMBANOVA_API_KEY"],
    "models": {{
      "{MODEL_ID}": {{
        "id": "{MODEL_ID}",
        "name": "Catalogue Fixture",
        "tool_call": true,
        "cost": {{ "input": 0.0, "output": 0.0 }},
        "limit": {{ "context": 8192, "output": 1024 }}
      }}
    }}
  }}
}}"#
    )
}

/// Catalogue-only provider: config supplies neither `base_url` nor `api_key`/`api_key_env`.
/// Base URL must come from catalogue `api`; credential env name from catalogue `env`.
#[test]
fn catalogue_only_provider_runs_via_env_key_and_catalogue_api() {
    let scratch = Scratch::new("run");
    let (base_url, captures, chat_handle) = start_chat_fixture(MODEL_ID, FIXTURE_ANSWER);
    let catalogue_json = catalogue_fixture_json(&base_url);

    // Seed on-disk cache (offline fallback) and serve the same snapshot for the online refresh path.
    fs::write(scratch.data_dir.join("models.json"), &catalogue_json)
        .expect("seed models.json cache");
    let (models_url, models_handle) = serve_json_once(catalogue_json);

    // Minimal provider table so `sambanova/<model>` / model_overrides can select the provider.
    // Intentionally omit base_url, api_key, and api_key_env — those must come from the catalogue.
    scratch.write_project_config(&format!(
        r#"
[provider]
enabled = ["{PROVIDER_ID}"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "{MODEL_ID}"

[provider.{PROVIDER_ID}]
models = [{{ id = "{MODEL_ID}", name = "Catalogue Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
    ));

    let mut command = scratch.command(&["run", "--format", "json", "say hi"]);
    command.env(ENV_NAME, FIXTURE_API_KEY);
    command.env("CORTEX_MODELS_DEV_URL", &models_url);

    let out = run(command);
    let _ = models_handle.join();
    let _ = chat_handle.join();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Secret hygiene: fixture key must never leak to either stream.
    assert!(
        !stdout.contains(FIXTURE_API_KEY) && !stderr.contains(FIXTURE_API_KEY),
        "API key must not appear on stdout/stderr\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // Document current failure mode for task 8 if the catalogue-aware path is not wired end-to-end.
    if out.status.code() != Some(0) {
        let combined = format!("{stdout}\n{stderr}");
        let looks_like_missing_base = combined.contains("no `base_url`")
            || combined.contains("missing_base_url")
            || combined.contains("has no `base_url`");
        let looks_like_missing_key = combined.contains("API key")
            || combined.contains("api key")
            || combined.contains("credential")
            || combined.contains(ENV_NAME);
        panic!(
            "catalogue-only cortex run RED (task 8 may still be needed).\n\
             exit={:?}\n\
             missing_base_url_signal={looks_like_missing_base}\n\
             missing_credential_signal={looks_like_missing_key}\n\
             stdout:\n{stdout}\n\
             stderr:\n{stderr}",
            out.status.code()
        );
    }

    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("run json");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains(FIXTURE_ANSWER)),
        "expected fixture assistant text, got: {doc}"
    );
    assert!(
        !stdout.to_ascii_lowercase().contains("openrouter"),
        "catalogue-only turn must not route via OpenRouter: {stdout}"
    );

    let captures = captures.lock().expect("captures lock");
    let first = captures
        .first()
        .expect("fixture should have received a chat completion request");
    assert!(
        first.request_line.contains("POST /chat/completions"),
        "request should hit OpenAI-compatible chat completions: {}",
        first.request_line
    );
    assert!(
        first.body.contains(&format!("\"model\":\"{MODEL_ID}\"")),
        "request should keep the configured catalogue model id"
    );
    assert!(
        first
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer "),
        "request should forward the env API key as Bearer auth (value redacted from assertion)"
    );
    // Confirm the Authorization header carries our key without printing it in panic messages.
    let auth_ok = first
        .headers
        .lines()
        .any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("authorization:") && line.contains(FIXTURE_API_KEY)
        });
    assert!(
        auth_ok,
        "Authorization header must use the catalogue env key (header names only on failure): {}",
        first
            .headers
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("authorization"))
            .map(|line| {
                // Redact the token value in diagnostics.
                if let Some((name, _)) = line.split_once(':') {
                    format!("{name}: [REDACTED]")
                } else {
                    "[REDACTED]".to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    );
}
