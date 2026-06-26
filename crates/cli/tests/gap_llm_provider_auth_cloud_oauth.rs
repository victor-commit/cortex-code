use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const AUTH_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

struct Sandbox {
    root: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-provider-auth-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&project).expect("create project");
        fs::create_dir_all(root.join(".config")).expect("create config");
        fs::create_dir_all(root.join(".local/share")).expect("create xdg");
        Self {
            root,
            project,
            data_dir,
        }
    }

    fn home(&self) -> PathBuf {
        self.root.clone()
    }

    fn auth_file(&self) -> PathBuf {
        self.data_dir.join("auth.json")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command.current_dir(&self.project);
        command.args(args);
        command.env("HOME", self.home());
        command.env("XDG_CONFIG_HOME", self.home().join(".config"));
        command.env("XDG_DATA_HOME", self.home().join(".local/share"));
        command.env("CORTEX_DATA_DIR", &self.data_dir);
        command
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("GOOGLE_API_KEY")
            .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
            .env_remove("AWS_BEARER_TOKEN_BEDROCK")
            .env_remove("AWS_ACCESS_KEY_ID")
            .env_remove("AWS_SECRET_ACCESS_KEY")
            .env_remove("AWS_SESSION_TOKEN")
            .env_remove("AWS_PROFILE")
            .env_remove("AWS_DEFAULT_PROFILE")
            .env_remove("AWS_SHARED_CREDENTIALS_FILE")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET");
        command
    }
}

impl Drop for Sandbox {
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
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
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

fn run_auth_with_browser(mut command: Command) -> (Output, String) {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn auth login");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");

    let (tx, rx) = mpsc::channel::<String>();
    let stderr_thread = std::thread::spawn(move || {
        let mut collected = String::new();
        let mut reader = BufReader::new(stderr);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Some(url) = line
                        .split("open this URL to authorize:")
                        .nth(1)
                        .map(str::trim)
                        .filter(|value| value.starts_with("http://"))
                    {
                        let _ = tx.send(url.to_owned());
                    }
                    collected.push_str(&line);
                }
                Err(err) => panic!("read stderr failed: {err}"),
            }
        }
        collected.into_bytes()
    });
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stdout = stdout;
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });

    let authorize_url = match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(url) => url,
        Err(_) => {
            let status = wait_child(&mut child, Duration::from_secs(5));
            let stdout = stdout_thread.join().expect("join stdout");
            let stderr = stderr_thread.join().expect("join stderr");
            panic!(
                "auth login should print authorize URL (status={status:?})\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
    };
    let callback_body = http_get_follow_once(&authorize_url);
    let status = wait_child(&mut child, AUTH_TIMEOUT);
    let stdout = stdout_thread.join().expect("join stdout");
    let stderr = stderr_thread.join().expect("join stderr");

    (
        Output {
            status,
            stdout,
            stderr,
        },
        callback_body,
    )
}

fn wait_child(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 3200..=3299 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in 3200-3299");
}

fn parse_headers(head: &str) -> Vec<(String, String)> {
    head.split("\r\n")
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn read_http_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break bytes.len();
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(pos) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let request_line = head.lines().next().unwrap_or_default().to_owned();
    let content_length = parse_headers(&head)
        .into_iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = bytes[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).expect("read body");
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    CapturedRequest {
        request_line,
        headers: parse_headers(&head),
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

fn start_openai_fixture(
    expected_auth: &'static str,
    answer: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("addr").port();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    let auth = request
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                    if auth != expected_auth {
                        let body = "{\"error\":\"unauthorized\"}";
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    let chunk = serde_json::json!({
                        "id": "oauth-fixture",
                        "object": "chat.completion.chunk",
                        "model": "copilot/test-model",
                        "choices": [{ "index": 0, "delta": { "content": answer }, "finish_reason": serde_json::Value::Null }]
                    })
                    .to_string();
                    let done = serde_json::json!({
                        "id": "oauth-fixture",
                        "object": "chat.completion.chunk",
                        "model": "copilot/test-model",
                        "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }],
                        "usage": { "prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5 }
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
    (format!("http://127.0.0.1:{port}"), handle)
}

fn start_gemini_fixture(
    expected_key: &'static str,
    captures: Arc<Mutex<Vec<CapturedRequest>>>,
) -> (String, JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("addr").port();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    captures.lock().expect("captures").push(request.clone());
                    let key = request
                        .headers
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("x-goog-api-key"))
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default();
                    if key != expected_key {
                        let body = "{\"error\":{\"message\":\"bad api key\"}}";
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    let chunk = serde_json::json!({
                        "candidates": [{
                            "content": { "parts": [{ "text": "adc fixture ok" }] },
                            "finishReason": "STOP"
                        }],
                        "usageMetadata": {
                            "promptTokenCount": 4,
                            "candidatesTokenCount": 2,
                            "totalTokenCount": 6
                        }
                    })
                    .to_string();
                    let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
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
    (format!("http://127.0.0.1:{port}/v1beta"), handle)
}

#[derive(Default)]
struct OAuthFixtureState {
    authorization_grants: usize,
    refresh_grants: usize,
}

fn parse_form_field(body: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

fn start_oauth_fixture(
    state: Arc<Mutex<OAuthFixtureState>>,
    initial_access: &'static str,
    refreshed_access: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("addr").port();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    if request.request_line.starts_with("GET /authorize") {
                        let target = request
                            .request_line
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("/authorize");
                        let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
                        let params: Vec<(String, String)> =
                            url::form_urlencoded::parse(query.as_bytes())
                                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                                .collect();
                        let redirect_uri = params
                            .iter()
                            .find(|(name, _)| name == "redirect_uri")
                            .map(|(_, value)| value.clone())
                            .unwrap_or_else(|| "http://127.0.0.1:3200/callback".to_owned());
                        let state_value = params
                            .iter()
                            .find(|(name, _)| name == "state")
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default();
                        let location =
                            format!("{redirect_uri}?code=oauth-code-123&state={state_value}");
                        let response = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    if request.request_line.starts_with("POST /token") {
                        let grant_type =
                            parse_form_field(&request.body, "grant_type").unwrap_or_default();
                        let (access, refresh, expires_in) = if grant_type == "authorization_code" {
                            state.lock().expect("state").authorization_grants += 1;
                            (initial_access, "oauth-refresh-1", 1_u64)
                        } else if grant_type == "refresh_token" {
                            state.lock().expect("state").refresh_grants += 1;
                            (refreshed_access, "oauth-refresh-2", 3600_u64)
                        } else {
                            let body = "{\"error\":\"unsupported_grant_type\"}";
                            let response = format!(
                                "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.flush();
                            continue;
                        };
                        let body = serde_json::json!({
                            "access_token": access,
                            "refresh_token": refresh,
                            "token_type": "Bearer",
                            "expires_in": expires_in
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    let response =
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

fn http_get_follow_once(url: &str) -> String {
    let first = http_get(url);
    if first.status == 302 {
        let location = first
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.clone())
            .expect("redirect location");
        return http_get(&location).body;
    }
    first.body
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn http_get(url: &str) -> HttpResponse {
    let rest = url
        .strip_prefix("http://")
        .expect("only loopback http URLs are supported");
    let (authority, target) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    let (host, port) = authority
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse::<u16>().expect("port")))
        .unwrap_or((authority, 80));
    let mut stream = TcpStream::connect((host, port)).expect("connect");
    stream
        .write_all(
            format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write request");
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0_u8; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(err) if err.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(err) => panic!("read response: {err}"),
        }
    }
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .expect("status");
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect::<Vec<_>>();
    HttpResponse {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[test]
fn oauth_authorize_callback_refresh_and_storage_work_for_provider_runs() {
    let sandbox = Sandbox::new("oauth-refresh");
    let oauth_state = Arc::new(Mutex::new(OAuthFixtureState::default()));
    let (oauth_base, oauth_handle) = start_oauth_fixture(
        Arc::clone(&oauth_state),
        "oauth-access-initial",
        "oauth-access-refreshed",
    );
    let (provider_base, provider_handle) =
        start_openai_fixture("Bearer oauth-access-refreshed", "oauth fixture ok");

    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["copilot"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "copilot/test-model"

[provider.copilot]
base_url = "{provider_base}"
protocol = "openai-chat"
models = [{{ id = "copilot/test-model", name = "Copilot Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
        ),
    )
    .expect("write config");

    let login_args_owned = vec![
        "auth".to_owned(),
        "login".to_owned(),
        "copilot".to_owned(),
        "--auth-method".to_owned(),
        "oauth".to_owned(),
        "--auth-input".to_owned(),
        "client_id=fixture-client".to_owned(),
        "--auth-input".to_owned(),
        "client_secret=fixture-secret".to_owned(),
        "--auth-input".to_owned(),
        format!("authorize_endpoint={oauth_base}/authorize"),
        "--auth-input".to_owned(),
        format!("token_endpoint={oauth_base}/token"),
        "--auth-input".to_owned(),
        "timeout_seconds=20".to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    let login_args = login_args_owned
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let (login, callback_body) = run_auth_with_browser(sandbox.command(&login_args));
    assert_eq!(
        login.status.code(),
        Some(0),
        "oauth login should succeed: {}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        callback_body.contains("Authorization complete"),
        "callback response should acknowledge completion: {callback_body}"
    );
    let login_doc: Value = serde_json::from_slice(&login.stdout).expect("login json");
    assert_eq!(login_doc["provider"], "copilot");
    assert_eq!(login_doc["stored"], true);

    let run_out = run(sandbox.command(&["run", "--format", "json", "verify oauth provider auth"]));
    oauth_handle.join().expect("join oauth fixture");
    provider_handle.join().expect("join provider fixture");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "provider run should succeed with refreshed oauth token: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(run_doc["model"], "copilot/test-model");
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("oauth fixture ok")),
        "run should return fixture content: {run_doc}"
    );

    let state = oauth_state.lock().expect("oauth state");
    assert!(
        state.authorization_grants >= 1,
        "oauth login should exchange an authorization code"
    );
    assert!(
        state.refresh_grants >= 1,
        "runtime provider should refresh expired oauth tokens before use"
    );

    let auth_file = sandbox.auth_file();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&auth_file)
            .expect("auth metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "auth store must remain 0600");
    }
    let auth_store: Value =
        serde_json::from_str(&fs::read_to_string(&auth_file).expect("read auth store"))
            .expect("auth json");
    let stored = auth_store["copilot"]["key"]
        .as_str()
        .expect("copilot auth payload");
    let payload: Value = serde_json::from_str(stored).expect("stored oauth payload");
    assert_eq!(payload["access_token"], "oauth-access-refreshed");
    assert_eq!(payload["client_id"], "fixture-client");
    assert_eq!(payload["token_endpoint"], format!("{oauth_base}/token"));

    let output = format!(
        "{}{}{}{}",
        String::from_utf8_lossy(&login.stdout),
        String::from_utf8_lossy(&login.stderr),
        String::from_utf8_lossy(&run_out.stdout),
        String::from_utf8_lossy(&run_out.stderr)
    );
    for secret in [
        "oauth-access-initial",
        "oauth-access-refreshed",
        "oauth-refresh-1",
        "oauth-refresh-2",
        "fixture-secret",
    ] {
        assert!(
            !output.contains(secret),
            "stdout/stderr must not leak oauth secret `{secret}`"
        );
    }
}

#[test]
fn bedrock_uses_aws_credential_chain_without_manual_auth_login() {
    let sandbox = Sandbox::new("aws-chain");
    let (provider_base, provider_handle) =
        start_openai_fixture("Bearer aws-profile-access", "bedrock chain ok");
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["bedrock"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "bedrock/test-model"

[provider.bedrock]
base_url = "{provider_base}"
protocol = "openai-chat"
models = [{{ id = "bedrock/test-model", name = "Bedrock Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
        ),
    )
    .expect("write config");
    let aws_dir = sandbox.home().join(".aws");
    fs::create_dir_all(&aws_dir).expect("create aws dir");
    fs::write(
        aws_dir.join("credentials"),
        "[default]\naws_access_key_id = aws-profile-access\naws_secret_access_key = aws-profile-secret\n",
    )
    .expect("write aws credentials");

    let run_out = run(sandbox.command(&["run", "--format", "json", "verify aws chain"]));
    provider_handle.join().expect("join fixture");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "bedrock run should succeed via aws credential chain: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(doc["model"], "bedrock/test-model");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("bedrock chain ok")),
        "fixture response should be visible: {doc}"
    );
}

#[test]
fn vertex_uses_google_adc_refresh_without_manual_key_entry() {
    let sandbox = Sandbox::new("google-adc");
    let captures = Arc::new(Mutex::new(Vec::new()));
    let (gemini_base, gemini_handle) =
        start_gemini_fixture("adc-access-refreshed", Arc::clone(&captures));

    let token_listener = bind_loopback_listener();
    let token_port = token_listener.local_addr().expect("token addr").port();
    token_listener
        .set_nonblocking(true)
        .expect("set nonblocking token listener");
    let token_handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match token_listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    if request.request_line.starts_with("POST /token")
                        && parse_form_field(&request.body, "grant_type").as_deref()
                            == Some("refresh_token")
                    {
                        let body = serde_json::json!({
                            "access_token": "adc-access-refreshed",
                            "expires_in": 3600,
                            "token_type": "Bearer"
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        return;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });

    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["vertex"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "vertex/gemini-test"

[provider.vertex]
base_url = "{gemini_base}"
protocol = "gemini"
models = [{{ id = "vertex/gemini-test", name = "Vertex Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
        ),
    )
    .expect("write config");

    let adc_dir = sandbox.home().join(".config/gcloud");
    fs::create_dir_all(&adc_dir).expect("create adc dir");
    let adc_path = adc_dir.join("application_default_credentials.json");
    fs::write(
        &adc_path,
        serde_json::json!({
            "type": "authorized_user",
            "client_id": "adc-client-id",
            "client_secret": "adc-client-secret",
            "refresh_token": "adc-refresh-token",
            "token_uri": format!("http://127.0.0.1:{token_port}/token"),
            "access_token": "adc-stale-token",
            "expires_at": now_secs().saturating_sub(120)
        })
        .to_string(),
    )
    .expect("write adc file");

    let run_out = run(sandbox.command(&["run", "--format", "json", "verify google adc refresh"]));
    token_handle.join().expect("join token fixture");
    gemini_handle.join().expect("join gemini fixture");
    assert_eq!(
        run_out.status.code(),
        Some(0),
        "vertex run should succeed via google adc refresh: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let doc: Value = serde_json::from_slice(&run_out.stdout).expect("run json");
    assert_eq!(doc["model"], "vertex/gemini-test");
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("adc fixture ok")),
        "fixture response should be visible: {doc}"
    );
    let captured = captures.lock().expect("captures");
    assert!(
        captured
            .iter()
            .any(|request| request.headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("x-goog-api-key") && value == "adc-access-refreshed"
            })),
        "gemini request should use the refreshed ADC token as request credential"
    );
    let refreshed: Value =
        serde_json::from_str(&fs::read_to_string(&adc_path).expect("read refreshed adc"))
            .expect("adc json");
    assert_eq!(refreshed["access_token"], "adc-access-refreshed");
    assert!(
        refreshed["expires_at"].as_u64().unwrap_or_default() > now_secs(),
        "ADC file should persist refreshed expiry"
    );
}
