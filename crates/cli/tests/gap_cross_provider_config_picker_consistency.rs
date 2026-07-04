//! VAL-GAP-CROSS-011: applying a provider-configuration change is reflected CONSISTENTLY across
//! every surface that selects a provider/model — the `cortex models` catalog, the loopback
//! `cortex serve` provider routes (`/config/providers` + `/provider`, the same routes the SDK
//! consumes), and actual run selection — and a disabled provider is refused everywhere.
//!
//! The applied config both ADDS a provider (`local`, a loopback OpenAI-compatible fixture) and
//! DISABLES the built-in `openrouter` provider (which serves every embedded model). After that
//! change:
//! - `cortex models --format json` advertises `local` and omits `openrouter`;
//! - `GET /config/providers` and `GET /provider` advertise the same available provider set with the
//!   same resolved `default_model`;
//! - the available-provider id set is IDENTICAL across CLI, `/config/providers`, and `/provider`;
//! - a real `cortex run` turn with the selected (default) model reaches the configured `local`
//!   provider (the fixture receives the request carrying the selected model id);
//! - selecting the disabled provider is refused both on the CLI (`cortex run --model openrouter/...`
//!   exits with the config error) and on the server (`POST /session/{id}/message` is rejected with a
//!   provider-disabled diagnostic, not a successful turn).
//!
//! The TUI model picker shares the exact same `providers::ProviderConfigSet` enable/disable +
//! merged-catalogue computation (it marks a disabled provider's models `[unavailable]` and makes
//! them unselectable), so its availability metadata agrees with the surfaces asserted here; the
//! live picker render is proven black-box with tuistory in the feature handoff.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(45);

struct Scratch {
    home: PathBuf,
    project: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-provider-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("home/project");
        std::fs::create_dir_all(&project).expect("create scratch project");
        Self {
            home: root.join("home"),
            project,
        }
    }

    fn write_config(&self, body: &str) {
        std::fs::write(self.project.join("cortex.toml"), body).expect("write project config");
    }

    /// A `cortex` invocation with a hermetic environment. `CORTEX_NO_NET` is left UNSET so the
    /// provider-disabled refusal (a pre-socket config error) is observable rather than masked by the
    /// network kill-switch; no asserted path here ever opens a real outbound socket.
    fn cortex(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.env_clear();
        command.current_dir(&self.project);
        command.env("HOME", &self.home);
        command.env("PATH", std::env::var("PATH").unwrap_or_default());
        command.args(args);
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.home.parent().unwrap_or(&self.home));
    }
}

fn synthetic_config_api_key(label: &str) -> String {
    format!("cfg-{}-provider-value", label.replace('_', "-"))
}

/// The project config that applies the provider change shared by every surface assertion.
fn provider_config_body(base_url: &str, api_key: &str, model_id: &str) -> String {
    format!(
        r#"
model = "{model_id}"

[provider]
disabled = ["openrouter"]

[provider.local]
base_url = "{base_url}"
api_key = "{api_key}"
models = [{{ id = "{model_id}", name = "Local Test Model", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]

[provider.local.options.params]
top_p = 0.3
"#
    )
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn run_json(scratch: &Scratch, args: &[&str]) -> Value {
    let out = run(scratch.cortex(args));
    assert_eq!(
        out.status.code(),
        Some(0),
        "`cortex {}` failed:\nstdout={}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "strict json from `cortex {}` failed: {err}\nstdout={}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
        )
    })
}

/// The sorted set of provider ids carried by a `{providers:[{id,..}]}` document.
fn provider_id_set(providers: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = providers
        .iter()
        .filter_map(|provider| provider["id"].as_str().map(ToOwned::to_owned))
        .collect();
    ids.sort();
    ids
}

// ---- Loopback OpenAI-compatible provider fixture -----------------------------------------------

/// A bound-but-not-yet-accepting fixture listener, so the port is known (and reserved) while the
/// CLI/server catalog assertions run, and the accept loop is started only just before the run turn.
struct Fixture {
    listener: TcpListener,
    port: u16,
}

fn bind_fixture() -> Fixture {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    Fixture { listener, port }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set fixture read timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => return String::from_utf8_lossy(&buffer).into_owned(),
            Ok(read) => read,
            Err(_) => return String::from_utf8_lossy(&buffer).into_owned(),
        };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
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
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                remaining = remaining.saturating_sub(read);
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buffer[header_end..]).into_owned()
}

/// Start the fixture accept loop: serve ONE streamed OpenAI-compatible answer and record the request
/// body so the test can prove the run reached the configured provider with the selected model.
fn serve_fixture(
    fixture: Fixture,
    model_id: &str,
    answer: &str,
) -> (Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = fixture.listener;
    listener
        .set_nonblocking(true)
        .expect("set fixture nonblocking");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&requests);
    let model_id = model_id.to_owned();
    let answer = answer.to_owned();
    let handle = thread::spawn(move || {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let body = read_http_request(&mut stream);
                    captured.lock().expect("fixture request lock").push(body);
                    let chunk = serde_json::json!({
                        "id": "cross-provider-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{"index": 0, "delta": {"content": answer}, "finish_reason": Value::Null}]
                    })
                    .to_string();
                    let done = serde_json::json!({
                        "id": "cross-provider-fixture",
                        "object": "chat.completion.chunk",
                        "model": model_id,
                        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
                    })
                    .to_string();
                    let payload = format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{}",
                        payload.len(),
                        payload
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (requests, handle)
}

// ---- Loopback server (cortex serve) helpers ----------------------------------------------------

fn wait_for_health(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if curl(port, "GET", "/health", None).is_some_and(|(status, _)| status == 200) {
            return;
        }
        thread::sleep(Duration::from_millis(150));
    }
    panic!("server on port {port} did not become healthy");
}

/// Minimal loopback HTTP client returning `(status, body)` or `None` if the request failed.
fn curl(port: u16, method: &str, path: &str, json_body: Option<&str>) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .ok()?;
    let body = json_body.unwrap_or("");
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if json_body.is_some() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8_lossy(&response);
    let (head, payload) = text.split_once("\r\n\r\n")?;
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()?;
    Some((status, payload.to_owned()))
}

fn curl_ok_json(port: u16, path: &str) -> Value {
    let (status, body) = curl(port, "GET", path, None).expect("server request");
    assert_eq!(status, 200, "GET {path} should return 200, body={body}");
    serde_json::from_str(&body).unwrap_or_else(|err| panic!("json from {path}: {err}\nbody={body}"))
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u32>().ok())
}

fn stop_server(mut child: Child, port: u16) {
    let pid = listener_pid(port).unwrap_or_else(|| child.id());
    let _ = Command::new("kill").arg(pid.to_string()).status();
    let _ = child.wait();
}

fn free_server_port(avoid: u16) -> u16 {
    for port in 3260..=3299 {
        if port != avoid && TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 3260-3299");
}

#[test]
fn provider_config_change_is_consistent_across_cli_server_and_run_selection() {
    let scratch = Scratch::new("consistency");
    let model_id = "local/test-model";
    let api_key = synthetic_config_api_key("consistency");
    let fixture = bind_fixture();
    let fixture_base_url = format!("http://127.0.0.1:{}", fixture.port);
    scratch.write_config(&provider_config_body(&fixture_base_url, &api_key, model_id));

    // ---- Surface 1: `cortex models --format json` ----
    let models = run_json(&scratch, &["models", "--format", "json"]);
    assert_eq!(
        models["default_model"], model_id,
        "CLI default model should resolve to the configured provider's model: {models}"
    );
    let cli_providers = models["providers"].as_array().expect("cli providers array");
    let cli_ids = provider_id_set(cli_providers);
    assert!(
        cli_ids.iter().any(|id| id == "local"),
        "the added provider must appear in `cortex models`: {cli_ids:?}"
    );
    assert!(
        !cli_ids.iter().any(|id| id == "openrouter"),
        "the disabled provider must be absent from `cortex models`: {cli_ids:?}"
    );
    // The raw secret never appears in machine output.
    assert!(
        !String::from_utf8_lossy(&run(scratch.cortex(&["models", "--format", "json"])).stdout)
            .contains(&api_key),
        "the configured api key must be redacted in `cortex models`"
    );

    // ---- Surface 2 + 3: loopback `cortex serve` provider routes ----
    let server_port = free_server_port(fixture.port);
    let mut serve = scratch.cortex(&[
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &server_port.to_string(),
    ]);
    serve.stdout(Stdio::piped()).stderr(Stdio::piped());
    let server = serve.spawn().expect("spawn cortex serve");
    wait_for_health(server_port);

    // GET /config/providers mirrors `cortex models`.
    let config_providers = curl_ok_json(server_port, "/config/providers");
    assert_eq!(
        config_providers["default_model"], model_id,
        "server /config/providers default model should match the CLI: {config_providers}"
    );
    let server_catalog_ids = provider_id_set(
        config_providers["providers"]
            .as_array()
            .expect("server providers"),
    );

    // GET /provider lists providers with authenticated state.
    let provider_state = curl_ok_json(server_port, "/provider");
    assert_eq!(
        provider_state["default_model"], model_id,
        "server /provider default model should match the CLI: {provider_state}"
    );
    let server_state_ids = provider_id_set(
        provider_state["providers"]
            .as_array()
            .expect("provider state"),
    );

    // The available provider id set is IDENTICAL across CLI, /config/providers, and /provider —
    // this is the same shape the SDK (`client.config.providers()` / `client.provider.list()`)
    // consumes, asserted directly in the SDK suite.
    assert_eq!(
        cli_ids, server_catalog_ids,
        "CLI and server /config/providers must advertise the same provider set"
    );
    assert_eq!(
        cli_ids, server_state_ids,
        "CLI and server /provider must advertise the same provider set"
    );
    assert!(
        !server_state_ids.iter().any(|id| id == "openrouter"),
        "the disabled provider must be absent from /provider: {server_state_ids:?}"
    );

    // ---- Disabled provider is refused on the server ----
    let (create_status, create_body) =
        curl(server_port, "POST", "/session", Some("{}")).expect("create session");
    assert_eq!(
        create_status, 201,
        "session creation should succeed: {create_body}"
    );
    let session: Value = serde_json::from_str(&create_body).expect("session json");
    let session_id = session["id"].as_str().expect("session id").to_owned();

    let (refusal_status, refusal_body) = curl(
        server_port,
        "POST",
        &format!("/session/{session_id}/message"),
        Some(r#"{"model":"openrouter/auto","text":"hello"}"#),
    )
    .expect("disabled-provider prompt");
    assert_ne!(
        refusal_status, 200,
        "selecting the disabled provider must NOT succeed on the server: {refusal_body}"
    );
    assert!(
        refusal_body.contains("disabled"),
        "server refusal should explain the provider is disabled, got: {refusal_body}"
    );

    stop_server(server, server_port);
    assert!(
        TcpListener::bind(("127.0.0.1", server_port)).is_ok(),
        "server port {server_port} should be released after stopping the listener"
    );

    // ---- Surface 4: actual run selection reaches the configured provider ----
    let (requests, fixture_handle) =
        serve_fixture(fixture, model_id, "configured local provider ok");
    let run_out = run(scratch.cortex(&["run", "--format", "json", "say hi"]));
    fixture_handle.join().expect("join fixture");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "run with the selected model should succeed against the configured provider: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(
        run_doc["model"], model_id,
        "the run selected the configured provider's model: {run_doc}"
    );
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("configured local provider ok")),
        "the configured provider's answer should be returned: {run_doc}"
    );
    let captured = requests.lock().expect("fixture requests lock");
    let request_body = captured
        .first()
        .expect("the fixture received the run request");
    assert!(
        request_body.contains(model_id),
        "the request that reached the configured provider carried the selected model id: {request_body}"
    );

    // ---- Disabled provider is refused on the CLI ----
    let refused = run(scratch.cortex(&["run", "--model", "openrouter/auto", "hello"]));
    assert_eq!(
        refused.status.code(),
        Some(5),
        "selecting the disabled provider on the CLI must fail as a config error"
    );
    let refused_stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refused_stderr.contains("provider `openrouter` is disabled"),
        "the CLI refusal should explain the provider is disabled, got: {refused_stderr}"
    );
}

/// Live proof (gated on `OPENROUTER_API_KEY`): with the default configuration (openrouter enabled),
/// a real `cortex run` turn selects the configured OpenRouter provider and returns a real answer,
/// proving run selection reaches the live configured provider, not just the local fixture.
#[test]
fn live_default_run_selects_configured_openrouter_provider() {
    let Ok(key) = std::env::var("OPENROUTER_API_KEY") else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if key.trim().is_empty() {
        eprintln!("skipping live test: OPENROUTER_API_KEY empty");
        return;
    }

    let scratch = Scratch::new("live-default");
    // No provider config change: the default openrouter provider stays enabled and selected.
    let mut command = scratch.cortex(&[
        "run",
        "--format",
        "json",
        "--model",
        "google/gemini-2.5-flash",
        "Reply with the single word: pong",
    ]);
    command.env("OPENROUTER_API_KEY", key);
    let out = run(command);
    assert_eq!(
        out.status.code(),
        Some(0),
        "live default run should succeed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let doc: Value = serde_json::from_slice(&out.stdout).expect("live run json");
    assert_eq!(
        doc["model"], "google/gemini-2.5-flash",
        "the live run selected the configured OpenRouter model: {doc}"
    );
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|c| !c.trim().is_empty()),
        "the live OpenRouter turn returned a real answer: {doc}"
    );
}
