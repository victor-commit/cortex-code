//! The MCP-remote OAuth 2.0 authorization-code flow with a CSRF-validated loopback callback.
//!
//! When a remote server answers the handshake with `401`/needs-auth, the user runs the auth flow:
//! cortex discovers the server's authorization and token endpoints, generates a random CSRF
//! `state`, starts a loopback HTTP listener on a port in the mission's reserved range (3200-3299)
//! or a product-pinned callback port (e.g. ChatGPT Codex `1455`, xAI xAI `56121`),
//! and prints an authorization URL. The user (or an automated validator) visits that URL; the
//! server redirects back to the loopback callback with an authorization `code` and the `state`.
//! cortex validates the returned `state` against the one it generated (a forged/mismatched state is
//! rejected), exchanges the code for tokens at the token endpoint, and returns the credential to be
//! persisted. Expired credentials are refreshed in place via the refresh token.
//!
//! Token values are never logged or printed here — only the authorization URL (which carries no
//! secret) is surfaced.

use std::future::Future;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::credentials::{StoredToken, now_secs};

/// The reserved loopback port range the OAuth callback binds within (mission boundary).
const CALLBACK_PORT_RANGE: std::ops::RangeInclusive<u16> = 3200..=3299;

/// Product-pinned callback ports allowed outside the mission scan range (explicit pin only).
/// ChatGPT/Codex registers `http://localhost:1455/auth/callback`; xAI uses `56121`.
const PINNED_CALLBACK_PORTS: &[u16] = &[1455, 56121];

/// Default loopback callback path when a provider does not pin a product path.
const DEFAULT_CALLBACK_PATH: &str = "/callback";

/// A failure during the OAuth authorization flow.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// An HTTP request (metadata discovery or token exchange) failed.
    #[error("OAuth HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// No free loopback port could be bound in the reserved range.
    #[error("no free loopback callback port available in {}-{}", CALLBACK_PORT_RANGE.start(), CALLBACK_PORT_RANGE.end())]
    NoCallbackPort,
    /// Binding or operating the loopback callback listener failed.
    #[error("loopback callback error: {0}")]
    Callback(String),
    /// The authorization server returned an error (or no code).
    #[error("authorization failed: {0}")]
    Authorization(String),
    /// The returned CSRF state did not match the one issued (a forged/replayed callback).
    #[error("OAuth state mismatch: the callback state did not match the request (rejected)")]
    StateMismatch,
    /// The flow did not complete within the allotted time.
    #[error("timed out waiting for the OAuth callback")]
    Timeout,
    /// A base URL or endpoint could not be parsed.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// An explicit callback port fell outside the mission's reserved range.
    #[error("invalid OAuth callback port {0}: expected a port in 3200-3299 or a product pin (1455, 56121)")]
    InvalidCallbackPort(u16),
    /// The token endpoint response could not be parsed.
    #[error("invalid token response: {0}")]
    TokenResponse(String),
    /// The OS CSPRNG was unavailable, so a secure CSRF `state` could not be generated.
    #[error("cannot generate a secure OAuth state: {0}")]
    RandomSource(String),
}

/// The resolved OAuth endpoints and client identity for a remote server.
#[derive(Debug, Clone, Default)]
pub struct OAuthConfig {
    /// The authorization endpoint (`/authorize`).
    pub authorize_endpoint: String,
    /// The token endpoint (`/token`).
    pub token_endpoint: String,
    /// The client id presented to the authorization server.
    pub client_id: String,
    /// The requested scopes (may be empty).
    pub scopes: Vec<String>,
    /// Extra `key=value` query parameters appended to the authorization URL (provider-specific).
    pub authorize_extra_params: Vec<(String, String)>,
    /// Loopback callback path (e.g. `/callback` or `/auth/callback`). `None` → `/callback`.
    pub callback_path: Option<String>,
}

/// Discover the OAuth endpoints for `base_url`, via RFC 8414 metadata when available, else the
/// conventional `<origin>/authorize` and `<origin>/token`.
pub async fn discover(base_url: &str, client_id: &str) -> Result<OAuthConfig, OAuthError> {
    let base =
        url::Url::parse(base_url).map_err(|_| OAuthError::InvalidUrl(base_url.to_owned()))?;
    let origin = origin_of(&base);

    if let Some(config) = discover_metadata(&origin, client_id).await {
        return Ok(config);
    }

    Ok(OAuthConfig {
        authorize_endpoint: format!("{origin}/authorize"),
        token_endpoint: format!("{origin}/token"),
        client_id: client_id.to_owned(),
        scopes: Vec::new(),
        authorize_extra_params: Vec::new(),
        callback_path: None,
    })
}

/// Attempt RFC 8414 authorization-server metadata discovery.
async fn discover_metadata(origin: &str, client_id: &str) -> Option<OAuthConfig> {
    #[derive(Deserialize)]
    struct Metadata {
        authorization_endpoint: String,
        token_endpoint: String,
        #[serde(default)]
        scopes_supported: Vec<String>,
    }

    let url = format!("{origin}/.well-known/oauth-authorization-server");
    let client = reqwest::Client::builder().build().ok()?;
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let metadata: Metadata = response.json().await.ok()?;
    Some(OAuthConfig {
        authorize_endpoint: metadata.authorization_endpoint,
        token_endpoint: metadata.token_endpoint,
        client_id: client_id.to_owned(),
        scopes: metadata.scopes_supported,
        authorize_extra_params: Vec::new(),
        callback_path: None,
    })
}

/// The result of starting the authorization flow: the URL to visit and the live callback listener.
pub struct AuthorizationRequest {
    /// The full authorization URL to open in a browser (carries the CSRF `state`, no secret).
    pub authorize_url: String,
    /// The loopback redirect URI the server will call back.
    pub redirect_uri: String,
    state: String,
    /// PKCE `code_verifier` retained until the token exchange (never logged).
    code_verifier: String,
    listener: TcpListener,
    config: OAuthConfig,
}

/// Begin the authorization flow: bind a loopback callback, generate the CSRF state, and build the
/// authorization URL. `callback_port` pins the loopback port (must be in 3200-3299 or a product pin
/// such as 1455 or 56121); `None` scans the reserved range for a free port.
pub async fn begin(
    config: OAuthConfig,
    callback_port: Option<u16>,
) -> Result<AuthorizationRequest, OAuthError> {
    let listener = bind_callback(callback_port).await?;
    let port = listener
        .local_addr()
        .map_err(|err| OAuthError::Callback(err.to_string()))?
        .port();
    let callback_path = normalize_callback_path(config.callback_path.as_deref());
    let redirect_uri = format!("http://127.0.0.1:{port}{callback_path}");
    let state = random_token()?;
    let pkce = generate_pkce()?;

    let mut authorize = url::Url::parse(&config.authorize_endpoint)
        .map_err(|_| OAuthError::InvalidUrl(config.authorize_endpoint.clone()))?;
    {
        let mut query = authorize.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &config.client_id);
        query.append_pair("redirect_uri", &redirect_uri);
        query.append_pair("state", &state);
        query.append_pair("code_challenge", &pkce.challenge);
        query.append_pair("code_challenge_method", "S256");
        if !config.scopes.is_empty() {
            query.append_pair("scope", &config.scopes.join(" "));
        }
        for (key, value) in &config.authorize_extra_params {
            query.append_pair(key, value);
        }
    }

    Ok(AuthorizationRequest {
        authorize_url: authorize.to_string(),
        redirect_uri,
        state,
        code_verifier: pkce.verifier,
        listener,
        config,
    })
}

impl AuthorizationRequest {
    /// Wait for the loopback callback, validate the CSRF state, and exchange the code for tokens.
    ///
    /// A callback whose `state` does not match the issued one is rejected with
    /// [`OAuthError::StateMismatch`].
    pub async fn complete(self, timeout: Duration) -> Result<StoredToken, OAuthError> {
        let code = match tokio::time::timeout(timeout, accept_callback(&self.listener, &self.state))
            .await
        {
            Ok(result) => result?,
            Err(_) => return Err(OAuthError::Timeout),
        };
        exchange_code(
            &self.config,
            &code,
            &self.redirect_uri,
            &self.code_verifier,
        )
        .await
    }
}

/// Bind a loopback TCP listener for the callback, either on `port` or the first free port in the
/// reserved range.
async fn bind_callback(port: Option<u16>) -> Result<TcpListener, OAuthError> {
    if let Some(port) = port {
        if !is_allowed_callback_port(port) {
            return Err(OAuthError::InvalidCallbackPort(port));
        }
        return TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|err| {
                let detail = err.to_string();
                // Explicit pins (product redirects) must name the port — never opaque OS text alone,
                // and never fall through to the 3200–3299 scan (caller only binds this port).
                let message = if PINNED_CALLBACK_PORTS.contains(&port) {
                    format!(
                        "OAuth callback port {port} is already in use (pinned product redirect; free the port or stop the other process): {detail}"
                    )
                } else {
                    format!("failed to bind OAuth callback port {port}: {detail}")
                };
                OAuthError::Callback(message)
            });
    }
    for candidate in CALLBACK_PORT_RANGE {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)).await {
            return Ok(listener);
        }
    }
    Err(OAuthError::NoCallbackPort)
}

/// Whether `port` may be used as an explicit OAuth loopback callback pin.
fn is_allowed_callback_port(port: u16) -> bool {
    CALLBACK_PORT_RANGE.contains(&port) || PINNED_CALLBACK_PORTS.contains(&port)
}

/// Normalize a provider callback path to a leading-slash absolute path.
fn normalize_callback_path(path: Option<&str>) -> String {
    let raw = path.map(str::trim).filter(|value| !value.is_empty()).unwrap_or(DEFAULT_CALLBACK_PATH);
    if raw.starts_with('/') {
        raw.to_owned()
    } else {
        format!("/{raw}")
    }
}

/// Accept loopback connections until a callback request arrives, validate its state, and return
/// the authorization code. Non-callback requests get a 404 and the loop continues.
async fn accept_callback(
    listener: &TcpListener,
    expected_state: &str,
) -> Result<String, OAuthError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|err| OAuthError::Callback(err.to_string()))?;
        let request_line = read_request_line(&mut stream).await?;
        let Some(target) = request_target(&request_line) else {
            respond(&mut stream, 400, "Bad Request").await;
            continue;
        };
        let (path, query) = split_target(&target);
        if !is_callback_path(&path) {
            respond(&mut stream, 404, "Not Found").await;
            continue;
        }

        let params = parse_query(&query);
        if let Some(error) = params
            .iter()
            .find(|(k, _)| k == "error")
            .map(|(_, v)| v.clone())
        {
            respond(&mut stream, 400, "Authorization denied").await;
            return Err(OAuthError::Authorization(error));
        }
        let state = params
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        if state != expected_state {
            respond(&mut stream, 400, "Invalid state (rejected)").await;
            return Err(OAuthError::StateMismatch);
        }
        let Some(code) = params
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone())
        else {
            respond(&mut stream, 400, "Missing authorization code").await;
            return Err(OAuthError::Authorization("no code in callback".to_owned()));
        };
        respond(
            &mut stream,
            200,
            "Authorization complete. You may close this window.",
        )
        .await;
        return Ok(code);
    }
}

/// True for the default `/callback` path and product pins such as `/auth/callback`.
fn is_callback_path(path: &str) -> bool {
    path == DEFAULT_CALLBACK_PATH || path == "/auth/callback"
}

/// Exchange an authorization code for tokens at the token endpoint.
async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<StoredToken, OAuthError> {
    let client = reqwest::Client::builder().build()?;
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", config.client_id.as_str()),
        ("code_verifier", code_verifier),
    ];
    let response = client
        .post(&config.token_endpoint)
        .form(&form)
        .send()
        .await?;
    parse_token_response(response, config).await
}

/// Mint a fresh credential from a stored refresh token.
pub async fn refresh(stored: &StoredToken) -> Result<StoredToken, OAuthError> {
    let refresh_token = stored
        .refresh_token
        .as_deref()
        .ok_or_else(|| OAuthError::Authorization("no refresh token available".to_owned()))?;
    let config = OAuthConfig {
        authorize_endpoint: String::new(),
        token_endpoint: stored.token_endpoint.clone(),
        client_id: stored.client_id.clone(),
        scopes: Vec::new(),
        authorize_extra_params: Vec::new(),
        callback_path: None,
    };
    let client = reqwest::Client::builder().build()?;
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", config.client_id.as_str()),
    ];
    let response = client
        .post(&config.token_endpoint)
        .form(&form)
        .send()
        .await?;
    let mut refreshed = parse_token_response(response, &config).await?;
    // Some servers omit a new refresh token on refresh; keep the prior one so the next refresh works.
    if refreshed.refresh_token.is_none() {
        refreshed.refresh_token = stored.refresh_token.clone();
    }
    Ok(refreshed)
}

/// Client-side view of an RFC 8628 device authorization response (§3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeAuthorization {
    /// Opaque device code presented at the token endpoint while polling.
    pub device_code: String,
    /// Short user-facing code shown to the end user.
    pub user_code: String,
    /// URL the end user visits to authorize the device.
    pub verification_uri: String,
    /// Lifetime of `device_code` / `user_code` in seconds.
    pub expires_in: u64,
    /// Minimum seconds between token-endpoint polls (RFC 8628 default is 5).
    pub interval: u64,
}

/// Poll the token endpoint for a device-code grant until authorized (RFC 8628 §3.4 / §3.5).
///
/// Between polls the client waits `interval`. On `authorization_pending` it retries after the
/// current interval. On `slow_down` it increases the interval by five seconds and continues.
/// Other token-endpoint errors (`expired_token`, `access_denied`, …) fail closed.
pub async fn poll_device_token(
    config: &OAuthConfig,
    device_code: &str,
    interval: Duration,
) -> Result<StoredToken, OAuthError> {
    poll_device_token_with_sleep(config, device_code, interval, tokio::time::sleep).await
}

/// Like [`poll_device_token`], but `sleep` is injected so tests can assert interval / `slow_down`
/// backoff without wall-clock waits (RFC 8628 §3.5).
pub async fn poll_device_token_with_sleep<F, Fut>(
    config: &OAuthConfig,
    device_code: &str,
    interval: Duration,
    mut sleep: F,
) -> Result<StoredToken, OAuthError>
where
    F: FnMut(Duration) -> Fut,
    Fut: Future<Output = ()>,
{
    let client = reqwest::Client::builder().build()?;
    let mut interval = interval;
    // RFC 8628 §3.4 grant type for the device access token request.
    const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

    loop {
        let form = [
            ("grant_type", GRANT_TYPE),
            ("device_code", device_code),
            ("client_id", config.client_id.as_str()),
        ];
        let response = client
            .post(&config.token_endpoint)
            .form(&form)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;

        if status.is_success() {
            return parse_token_body(&text, config);
        }

        // Error responses carry an OAuth error code in the JSON body (RFC 8628 §3.5).
        #[derive(Deserialize)]
        struct TokenErrorBody {
            error: String,
            #[serde(default)]
            error_description: Option<String>,
        }

        let err = match serde_json::from_str::<TokenErrorBody>(&text) {
            Ok(body) => body,
            Err(_) => {
                return Err(OAuthError::Authorization(format!(
                    "token endpoint returned HTTP {status}"
                )));
            }
        };

        match err.error.as_str() {
            "authorization_pending" => {
                sleep(interval).await;
            }
            "slow_down" => {
                // RFC 8628 §3.5: increase the polling interval by 5 seconds for subsequent polls.
                interval += Duration::from_secs(5);
                sleep(interval).await;
            }
            other => {
                // Terminal errors (expired_token, access_denied, …) fail closed — no further polls.
                let detail = err
                    .error_description
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| other.to_owned());
                return Err(OAuthError::Authorization(detail));
            }
        }
    }
}

/// POST the RFC 8628 §3.1 device authorization request and parse the §3.2 response.
///
/// Returns `device_code`, `user_code`, `verification_uri`, `expires_in`, and `interval` (default 5
/// when the server omits it). Token values are never logged.
pub async fn request_device_authorization(
    config: &OAuthConfig,
    device_authorization_endpoint: &str,
) -> Result<DeviceCodeAuthorization, OAuthError> {
    #[derive(Deserialize)]
    struct DeviceAuthBody {
        device_code: String,
        user_code: String,
        verification_uri: String,
        expires_in: u64,
        #[serde(default = "default_device_interval")]
        interval: u64,
    }

    let client = reqwest::Client::builder().build()?;
    let scope = config.scopes.join(" ");
    let mut form: Vec<(&str, &str)> = vec![("client_id", config.client_id.as_str())];
    if !scope.is_empty() {
        form.push(("scope", scope.as_str()));
    }
    let response = client
        .post(device_authorization_endpoint)
        .form(&form)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Authorization(format!(
            "device authorization endpoint returned HTTP {status}"
        )));
    }
    let body: DeviceAuthBody =
        serde_json::from_str(&text).map_err(|err| OAuthError::TokenResponse(err.to_string()))?;
    Ok(DeviceCodeAuthorization {
        device_code: body.device_code,
        user_code: body.user_code,
        verification_uri: body.verification_uri,
        expires_in: body.expires_in,
        interval: body.interval,
    })
}

fn default_device_interval() -> u64 {
    5
}

/// Full RFC 8628 device-code flow: device authorization request then token poll.
///
/// Callers that need to surface `user_code` / `verification_uri` to the end user should use
/// [`request_device_authorization`] + [`poll_device_token`] instead so they can display instructions
/// before polling begins.
pub async fn device_code_flow(
    config: &OAuthConfig,
    device_authorization_endpoint: &str,
) -> Result<StoredToken, OAuthError> {
    let auth = request_device_authorization(config, device_authorization_endpoint).await?;
    let interval = Duration::from_secs(auth.interval);
    poll_device_token(config, &auth.device_code, interval).await
}

/// Parse a token endpoint response into a [`StoredToken`].
async fn parse_token_response(
    response: reqwest::Response,
    config: &OAuthConfig,
) -> Result<StoredToken, OAuthError> {
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(OAuthError::Authorization(format!(
            "token endpoint returned HTTP {status}"
        )));
    }
    parse_token_body(&text, config)
}

/// Parse a successful token-endpoint JSON body into a [`StoredToken`].
fn parse_token_body(text: &str, config: &OAuthConfig) -> Result<StoredToken, OAuthError> {
    #[derive(Deserialize)]
    struct TokenBody {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        token_type: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let body: TokenBody =
        serde_json::from_str(text).map_err(|err| OAuthError::TokenResponse(err.to_string()))?;
    let expires_at = body.expires_in.map(|secs| now_secs() + secs);
    Ok(StoredToken {
        access_token: body.access_token,
        refresh_token: body.refresh_token,
        token_type: body.token_type.unwrap_or_else(|| "Bearer".to_owned()),
        expires_at,
        token_endpoint: config.token_endpoint.clone(),
        client_id: config.client_id.clone(),
    })
}

/// The scheme://host:port origin of a URL.
fn origin_of(url: &url::Url) -> String {
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("127.0.0.1");
    match url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// Read just the HTTP request line (the first CRLF-terminated line) from the stream.
async fn read_request_line(stream: &mut TcpStream) -> Result<String, OAuthError> {
    let mut buffer = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        let read = stream
            .read(&mut byte)
            .await
            .map_err(|err| OAuthError::Callback(err.to_string()))?;
        if read == 0 {
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            buffer.push(byte[0]);
        }
        if buffer.len() > 8192 {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

/// Extract the request target (path + query) from a request line like `GET /callback?... HTTP/1.1`.
fn request_target(request_line: &str) -> Option<String> {
    request_line.split_whitespace().nth(1).map(str::to_owned)
}

/// Split a request target into its path and raw query.
fn split_target(target: &str) -> (String, String) {
    match target.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (target.to_owned(), String::new()),
    }
}

/// Parse a `key=value&key=value` query string into decoded pairs.
fn parse_query(query: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Write a minimal HTTP/1.1 response and close the connection.
async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body = format!("<html><body>{message}</body></html>");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// A random hex token (128 bits) read from the OS CSPRNG, used for the CSRF state.
///
/// This FAILS CLOSED: if the OS CSPRNG is unavailable there is no time-based fallback (a
/// `SystemTime`-nanos value would be predictable, making the single-use state guessable), so the
/// caller refuses the authorize request instead of issuing a weak challenge.
fn random_token() -> Result<String, OAuthError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| OAuthError::RandomSource(err.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// RFC 7636 PKCE pair: high-entropy `code_verifier` and its S256 `code_challenge`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pkce {
    /// `code_verifier` — 43..=128 unreserved characters (RFC 7636 §4.1).
    verifier: String,
    /// `code_challenge` — BASE64URL-ENCODE(SHA256(verifier)) without padding (S256).
    challenge: String,
}

/// Generate a PKCE `code_verifier` and its S256 `code_challenge` (RFC 7636).
///
/// Uses 32 CSPRNG bytes encoded as base64url-nopad (43 unreserved chars). Fails closed when the
/// OS CSPRNG is unavailable — same policy as [`random_token`].
fn generate_pkce() -> Result<Pkce, OAuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| OAuthError::RandomSource(err.to_string()))?;
    let verifier = base64url_nopad(&bytes);
    let challenge = pkce_challenge(&verifier);
    Ok(Pkce {
        verifier,
        challenge,
    })
}

/// Compute the S256 `code_challenge` for an existing `code_verifier` (RFC 7636 §4.2).
///
/// `BASE64URL-NOPAD(SHA256(ASCII(code_verifier)))`.
fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64url_nopad(digest.as_slice())
}

/// BASE64URL encode without `=` padding (RFC 4648 §5 / RFC 7636).
fn base64url_nopad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = (u32::from(bytes[i]) << 16)
            | (u32::from(bytes[i + 1]) << 8)
            | u32::from(bytes[i + 2]);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = u32::from(bytes[i]) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
    } else if rem == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_strips_path_and_keeps_port() {
        let url = url::Url::parse("http://127.0.0.1:3201/mcp").unwrap();
        assert_eq!(origin_of(&url), "http://127.0.0.1:3201");
    }

    #[test]
    fn random_tokens_are_unique_and_hex() {
        let a = random_token().expect("CSPRNG available");
        let b = random_token().expect("CSPRNG available");
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn query_parsing_decodes_pairs() {
        let pairs = parse_query("code=abc%20def&state=xyz");
        assert_eq!(pairs[0], ("code".to_owned(), "abc def".to_owned()));
        assert_eq!(pairs[1], ("state".to_owned(), "xyz".to_owned()));
    }

    #[test]
    fn request_target_is_extracted() {
        assert_eq!(
            request_target("GET /callback?code=1&state=2 HTTP/1.1").as_deref(),
            Some("/callback?code=1&state=2")
        );
    }

    #[tokio::test]
    async fn explicit_callback_port_must_stay_in_reserved_range() {
        let outcome = bind_callback(Some(3199)).await;
        assert!(
            matches!(outcome, Err(OAuthError::InvalidCallbackPort(3199))),
            "out-of-range callback ports must be rejected, got: {outcome:?}"
        );
    }

    /// Product pin occupied: fail closed with an actionable message naming the port (no scan).
    #[tokio::test]
    async fn pinned_callback_port_occupied_fails_actionably_without_scan_fallback() {
        let holder = TcpListener::bind(("127.0.0.1", 56121))
            .await
            .expect("hold xAI pin 56121");
        let outcome = bind_callback(Some(56121)).await;
        drop(holder);
        let err = match outcome {
            Err(err) => err,
            Ok(_) => panic!("bind_callback must not succeed when pin 56121 is occupied"),
        };
        assert!(
            !matches!(err, OAuthError::NoCallbackPort),
            "occupied pin must not report scan-range exhaustion: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("56121"),
            "error must name pinned port 56121: {msg}"
        );
        let lower = msg.to_ascii_lowercase();
        assert!(
            lower.contains("already in use")
                || lower.contains("in use")
                || lower.contains("occupied"),
            "error must be actionable about occupancy: {msg}"
        );
        assert!(
            lower.contains("pinned") || lower.contains("product"),
            "pinned-port collision should mention the pin contract: {msg}"
        );
    }

    /// RFC 7636 S256: verifier length 43..=128 and challenge == base64url-nopad(SHA256(verifier)).
    ///
    /// Pins S256 semantics against the independent test oracle (RFC 7636 Appendix B).
    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let pkce = generate_pkce().expect("generate_pkce must succeed when CSPRNG is available");
        assert!(
            (43..=128).contains(&pkce.verifier.len()),
            "RFC 7636 code_verifier length must be 43..=128, got {}",
            pkce.verifier.len()
        );

        let challenge = pkce_challenge(&pkce.verifier);
        assert_eq!(
            challenge, pkce.challenge,
            "generate_pkce challenge must match pkce_challenge(verifier)"
        );
        assert!(
            !challenge.contains('='),
            "code_challenge must be base64url without padding, got {challenge:?}"
        );

        let expected = independent_s256_challenge(&pkce.verifier);
        assert_eq!(
            challenge, expected,
            "code_challenge must equal BASE64URL-NOPAD(SHA256(ASCII(code_verifier)))"
        );
    }

    /// Independent RFC 7636 Appendix B oracle — test-only, not production PKCE.
    fn independent_s256_challenge(verifier: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(verifier.as_bytes());
        base64url_nopad(digest.as_slice())
    }

    /// BASE64URL encode without `=` padding (RFC 4648 §5 / RFC 7636).
    fn base64url_nopad(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let n = (u32::from(bytes[i]) << 16)
                | (u32::from(bytes[i + 1]) << 8)
                | u32::from(bytes[i + 2]);
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push(TABLE[(n & 63) as usize] as char);
            i += 3;
        }
        let rem = bytes.len() - i;
        if rem == 1 {
            let n = u32::from(bytes[i]) << 16;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
        } else if rem == 2 {
            let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        }
        out
    }

    #[test]
    fn independent_s256_matches_rfc7636_appendix_b() {
        // RFC 7636 Appendix B test vector — pins the test oracle, not production PKCE.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(independent_s256_challenge(verifier), expected);
        assert!(!expected.contains('='));
        assert!((43..=128).contains(&verifier.len()));
    }

    /// RFC 8628 §3.4 / §3.5: poll until authorized; honor `interval`; `slow_down` adds 5s backoff.
    ///
    /// Fixture sequence: `authorization_pending` ×2 → `slow_down` → token success.
    /// Sleep injection records waits so the test asserts timing without wall-clock delay.
    #[tokio::test]
    async fn device_code_polls_until_authorized() {
        use std::sync::{Arc, Mutex};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind device-code fixture listener");
        let addr = listener
            .local_addr()
            .expect("fixture listener local_addr");
        let token_endpoint = format!("http://{addr}/token");

        // Response bodies only — never logged. Sequence is the RFC 8628 contract under test.
        let sequence: Arc<Vec<&'static str>> = Arc::new(vec![
            r#"{"error":"authorization_pending"}"#,
            r#"{"error":"authorization_pending"}"#,
            r#"{"error":"slow_down"}"#,
            r#"{"access_token":"device-access-token","token_type":"Bearer","expires_in":3600}"#,
        ]);
        let hit = Arc::new(AtomicUsize::new(0));
        let sequence_srv = Arc::clone(&sequence);
        let hit_srv = Arc::clone(&hit);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                // Drain request headers (stop at blank line).
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                let mut consecutive_newlines = 0u8;
                while consecutive_newlines < 2 {
                    if stream.read(&mut byte).await.ok() != Some(1) {
                        break;
                    }
                    if byte[0] == b'\n' {
                        consecutive_newlines += 1;
                    } else if byte[0] != b'\r' {
                        consecutive_newlines = 0;
                    }
                    buf.push(byte[0]);
                    if buf.len() > 16_384 {
                        break;
                    }
                }
                let idx = hit_srv.fetch_add(1, Ordering::SeqCst);
                let body = sequence_srv
                    .get(idx)
                    .copied()
                    .unwrap_or(r#"{"error":"expired_token"}"#);
                let status = if body.contains("access_token") {
                    "200 OK"
                } else {
                    "400 Bad Request"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });

        let config = OAuthConfig {
            authorize_endpoint: String::new(),
            token_endpoint,
            client_id: "device-test-client".to_owned(),
            scopes: Vec::new(),
            authorize_extra_params: Vec::new(),
            callback_path: None,
        };

        let sleeps: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
        let sleeps_rec = Arc::clone(&sleeps);
        let sleep = move |d: Duration| {
            let sleeps_rec = Arc::clone(&sleeps_rec);
            async move {
                sleeps_rec
                    .lock()
                    .expect("sleep recorder lock")
                    .push(d);
            }
        };

        let initial_interval = Duration::from_secs(5);
        let token = poll_device_token_with_sleep(
            &config,
            "device-code-under-test",
            initial_interval,
            sleep,
        )
        .await
        .expect("device code poll must succeed after pending ×2 + slow_down");

        assert_eq!(
            token.access_token, "device-access-token",
            "final successful poll must yield the fixture access_token"
        );
        assert_eq!(token.client_id, "device-test-client");
        assert_eq!(token.token_type, "Bearer");

        let recorded = sleeps.lock().expect("sleep recorder lock").clone();
        // After each non-success poll the client waits the *current* interval, then polls again.
        // pending → wait 5s; pending → wait 5s; slow_down → interval becomes 10s, wait 10s; token.
        assert_eq!(
            recorded,
            vec![
                Duration::from_secs(5),
                Duration::from_secs(5),
                Duration::from_secs(10),
            ],
            "RFC 8628: honor interval between polls; slow_down increases interval by 5 seconds"
        );
        assert_eq!(
            hit.load(Ordering::SeqCst),
            4,
            "fixture must be polled four times (pending ×2, slow_down, token)"
        );
    }
}
