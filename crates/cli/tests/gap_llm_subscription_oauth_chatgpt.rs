//! Gap tests: ChatGPT subscription OAuth login + subscription request routing.
//!
//! - #27/#28: login authorize URL carries Codex/ChatGPT public client defaults.
//! - #29 (RED until #30): outbound provider requests target the Codex backend
//!   (`…/backend-api/codex/responses`, not api.openai.com) and carry
//!   `ChatGPT-Account-Id` extracted from the stored OAuth `id_token` claims.
//!
//! Loopback AS / provider fixtures are injected so tests never hit the real
//! network. Secrets (access/refresh/id tokens, code_verifier) must never appear
//! on stdout/stderr.

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

/// Public ChatGPT/Codex OAuth client id (non-secret; redirect is loopback-only).
const CHATGPT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

const CHATGPT_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access"];

struct Sandbox {
    root: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-gap-chatgpt-oauth-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(root.join(".config")).expect("create config");
        std::fs::create_dir_all(root.join(".local/share")).expect("create xdg data");
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
            .env_remove("OPENAI_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET")
            // Ensure profile-driven values are not shadowed by a leftover shell env.
            .env_remove("CORTEX_OAUTH_CHATGPT_CLIENT_ID")
            .env_remove("CORTEX_OAUTH_CHATGPT_SCOPES")
            .env_remove("CORTEX_OAUTH_CHATGPT_AUTHORIZE_EXTRA_PARAMS")
            .env_remove("CORTEX_OAUTH_CLIENT_ID")
            .env_remove("CORTEX_OAUTH_SCOPES")
            .env_remove("CORTEX_OAUTH_AUTHORIZE_EXTRA_PARAMS");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct OAuthFixtureState {
    /// Full authorize request target (`/authorize?...`), never printed.
    authorize_target: Option<String>,
    authorize_code_challenge_method: Option<String>,
    authorize_client_id: Option<String>,
    authorize_scope: Option<String>,
    authorize_extras: Vec<(String, String)>,
    authorization_grants: usize,
}

fn bind_loopback_listener() -> TcpListener {
    TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener")
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

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
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
    let headers = parse_headers(&head);
    let content_length = headers
        .iter()
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
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

fn parse_form_field(body: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// Minimal OAuth AS fixture: GET /authorize → 302 to redirect_uri; POST /token → tokens.
/// Captures authorize query params for assertions (never logs them).
fn start_oauth_fixture(state: Arc<Mutex<OAuthFixtureState>>) -> (String, JoinHandle<()>) {
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
                            .unwrap_or_else(|| {
                                "http://localhost:1455/auth/callback".to_owned()
                            });
                        let state_value = params
                            .iter()
                            .find(|(name, _)| name == "state")
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default();
                        {
                            let mut guard = state.lock().expect("state");
                            guard.authorize_target = Some(target.to_owned());
                            guard.authorize_code_challenge_method = params
                                .iter()
                                .find(|(name, _)| name == "code_challenge_method")
                                .map(|(_, value)| value.clone());
                            guard.authorize_client_id = params
                                .iter()
                                .find(|(name, _)| name == "client_id")
                                .map(|(_, value)| value.clone());
                            guard.authorize_scope = params
                                .iter()
                                .find(|(name, _)| name == "scope")
                                .map(|(_, value)| value.clone());
                            // Capture non-standard extras (everything beyond the OAuth core set).
                            const CORE: &[&str] = &[
                                "response_type",
                                "client_id",
                                "redirect_uri",
                                "state",
                                "scope",
                                "code_challenge",
                                "code_challenge_method",
                            ];
                            guard.authorize_extras = params
                                .into_iter()
                                .filter(|(name, _)| !CORE.contains(&name.as_str()))
                                .collect();
                        }
                        let location =
                            format!("{redirect_uri}?code=chatgpt-oauth-code&state={state_value}");
                        let response = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    if request.request_line.starts_with("POST /token") {
                        state.lock().expect("state").authorization_grants += 1;
                        let _ = parse_form_field(&request.body, "code_verifier");
                        let body = serde_json::json!({
                            "access_token": "chatgpt-access-fixture",
                            "refresh_token": "chatgpt-refresh-fixture",
                            "id_token": "chatgpt-id-token-fixture",
                            "token_type": "Bearer",
                            "expires_in": 3600
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

/// Spawn `cortex auth login …`, capture the printed authorize URL from stderr, follow it once
/// (so the loopback AS can 302 to the CLI callback), then wait for the process.
fn run_auth_with_browser(mut command: Command) -> (Output, String, String) {
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
                        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
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
                "chatgpt oauth login should print authorize URL (status={status:?})\nstdout={}\nstderr={}",
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
        authorize_url,
        callback_body,
    )
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn http_get(url: &str) -> HttpResponse {
    let rest = url
        .strip_prefix("http://")
        .expect("only loopback http URLs are supported in this fixture helper");
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
        .set_read_timeout(Some(RUN_TIMEOUT))
        .expect("set read timeout");
    stream
        .set_write_timeout(Some(RUN_TIMEOUT))
        .expect("set write timeout");
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
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => break,
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
        .unwrap_or(0);
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

fn query_params(authorize_url: &str) -> Vec<(String, String)> {
    let url = url::Url::parse(authorize_url).expect("authorize URL must parse");
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// #27 — ChatGPT subscription OAuth login authorize URL carries Codex client defaults.
///
/// RED until `chatgpt` lands in `OAUTH_DEFAULTS` / profile (`#28`) and the authorize URL
/// builder wires profile extras + PKCE (W3) for the builtin OAuth path.
#[test]
fn chatgpt_oauth_login_authorize_url_carries_codex_client_defaults() {
    let sandbox = Sandbox::new("login-authorize");
    let oauth_state = Arc::new(Mutex::new(OAuthFixtureState::default()));
    let (oauth_base, oauth_handle) = start_oauth_fixture(Arc::clone(&oauth_state));

    // Loopback AS only — client_id / scopes / extras must resolve from the chatgpt profile
    // (or fail RED until #28). Do not pass them via --auth-input.
    let mut login_cmd = sandbox.command(&[
        "auth",
        "login",
        "chatgpt",
        "--auth-method",
        "oauth",
        "--format",
        "json",
    ]);
    login_cmd.env(
        "CORTEX_OAUTH_CHATGPT_AUTHORIZE_ENDPOINT",
        format!("{oauth_base}/authorize"),
    );
    login_cmd.env(
        "CORTEX_OAUTH_CHATGPT_TOKEN_ENDPOINT",
        format!("{oauth_base}/token"),
    );
    // Prefer the product-pinned ChatGPT callback port/path when the engine honors it.
    // If the engine still scans 3200-3299 only, #28 may also expand the pin — either way
    // the authorize URL redirect_uri is asserted below.
    login_cmd.env("CORTEX_OAUTH_CHATGPT_CALLBACK_PORT", "1455");

    let (login, authorize_url, _callback_body) = run_auth_with_browser(login_cmd);
    let _ = oauth_handle.join();

    let stdout = String::from_utf8_lossy(&login.stdout);
    let stderr = String::from_utf8_lossy(&login.stderr);
    let combined = format!("{stdout}{stderr}");

    // Secret hygiene: fixture tokens must never leak to CLI surfaces.
    for secret in [
        "chatgpt-access-fixture",
        "chatgpt-refresh-fixture",
        "chatgpt-id-token-fixture",
        "chatgpt-oauth-code",
    ] {
        assert!(
            !combined.contains(secret),
            "stdout/stderr must not leak oauth secret `{secret}`"
        );
    }

    let params = query_params(&authorize_url);

    assert_eq!(
        param(&params, "client_id"),
        Some(CHATGPT_CLIENT_ID),
        "authorize URL must carry ChatGPT client_id={CHATGPT_CLIENT_ID}; url={authorize_url}"
    );

    let scope = param(&params, "scope").unwrap_or_default();
    let scope_tokens: Vec<&str> = scope.split_whitespace().collect();
    for required in CHATGPT_SCOPES {
        assert!(
            scope_tokens.contains(required),
            "authorize URL scope must include `{required}`; got `{scope}` url={authorize_url}"
        );
    }

    assert_eq!(
        param(&params, "code_challenge_method"),
        Some("S256"),
        "authorize URL must carry code_challenge_method=S256; url={authorize_url}"
    );
    let challenge = param(&params, "code_challenge").unwrap_or_default();
    assert!(
        !challenge.is_empty() && !challenge.contains('='),
        "code_challenge must be non-empty base64url without padding; url={authorize_url}"
    );

    assert_eq!(
        param(&params, "id_token_add_organizations"),
        Some("true"),
        "authorize URL must carry id_token_add_organizations=true; url={authorize_url}"
    );
    assert_eq!(
        param(&params, "codex_cli_simplified_flow"),
        Some("true"),
        "authorize URL must carry codex_cli_simplified_flow=true; url={authorize_url}"
    );

    // Pinned ChatGPT callback path (plan #28 redirect).
    let redirect_uri = param(&params, "redirect_uri").unwrap_or_default();
    assert!(
        redirect_uri.contains("/auth/callback"),
        "redirect_uri must use the pinned /auth/callback path; got `{redirect_uri}`"
    );
    assert!(
        redirect_uri.contains("1455")
            || redirect_uri.contains("localhost")
            || redirect_uri.contains("127.0.0.1"),
        "redirect_uri must be loopback (preferably port 1455); got `{redirect_uri}`"
    );

    // Cross-check fixture capture when the browser hop reached the AS.
    let state = oauth_state.lock().expect("oauth state");
    if state.authorize_target.is_some() {
        assert_eq!(
            state.authorize_client_id.as_deref(),
            Some(CHATGPT_CLIENT_ID)
        );
        assert_eq!(
            state.authorize_code_challenge_method.as_deref(),
            Some("S256")
        );
        let extras: Vec<(&str, &str)> = state
            .authorize_extras
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(
            extras.contains(&("id_token_add_organizations", "true")),
            "fixture must observe id_token_add_organizations=true; extras={extras:?}"
        );
        assert!(
            extras.contains(&("codex_cli_simplified_flow", "true")),
            "fixture must observe codex_cli_simplified_flow=true; extras={extras:?}"
        );
    }

    assert_eq!(
        login.status.code(),
        Some(0),
        "chatgpt oauth login should succeed once profile + extras are wired: stderr={stderr}"
    );
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

fn b64url_json(value: &Value) -> String {
    let raw = serde_json::to_vec(value).expect("json");
    base64url_nopad(&raw)
}

fn base64url_nopad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = bytes.get(i + 1).copied();
        let b2 = bytes.get(i + 2).copied();
        out.push(TABLE[(b0 >> 2) as usize] as char);
        match (b1, b2) {
            (Some(b1), Some(b2)) => {
                out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
                out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
                out.push(TABLE[(b2 & 0x3f) as usize] as char);
                i += 3;
            }
            (Some(b1), None) => {
                out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
                out.push(TABLE[((b1 & 0x0f) << 2) as usize] as char);
                i += 2;
            }
            (None, _) => {
                out.push(TABLE[((b0 & 0x03) << 4) as usize] as char);
                i += 1;
            }
        }
    }
    out
}

/// Build a JWT-shaped id_token whose payload carries ChatGPT account claims.
///
/// Mirrors Codex claim extraction order:
/// `chatgpt_account_id` → `https://api.openai.com/auth.chatgpt_account_id` →
/// `organizations[0].id`. Signature is a non-empty fixture stub (runtime only
/// base64url-decodes the payload).
fn chatgpt_id_token_with_account(account_id: &str) -> String {
    let header = b64url_json(&serde_json::json!({"alg": "none", "typ": "JWT"}));
    let payload = b64url_json(&serde_json::json!({
        "chatgpt_account_id": account_id,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id
        },
        "organizations": [{ "id": account_id }],
        "email": "fixture@example.invalid"
    }));
    format!("{header}.{payload}.chatgpt-id-token-sig")
}

fn write_chatgpt_oauth_credential(sandbox: &Sandbox, account_id: &str) -> String {
    let id_token = chatgpt_id_token_with_account(account_id);
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() + 3600)
        .unwrap_or(u64::MAX);
    let payload = serde_json::json!({
        "type": "oauth",
        "access_token": "chatgpt-access-request-fixture",
        "refresh_token": "chatgpt-refresh-request-fixture",
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_at": expires_at,
        "client_id": CHATGPT_CLIENT_ID,
        "token_endpoint": "https://auth.openai.com/oauth/token"
    });
    let key = payload.to_string();
    let store = serde_json::json!({
        "chatgpt": { "key": key }
    });
    if let Some(parent) = sandbox.auth_file().parent() {
        fs::create_dir_all(parent).expect("create auth parent");
    }
    fs::write(
        sandbox.auth_file(),
        serde_json::to_string_pretty(&store).expect("auth json"),
    )
    .expect("write auth.json");
    id_token
}

/// Loopback OpenAI-Responses SSE fixture that records every inbound request.
fn start_responses_fixture(
    answer: &'static str,
) -> (String, Arc<Mutex<Vec<CapturedRequest>>>, JoinHandle<()>) {
    let listener = bind_loopback_listener();
    let port = listener.local_addr().expect("addr").port();
    listener
        .set_nonblocking(true)
        .expect("set nonblocking listener");
    let captures = Arc::new(Mutex::new(Vec::new()));
    let captures_thread = Arc::clone(&captures);
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_http_request(&mut stream);
                    captures_thread.lock().expect("captures").push(request);
                    let chunk = serde_json::json!({
                        "type": "response.output_text.delta",
                        "delta": answer
                    })
                    .to_string();
                    let done = serde_json::json!({
                        "type": "response.completed",
                        "response": {
                            "usage": {
                                "input_tokens": 4,
                                "output_tokens": 2,
                                "total_tokens": 6
                            }
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

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// #29 — ChatGPT subscription OAuth run targets Codex backend + account header.
///
/// RED until `#30` wires runtime_provider for chatgpt OAuth:
/// - default/subscription base → `https://chatgpt.com/backend-api/codex`
///   (stream path `/responses` → full `…/backend-api/codex/responses`)
/// - `ChatGPT-Account-Id` from stored `id_token` JWT claims
/// - never `api.openai.com`
#[test]
fn chatgpt_oauth_subscription_requests_use_codex_backend_and_account_id() {
    let sandbox = Sandbox::new("subscription-request");
    const ACCOUNT_ID: &str = "acct_chatgpt_fixture_9f3a";
    let id_token = write_chatgpt_oauth_credential(&sandbox, ACCOUNT_ID);

    let (provider_base, captures, provider_handle) =
        start_responses_fixture("chatgpt subscription fixture ok");

    // Loopback stand-in for https://chatgpt.com/backend-api/codex — path under the
    // host must remain /backend-api/codex/responses (openai-responses → /responses).
    // Do not point at api.openai.com/v1. #30 supplies this default when base_url is
    // unset; the override here is only so the fixture can observe the outbound call.
    let subscription_base = format!("{provider_base}/backend-api/codex");
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            r#"
[provider]
enabled = ["chatgpt"]
disabled = ["openrouter"]

[provider.model_overrides]
"google/gemini-2.5-flash" = "chatgpt/codex-fixture"

[provider.chatgpt]
base_url = "{subscription_base}"
protocol = "openai-responses"
models = [{{ id = "chatgpt/codex-fixture", name = "ChatGPT Codex Fixture", tool_call = true, context_length = 8192, max_output_tokens = 1024 }}]
"#
        ),
    )
    .expect("write config");

    let run_out = run(sandbox.command(&[
        "run",
        "--format",
        "json",
        "verify chatgpt subscription route",
    ]));
    provider_handle.join().expect("join provider fixture");

    let stdout = String::from_utf8_lossy(&run_out.stdout);
    let stderr = String::from_utf8_lossy(&run_out.stderr);
    let combined = format!("{stdout}{stderr}");

    // Secret hygiene: tokens + full id_token must never leak to CLI surfaces.
    for secret in [
        "chatgpt-access-request-fixture",
        "chatgpt-refresh-request-fixture",
        id_token.as_str(),
        "chatgpt-id-token-sig",
    ] {
        assert!(
            !combined.contains(secret),
            "stdout/stderr must not leak oauth secret material `{secret}`"
        );
    }

    let captured = captures.lock().expect("captures");
    let first = captured.first().unwrap_or_else(|| {
        panic!(
            "chatgpt subscription run must issue a provider request\n\
             status={:?}\nstdout={stdout}\nstderr={stderr}",
            run_out.status.code()
        )
    });

    // Production target: https://chatgpt.com/backend-api/codex/responses
    // Loopback observes the same path under the fixture host — never api.openai.com.
    assert!(
        first
            .request_line
            .contains("POST /backend-api/codex/responses"),
        "outbound request must target …/backend-api/codex/responses \
         (Codex subscription route), not api.openai.com; got `{}`",
        first.request_line
    );
    assert!(
        !first.request_line.contains("/v1/"),
        "must not use OpenAI API /v1 paths; got `{}`",
        first.request_line
    );
    let host = header_value(&first.headers, "host").unwrap_or_default();
    assert!(
        !host.contains("api.openai.com"),
        "Host must not be api.openai.com; got `{host}`"
    );

    let account_header = header_value(&first.headers, "ChatGPT-Account-Id");
    assert_eq!(
        account_header,
        Some(ACCOUNT_ID),
        "ChatGPT-Account-Id must equal id_token chatgpt_account_id claim `{ACCOUNT_ID}`; \
         headers={:?}",
        first.headers
    );

    let auth = header_value(&first.headers, "authorization").unwrap_or_default();
    assert!(
        auth == "Bearer chatgpt-access-request-fixture",
        "Authorization must carry the stored access_token as Bearer; got `{auth}`"
    );

    assert_eq!(
        run_out.status.code(),
        Some(0),
        "chatgpt subscription run should succeed once route + account header are wired (#30): \
         stderr={stderr}"
    );
    let run_doc: Value = serde_json::from_slice(&run_out.stdout).unwrap_or_else(|_| {
        panic!("run json parse failed: stdout={stdout}");
    });
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|text| text.contains("chatgpt subscription fixture ok")),
        "run should return fixture content: {run_doc}"
    );
}

