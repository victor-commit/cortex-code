//! Gap test: xAI xAI subscription OAuth login defaults (plan #31 RED).
//!
//! Product must supply (without `--auth-input` for these fields):
//! - client_id `b1a00492-073a-47ea-816f-4c329264a828`
//! - pinned redirect `http://127.0.0.1:56121/callback`
//! - scopes `openid profile email offline_access grok-cli:access api:access`
//!
//! Loopback authorize/token endpoints may be injected via `CORTEX_OAUTH_XAI_*`.
//! GREEN lands with plan #32 (xAI profile + `OAUTH_DEFAULTS` + CLI defaults merge).

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const AUTH_TIMEOUT: Duration = Duration::from_secs(35);

/// xAI public (non-secret) OAuth client id — product default after #32.
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
/// Pinned loopback callback; product must not silently pick another port.
const XAI_REDIRECT_URI: &str = "http://127.0.0.1:56121/callback";
const XAI_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "email",
    "offline_access",
    "grok-cli:access",
    "api:access",
];

#[derive(Clone, Debug)]
struct CapturedRequest {
    request_line: String,
    #[allow(dead_code)] // retained for parity with cloud oauth fixture capture
    headers: Vec<(String, String)>,
    #[allow(dead_code)]
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
            "cortex-gap-xai-oauth-{tag}-{}-{}",
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
            .env_remove("XAI_API_KEY")
            .env_remove("CORTEX_AUTH_FILE")
            .env_remove("CORTEX_NO_NET")
            .env_remove("CORTEX_OAUTH_XAI_CLIENT_ID")
            .env_remove("CORTEX_OAUTH_XAI_SCOPES")
            .env_remove("CORTEX_OAUTH_XAI_CALLBACK_PORT")
            .env_remove("CORTEX_OAUTH_CLIENT_ID")
            .env_remove("CORTEX_OAUTH_SCOPES")
            .env_remove("CORTEX_OAUTH_CALLBACK_PORT");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}


fn run_auth_capture(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn auth login");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stdout = stdout;
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut stderr = stderr;
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let status = wait_child(&mut child, AUTH_TIMEOUT);
    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
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

#[derive(Default)]
struct OAuthFixtureState {
    authorization_grants: usize,
    /// Captured from the authorize redirect query (never logged to stdout).
    authorize_client_id: Option<String>,
    authorize_redirect_uri: Option<String>,
    authorize_scope: Option<String>,
    /// Device-code authorization POSTs (RFC 8628).
    device_authorization_grants: usize,
    device_client_id: Option<String>,
    device_scope: Option<String>,
    device_token_grants: usize,
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
                            .unwrap_or_else(|| XAI_REDIRECT_URI.to_owned());
                        let state_value = params
                            .iter()
                            .find(|(name, _)| name == "state")
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default();
                        let client_id = params
                            .iter()
                            .find(|(name, _)| name == "client_id")
                            .map(|(_, value)| value.clone());
                        let scope = params
                            .iter()
                            .find(|(name, _)| name == "scope")
                            .map(|(_, value)| value.clone());
                        {
                            let mut guard = state.lock().expect("state");
                            guard.authorize_client_id = client_id;
                            guard.authorize_redirect_uri = Some(redirect_uri.clone());
                            guard.authorize_scope = scope;
                        }
                        let location =
                            format!("{redirect_uri}?code=oauth-code-xai&state={state_value}");
                        let response = format!(
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        continue;
                    }
                    if request.request_line.starts_with("POST /device/code")
                        || request.request_line.starts_with("POST /oauth2/device/code")
                    {
                        {
                            let mut guard = state.lock().expect("state");
                            guard.device_authorization_grants += 1;
                            guard.device_client_id =
                                parse_form_field(&request.body, "client_id");
                            guard.device_scope = parse_form_field(&request.body, "scope");
                        }
                        let body = serde_json::json!({
                            "device_code": "xai-device-code-secret",
                            "user_code": "WDJB-MJHT",
                            "verification_uri": "http://127.0.0.1/device",
                            "expires_in": 600,
                            "interval": 1
                        })
                        .to_string();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
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
                            (initial_access, "xai-refresh-1", 3600_u64)
                        } else if grant_type
                            == "urn:ietf:params:oauth:grant-type:device_code"
                        {
                            let device_code =
                                parse_form_field(&request.body, "device_code").unwrap_or_default();
                            assert_eq!(
                                device_code, "xai-device-code-secret",
                                "token poll must present the opaque device_code from authorization"
                            );
                            state.lock().expect("state").device_token_grants += 1;
                            (initial_access, "xai-device-refresh-1", 3600_u64)
                        } else if grant_type == "refresh_token" {
                            (refreshed_access, "xai-refresh-2", 3600_u64)
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

/// Login case: product defaults must pin xAI client_id, callback port 56121, and scopes.
///
/// Only authorize/token endpoints are overridden via `CORTEX_OAUTH_XAI_*` for the loopback fixture.
/// Do **not** pass client_id / scopes / callback_port via `--auth-input` — those are the product
/// contract under test.
#[test]
fn xai_subscription_oauth_login_uses_product_client_id_pinned_redirect_and_scopes() {
    with_xai_pinned_callback_port(|| {
        let sandbox = Sandbox::new("login-defaults");
        let oauth_state = Arc::new(Mutex::new(OAuthFixtureState::default()));
        let (oauth_base, oauth_handle) = start_oauth_fixture(
            Arc::clone(&oauth_state),
            "xai-access-initial",
            "xai-access-refreshed",
        );

        let mut cmd = sandbox.command(&[
            "auth",
            "login",
            "xai",
            "--auth-method",
            "oauth",
            // Intentional: no client_id / scopes / callback_port / authorize / token via --auth-input.
            "--auth-input",
            "timeout_seconds=20",
            "--format",
            "json",
        ]);
        cmd.env(
            "CORTEX_OAUTH_XAI_AUTHORIZE_ENDPOINT",
            format!("{oauth_base}/authorize"),
        );
        cmd.env(
            "CORTEX_OAUTH_XAI_TOKEN_ENDPOINT",
            format!("{oauth_base}/token"),
        );

        let (login, callback_body) = run_auth_with_browser(cmd);
        let _ = oauth_handle.join();

        assert_eq!(
            login.status.code(),
            Some(0),
            "xai oauth login should succeed with product defaults + CORTEX_OAUTH_XAI_* endpoints only; stderr={}",
            String::from_utf8_lossy(&login.stderr)
        );
        assert!(
            callback_body.contains("Authorization complete"),
            "callback response should acknowledge completion: {callback_body}"
        );

        let state = oauth_state.lock().expect("oauth state");
        assert_eq!(
            state.authorize_client_id.as_deref(),
            Some(XAI_CLIENT_ID),
            "authorize URL must carry xAI public client_id from product defaults"
        );
        assert_eq!(
            state.authorize_redirect_uri.as_deref(),
            Some(XAI_REDIRECT_URI),
            "product must pin redirect_uri to http://127.0.0.1:56121/callback (port 56121; no silent fallback)"
        );
        let scope = state
            .authorize_scope
            .as_deref()
            .expect("authorize URL must carry scope");
        let scope_tokens: Vec<&str> = scope
            .split([' ', '+', ','])
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .collect();
        for expected in XAI_SCOPES {
            assert!(
                scope_tokens.iter().any(|token| token == expected),
                "authorize scope missing `{expected}`: {scope:?}"
            );
        }
        assert!(
            state.authorization_grants >= 1,
            "oauth login should exchange an authorization code"
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
        let auth_store: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth_file).expect("read auth store"))
                .expect("auth json");
        let stored = auth_store["xai"]["key"]
            .as_str()
            .expect("xai auth payload");
        let payload: serde_json::Value = serde_json::from_str(stored).expect("stored oauth payload");
        assert_eq!(payload["type"], "oauth");
        assert_eq!(payload["client_id"], XAI_CLIENT_ID);

        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&login.stdout),
            String::from_utf8_lossy(&login.stderr)
        );
        for secret in [
            "xai-access-initial",
            "xai-access-refreshed",
            "xai-refresh-1",
            "xai-refresh-2",
            "oauth-code-xai",
        ] {
            assert!(
                !output.contains(secret),
                "stdout/stderr must not leak oauth secret `{secret}`"
            );
        }
    });
}

/// Device-code xAI login: product client_id + default device endpoint (overridable for
/// loopback), prints user_code (never device_code), stores refreshable oauth under `xai`.
#[test]
fn xai_subscription_oauth_device_method_completes_and_stores_refreshable_credential() {
    let sandbox = Sandbox::new("device-login");
    let oauth_state = Arc::new(Mutex::new(OAuthFixtureState::default()));
    let (oauth_base, oauth_handle) = start_oauth_fixture(
        Arc::clone(&oauth_state),
        "xai-device-access-initial",
        "xai-device-access-refreshed",
    );

    // Product contract: no client_id / scopes via --auth-input. Only loopback endpoints for the fixture.
    let mut cmd = sandbox.command(&[
        "auth",
        "login",
        "xai",
        "--auth-method",
        "device",
        "--format",
        "json",
    ]);
    cmd.env(
        "CORTEX_OAUTH_XAI_DEVICE_AUTHORIZATION_ENDPOINT",
        format!("{oauth_base}/device/code"),
    );
    cmd.env(
        "CORTEX_OAUTH_XAI_TOKEN_ENDPOINT",
        format!("{oauth_base}/token"),
    );

    let login = run_auth_capture(cmd);
    let _ = oauth_handle.join();

    let stdout = String::from_utf8_lossy(&login.stdout);
    let stderr = String::from_utf8_lossy(&login.stderr);
    let combined = format!("{stdout}{stderr}");

    assert_eq!(
        login.status.code(),
        Some(0),
        "xai device login should succeed with product client_id + CORTEX_OAUTH_XAI_* endpoints; stderr={stderr}"
    );

    assert!(
        stderr.contains("enter code: WDJB-MJHT") || combined.contains("WDJB-MJHT"),
        "device flow must print the user-facing user_code, not only hang on poll: stderr={stderr}"
    );
    assert!(
        stderr.contains("open this URL to authorize:") || combined.contains("http://127.0.0.1/device"),
        "device flow must print verification_uri: stderr={stderr}"
    );
    assert!(
        !combined.contains("xai-device-code-secret"),
        "stdout/stderr must never leak the opaque device_code"
    );

    let state = oauth_state.lock().expect("oauth state");
    assert!(
        state.device_authorization_grants >= 1,
        "device login must POST the device authorization endpoint"
    );
    assert_eq!(
        state.device_client_id.as_deref(),
        Some(XAI_CLIENT_ID),
        "device authorization must carry xAI public client_id from product defaults"
    );
    let device_scope = state
        .device_scope
        .as_deref()
        .expect("device authorization must request scopes");
    let scope_tokens: Vec<&str> = device_scope
        .split([' ', '+', ','])
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect();
    for expected in XAI_SCOPES {
        assert!(
            scope_tokens.iter().any(|token| token == expected),
            "device scope missing `{expected}`: {device_scope:?}"
        );
    }
    assert!(
        state.device_token_grants >= 1,
        "device login must poll the token endpoint with the device_code grant"
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
    let auth_store: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&auth_file).expect("read auth store"))
            .expect("auth json");
    let stored = auth_store["xai"]["key"]
        .as_str()
        .expect("xai auth payload");
    let payload: serde_json::Value = serde_json::from_str(stored).expect("stored oauth payload");
    assert_eq!(payload["type"], "oauth");
    assert_eq!(payload["client_id"], XAI_CLIENT_ID);
    assert_eq!(payload["access_token"], "xai-device-access-initial");
    assert_eq!(
        payload["refresh_token"], "xai-device-refresh-1",
        "device login must store a refreshable credential"
    );
    assert!(
        payload["token_endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.ends_with("/token")),
        "stored credential must retain token_endpoint for refresh: {payload:?}"
    );

    for secret in [
        "xai-device-access-initial",
        "xai-device-access-refreshed",
        "xai-device-refresh-1",
        "xai-device-code-secret",
    ] {
        assert!(
            !combined.contains(secret),
            "stdout/stderr must not leak oauth secret `{secret}`"
        );
    }
}

/// Process-wide lock so tests that need exclusive xAI pin 56121 do not race.
fn with_xai_pinned_callback_port<R>(f: impl FnOnce() -> R) -> R {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}

/// Occupied pin must fail closed with an actionable error — never silent fallback to 3200–3299.
#[test]
fn xai_oauth_login_fails_actionably_when_pinned_callback_port_56121_is_occupied() {
    with_xai_pinned_callback_port(|| {
        let sandbox = Sandbox::new("port-occupied");
        // Hold the product-pinned xAI callback for the whole login attempt.
        let occupied = TcpListener::bind(("127.0.0.1", 56121)).expect(
            "fixture must occupy xAI pinned callback port 56121",
        );
        assert_eq!(
            occupied.local_addr().expect("addr").port(),
            56121,
            "fixture must hold exactly port 56121"
        );

        let mut cmd = sandbox.command(&[
            "auth",
            "login",
            "xai",
            "--auth-method",
            "oauth",
            "--auth-input",
            "timeout_seconds=5",
            "--format",
            "json",
        ]);
        // Unreachable endpoints: if the pin were silently abandoned for 3200–3299, login still
        // must not succeed — and must not print an authorize URL on a non-pin port.
        cmd.env(
            "CORTEX_OAUTH_XAI_AUTHORIZE_ENDPOINT",
            "http://127.0.0.1:9/authorize",
        );
        cmd.env(
            "CORTEX_OAUTH_XAI_TOKEN_ENDPOINT",
            "http://127.0.0.1:9/token",
        );

        let login = run_auth_capture(cmd);
        // Keep `occupied` alive until after the child exits.
        drop(occupied);

        let stdout = String::from_utf8_lossy(&login.stdout);
        let stderr = String::from_utf8_lossy(&login.stderr);
        let combined = format!("{stdout}{stderr}");
        let combined_lower = combined.to_ascii_lowercase();

        assert_ne!(
            login.status.code(),
            Some(0),
            "pinned-port collision must fail closed (not silent success); stderr={stderr}"
        );
        assert!(
            stderr.contains("56121") || combined.contains("56121"),
            "error must name pinned port 56121 so the user can free it; stderr={stderr}"
        );
        let actionable = ["already in use", "in use", "occupied", "address already in use"]
            .iter()
            .any(|needle| combined_lower.contains(needle));
        assert!(
            actionable,
            "error must be actionable about occupancy (in use / occupied); stderr={stderr}"
        );
        assert!(
            !combined.contains("open this URL to authorize:"),
            "must not print authorize URL after failing to bind the pin (no silent fallback): {combined}"
        );
        assert!(
            !combined.contains("waiting for OAuth callback on http://127.0.0.1:"),
            "must not wait on any loopback callback after pin bind failure: {combined}"
        );
        // Explicit: no mission-range silent fallback in user-visible output.
        for port in 3200u16..=3299 {
            let needle = format!("127.0.0.1:{port}");
            assert!(
                !combined.contains(&needle),
                "must not fall back to mission scan port {port} when pin 56121 is set: {combined}"
            );
        }
        if sandbox.auth_file().exists() {
            let auth_store: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(sandbox.auth_file()).expect("read auth store"),
            )
            .expect("auth json");
            assert!(
                auth_store.get("xai").is_none(),
                "failed pin bind must not store an xai credential: {auth_store:?}"
            );
        }
    });
}

